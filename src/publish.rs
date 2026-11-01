//! Saving a Story as a draft on a WordPress site (`docs/TRAVEL.md`): each
//! photo rendered in the chosen size and uploaded, or a confirmed upload of
//! the same bytes reused, then the blocks written with the site's media and
//! the draft made, or the Story's earlier draft changed in place.
//!
//! It runs on a thread and reports each confirmed upload as it happens, so
//! the library records it at once and a retry after a failure, a cancel, or
//! a restart reuses it instead of sending it again. Nothing on the site is
//! deleted, and a post that is no longer a draft is never changed.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::gutenberg::{self, RouteMap, Unsupported, Uploaded};
use crate::story_html::{MapTiles, Progress, Source};
use crate::trip::{Block, Story, StoryBlock};
use crate::upload::{self, UploadSize};
use crate::wordpress::{edit_link, DraftPost, Api, Client, Credentials, Failure, Http, Media, Policy};

/// Everything the job needs, read on the UI thread before it starts.
#[derive(Clone)]
pub struct Job {
    pub api: Api,
    pub credentials: Credentials,
    pub policy: Policy,
    pub story: Story,
    pub blocks: Vec<StoryBlock>,
    /// Each photo of the Story by media reference, `None` when it is unavailable.
    pub sources: HashMap<i64, Source>,
    pub size: UploadSize,
    /// Whether photos keep how they were taken (`upload::render`).
    pub exif: bool,
    /// Confirmed uploads on this site, by the hash of their bytes: (media id, address).
    pub uploads: HashMap<String, (i64, String)>,
    /// The Story's earlier draft on this site.
    pub post_id: Option<i64>,
    /// What Picasso writes in that post as the site had it after the last
    /// save (`wordpress::fingerprint`); empty when not known.
    pub fingerprint: String,
    /// The person chose to replace edits made to the post on the site since.
    pub overwrite: bool,
    /// When a draft was last about to be made for the Story on this site
    /// without the site confirming it (Unix seconds): one made then may be
    /// there, and is looked for before another is made.
    pub pending_since: Option<i64>,
    /// The person confirmed making the post public once saved.
    pub publish: bool,
    /// With `publish`, when the post goes public instead of at once, in the
    /// site's own time.
    pub schedule: Option<chrono::NaiveDateTime>,
    /// Publish as an index post and a post per Day, with the posts already
    /// made for it by key ([`INDEX`] or [`day_key`]); `None` makes one post.
    pub days: Option<HashMap<String, DayPost>>,
    /// The person chose to make a new post although the site has one with
    /// the same title, which the library has no record of.
    pub allow_new: bool,
    /// Tiles for the route maps; without them a route is drawn on its own.
    pub map: Option<MapTiles>,
    /// Each Route block's elevation by its uid, drawn in its map's corner.
    pub profiles: HashMap<String, crate::elevation::Profile>,
    /// The person chose to change the earlier post even though it is no
    /// longer a draft; its status on the site stays as it is.
    pub update_published: bool,
    /// The site's categories and tags chosen for the post; none chosen of
    /// either leaves the post's own on the site as they are.
    pub terms: crate::db::PostTerms,
    /// What photos rendered to earlier, shared and added to by each job.
    pub renders: Renders,
}

/// What photos rendered to in this session, by [`render_key`]: the hash and
/// length of their bytes. A photo whose bytes the site already has isn't
/// rendered again to learn them. Kept in memory only, so a newer Picasso,
/// which may render differently, starts afresh.
#[derive(Clone, Default)]
pub struct Renders(Arc<Mutex<HashMap<String, (String, u64)>>>);

/// Photos [`Renders`] keeps before it starts again.
const RENDERS_KEPT: usize = 20_000;

impl Renders {
    fn get(&self, key: &str) -> Option<(String, u64)> {
        self.0.lock().ok()?.get(key).cloned()
    }

    fn keep(&self, key: String, hash: &str, bytes: u64) {
        let Ok(mut kept) = self.0.lock() else { return };
        if kept.len() >= RENDERS_KEPT {
            kept.clear();
        }
        kept.insert(key, (hash.to_string(), bytes));
    }
}

/// What a photo's upload is made from: its file as it is now, its edit and
/// turn, and how it is sent. `None` when the file can't be looked at.
fn render_key(path: &Path, settings: &crate::edit::Settings, rotation: i32, size: UploadSize, exif: bool, crop: Option<crate::trip::CoverCrop>) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let what = format!("{}\n{}\n{modified}\n{settings:?}\n{rotation}\n{size:?}\n{exif}\n{crop:?}", path.display(), meta.len());
    Some(upload::hash(what.as_bytes()))
}

/// What the job learns that the library should keep at once.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The site confirmed an upload of these bytes.
    Uploaded { hash: String, size: UploadSize, media_id: i64, url: String, details: String },
    /// An upload recorded before is no longer on the site.
    Gone { hash: String },
    /// Every photo is rendered, or known: this many different ones, of these bytes in
    /// all, are what may be sent, before anything is.
    Prepared { photos: usize, bytes: u64 },
    /// A draft is about to be made for the Story; until it is confirmed, one
    /// made after `since` (Unix seconds) is looked for before another is made.
    Creating { story_id: i64, since: i64 },
    /// A post of a Story published by Day, as the site has it now: made
    /// (still empty), saved, or found edited there or no longer a draft.
    Part { story_id: i64, revision: i64, key: String, post: DayPost },
}

/// One post of a Story published by Day, as the library keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DayPost {
    pub post_id: i64,
    pub status: String,
    /// See `wordpress::fingerprint`; empty until saved with content.
    pub fingerprint: String,
    pub changed_on_site: bool,
}

/// The key of the index post of a Story published by Day.
pub const INDEX: &str = "index";

/// The key of the post for the Day that starts at heading block `uid`: the
/// block's own id, so renaming or moving the Day keeps its post.
pub fn day_key(uid: &str) -> String {
    format!("day:{uid}")
}

/// A Story cut into posts by Day: each top-level heading starts a Day, and
/// what comes before the first goes in the index, with the introduction and
/// conclusion. The headings themselves become the Day posts' titles.
#[derive(Clone, Debug, PartialEq)]
pub struct DayPart {
    pub key: String,
    pub heading: String,
    pub blocks: Vec<StoryBlock>,
}

pub fn split_days(blocks: &[StoryBlock]) -> Vec<DayPart> {
    let mut parts = vec![DayPart { key: INDEX.into(), heading: String::new(), blocks: Vec::new() }];
    for block in blocks {
        match &block.block {
            Block::Heading(text) => parts.push(DayPart { key: day_key(&block.uid), heading: text.trim().to_string(), blocks: Vec::new() }),
            _ => parts.last_mut().expect("the index is first").blocks.push(block.clone()),
        }
    }
    parts
}

/// A draft saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub post_id: i64,
    /// Where to open it in the site's editor.
    pub edit_link: String,
    pub uploaded: usize,
    pub reused: usize,
    /// The post's status on the site: "draft", or the status it kept.
    pub status: String,
    /// What Picasso wrote, as the site keeps it, to compare before the next save.
    pub fingerprint: String,
    /// This save made the post public.
    pub published: bool,
    /// It was asked to go public at a set time.
    pub scheduled: bool,
    /// Published by Day: how many Day posts besides the index, which is
    /// the post named here; 0 for one post.
    pub days: usize,
    /// The site's media the post uses, photos and route maps, by id.
    pub media: Vec<i64>,
}

/// Why the draft wasn't saved. `uploaded` counts the photos that did reach
/// the site before it stopped; they stay there.
#[derive(Clone, Debug, PartialEq)]
pub enum Stopped {
    Cancelled { uploaded: usize },
    /// The site refused or couldn't be reached.
    Site { failure: Failure, uploaded: usize },
    /// A photo is unavailable or can't be read; nothing is left out.
    Photo { media_id: i64, uploaded: usize },
    /// A block from a newer Picasso has no form WordPress takes.
    Unsupported(Unsupported),
    /// The earlier post isn't a draft any more, with the status the site
    /// gave, and changing it wasn't chosen; a trashed post is never changed.
    NotDraft(String),
    /// The earlier post is gone from the site.
    PostGone,
    /// The earlier post was edited on the site since it was saved, and
    /// replacing those edits wasn't chosen; nothing was sent.
    ChangedOnSite,
    /// The site may have made a draft whose answer was lost, and more than
    /// one draft could be it, so none is chosen and no other is made.
    Uncertain { drafts: usize },
    /// The library has no post for the Story on the site, but the site has
    /// one with its title, as after restoring an older library: which to
    /// do wasn't chosen, so nothing was sent.
    TitleOnSite { post_id: i64 },
    /// The time chosen to publish at is past, or too close, on the site's
    /// clock; nothing was sent.
    TooSoon,
}

/// How far ahead of the site's clock a scheduled time must be, in minutes.
pub const SCHEDULE_AHEAD: i64 = 5;

/// A finished job, for the library: which Story and site, in which size,
/// from which revision, and how it ended.
#[derive(Debug)]
pub struct Finished {
    pub story_id: i64,
    pub site_id: i64,
    pub site: String,
    pub size: UploadSize,
    pub revision: i64,
    /// When the job started (Unix seconds).
    pub started_at: i64,
    /// Its row in the publishing log, kept since it started.
    pub log: Option<i64>,
    /// The job's own progress, which tells it from any started since.
    pub job: std::sync::Arc<crate::story_html::Progress>,
    pub outcome: Result<Saved, Stopped>,
}

/// Where a Story's post on a site stands, from what the library knows,
/// apart from the post's status there and from any job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// Not on the site yet.
    New,
    /// The site has the Story as it is here.
    Clean,
    /// The Story was changed here since it was last saved there.
    ChangedHere,
    /// The post was found edited on the site since; it waits for a choice.
    Conflict,
    /// A post was being made and the site never confirmed it.
    Unconfirmed,
}

pub fn sync_state(publication: Option<&crate::db::Publication>, revision: i64, pending: bool) -> SyncState {
    match publication {
        None if pending => SyncState::Unconfirmed,
        None => SyncState::New,
        Some(publication) if publication.changed_on_site => SyncState::Conflict,
        Some(publication) if publication.revision != revision => SyncState::ChangedHere,
        Some(_) => SyncState::Clean,
    }
}

/// What the publishing log keeps for a job until it ends; one that never
/// ended was cut short, as by a crash or a power cut.
pub const RUNNING: &str = "running";

/// How a job ended, in a word or two for the library's publishing log;
/// a site's failure is named by its kind, which holds nothing secret.
pub fn outcome_code(outcome: &Result<Saved, Stopped>) -> String {
    match outcome {
        Ok(saved) if saved.published && saved.status == "future" => "scheduled".into(),
        Ok(saved) if saved.days > 0 && saved.published => format!("published: {} posts", saved.days + 1),
        Ok(saved) if saved.days > 0 => format!("saved: {} posts", saved.days + 1),
        Ok(saved) if saved.published => "published".into(),
        Ok(saved) if saved.status == "draft" => "saved".into(),
        Ok(_) => "updated".into(),
        Err(Stopped::Cancelled { .. }) => "cancelled".into(),
        Err(Stopped::Site { failure, .. }) => format!("site: {failure:?}"),
        Err(Stopped::Photo { .. }) => "photo unavailable".into(),
        Err(Stopped::Unsupported(_)) => "unsupported block".into(),
        Err(Stopped::NotDraft(status)) => format!("not a draft: {status}"),
        Err(Stopped::PostGone) => "post gone".into(),
        Err(Stopped::ChangedOnSite) => "changed on site".into(),
        Err(Stopped::Uncertain { drafts }) => format!("uncertain: {drafts} drafts"),
        Err(Stopped::TitleOnSite { post_id }) => format!("title on site: post {post_id}"),
        Err(Stopped::TooSoon) => "too soon".into(),
    }
}

/// Photos that reached the site, and confirmed ones reused, however the job ended.
pub fn counts(outcome: &Result<Saved, Stopped>) -> (usize, usize) {
    match outcome {
        Ok(saved) => (saved.uploaded, saved.reused),
        Err(Stopped::Cancelled { uploaded } | Stopped::Site { uploaded, .. } | Stopped::Photo { uploaded, .. }) => (*uploaded, 0),
        Err(_) => (0, 0),
    }
}

pub fn run(http: &dyn Http, job: &Job, progress: &Progress, report: &mut dyn FnMut(Event)) -> Result<Saved, Stopped> {
    let client = Client::new(http, job.policy).cancelled_by(&progress.cancel);
    let mut uploaded = 0;
    let mut reused = 0;
    let site = |failure: Failure, uploaded: usize| Stopped::Site { failure, uploaded };
    // A post published or edited on the site since is changed only when
    // that was chosen, and is checked before anything is sent. The site
    // has no conditional write, so an edit in the moments between this
    // check and the save can still be replaced.
    let mut status = "draft".to_string();
    if let Some(id) = job.post_id {
        let Some(found) = client.post_state(&job.api, &job.credentials, id).map_err(|failure| site(failure, 0))? else {
            return Err(Stopped::PostGone);
        };
        if found.status == "trash" || (found.status != "draft" && !job.update_published) {
            return Err(Stopped::NotDraft(found.status));
        }
        // A digest kept by an earlier Picasso can't be compared, and is replaced on this save.
        let comparable = job.fingerprint.starts_with(crate::wordpress::FINGERPRINT);
        if comparable && found.fingerprint != job.fingerprint && !job.overwrite {
            return Err(Stopped::ChangedOnSite);
        }
        status = found.status;
    }
    // A time already past would make the post public at once, so it is
    // checked against the site's own clock before anything is sent.
    if let Some(at) = job.schedule.filter(|_| job.publish) {
        let now = client.site_now(&job.api).map_err(|failure| site(failure, 0))?;
        if at < now + chrono::Duration::minutes(SCHEDULE_AHEAD) {
            return Err(Stopped::TooSoon);
        }
    }
    // A post the library has lost track of is found before another is made.
    if job.post_id.is_none() && job.pending_since.is_none() && !job.allow_new {
        let title = job.story.title.trim();
        if let Some(&post_id) = client.find_drafts(&job.api, &job.credentials, title, 0).map_err(|failure| site(failure, 0))?.first() {
            return Err(Stopped::TitleOnSite { post_id });
        }
    }
    let mut order: Vec<i64> = job.sources.keys().copied().collect();
    // A cropped cover is a picture of its own, beside the photo uncropped.
    order.extend(job.story.cover_key().filter(|key| *key < 0));
    order.sort_unstable();
    // Every photo is rendered before any is sent, to a folder of its own,
    // so the bytes to send are known first and none is held in memory. One
    // that rendered before to bytes the site has is only rendered if sent.
    progress.total.store(order.len() * 2 + 1, Ordering::Relaxed);
    let staging = Staging::new(job.story.id);
    let made = crate::image_io::render_all(&order, |&media_id| {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(Stopped::Cancelled { uploaded: 0 });
        }
        let (hash, bytes) = match rendered_before(job, media_id) {
            Some(known) => known,
            None => render_photo(job, media_id, &staging, 0)?,
        };
        progress.done.fetch_add(1, Ordering::Relaxed);
        Ok((media_id, hash, bytes))
    })?;
    let mut rendered: Vec<(i64, String)> = Vec::new();
    let mut sizes: HashMap<String, u64> = HashMap::new();
    for (media_id, hash, bytes) in made {
        sizes.insert(hash.clone(), bytes);
        rendered.push((media_id, hash));
    }
    report(Event::Prepared { photos: sizes.len(), bytes: sizes.values().sum() });
    // Each photo's first description in the Story goes with its upload.
    let mut alts: HashMap<i64, &str> = HashMap::new();
    for item in job.blocks.iter().flat_map(|block| block.block.media()) {
        if !item.alt.trim().is_empty() {
            alts.entry(item.media_id).or_insert(item.alt.trim());
        }
    }
    // Uploads already on the site are checked together, a hundred to a
    // request; one the answer leaves out, or a failed read, is checked alone.
    let mut ids: Vec<i64> = rendered.iter().filter_map(|(_, hash)| job.uploads.get(hash).map(|(id, _)| *id)).collect();
    ids.sort_unstable();
    ids.dedup();
    let known = if ids.is_empty() { HashMap::new() } else { client.media_many(&job.api, &job.credentials, &ids).unwrap_or_default() };
    let mut media: HashMap<i64, Uploaded> = HashMap::new();
    let mut by_hash: HashMap<String, Uploaded> = HashMap::new();
    for (media_id, hash) in rendered {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(Stopped::Cancelled { uploaded });
        }
        // The same photo used twice in the Story is sent once.
        if let Some(same) = by_hash.get(&hash) {
            media.insert(media_id, same.clone());
            progress.done.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let so_far = uploaded;
        let mut bytes = || {
            if let Ok(bytes) = std::fs::read(staging.path(&hash)) {
                return Ok(bytes);
            }
            // Not rendered, as the site had these bytes; rendered now that it may not.
            let failed = Stopped::Photo { media_id: media_id.abs(), uploaded: so_far };
            match render_photo(job, media_id, &staging, so_far)? {
                (now, _) if now == hash => std::fs::read(staging.path(&hash)).map_err(|_| failed),
                _ => Err(failed),
            }
        };
        let alt = alts.get(&media_id).copied().unwrap_or_default();
        let file = job.sources.get(&media_id).cloned().flatten().and_then(|(path, ..)| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()));
        let stem = upload_name(&job.story.title, file.as_deref().unwrap_or_default(), &hash);
        let item = place(&client, job, &mut bytes, &hash, &stem, alt, &known, &mut uploaded, &mut reused, report)?;
        by_hash.insert(hash, item.clone());
        media.insert(media_id, item);
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    // Each route on its map, as a picture like the photos; a route whose
    // tiles can't be had is drawn on its own instead.
    let mut maps: HashMap<String, RouteMap> = HashMap::new();
    if let Some(tiles) = &job.map {
        for block in &job.blocks {
            let Block::Route { route, .. } = &block.block else { continue };
            if progress.cancel.load(Ordering::Relaxed) {
                return Err(Stopped::Cancelled { uploaded });
            }
            let Some(rgba) = crate::story_html::route_map(route, tiles, job.profiles.get(&block.uid).map(Vec::as_slice)) else { continue };
            let Ok(bytes) = crate::image_io::encode_jpeg(&rgba, job.size.quality()) else { continue };
            let hash = upload::hash(&bytes);
            let item = match by_hash.get(&hash) {
                Some(same) => same.clone(),
                None => place(&client, job, &mut || Ok(bytes.clone()), &hash, &upload_name(&job.story.title, "route", &hash), "", &known, &mut uploaded, &mut reused, report)?,
            };
            by_hash.insert(hash, item.clone());
            maps.insert(block.uid.clone(), RouteMap { uploaded: item, credit: tiles.source.attribution() });
        }
    }
    if progress.cancel.load(Ordering::Relaxed) {
        return Err(Stopped::Cancelled { uploaded });
    }
    let post = gutenberg::post(&job.story, &job.blocks, &|id| media.get(&id).cloned(), &|uid| maps.get(uid).cloned()).map_err(Stopped::Unsupported)?;
    let draft = DraftPost { publish: job.publish, schedule: job.schedule.filter(|_| job.publish), ..draft_post(post, &job.terms) };
    // A draft made before whose answer was lost is changed instead of made again.
    let mut post_id = job.post_id;
    if let (None, Some(since)) = (post_id, job.pending_since) {
        post_id = lost_draft(&client, job, &draft.title, since, uploaded)?;
    }
    if post_id.is_none() {
        let since = crate::types::unix_now() - CLOCK_SLACK;
        report(Event::Creating { story_id: job.story.id, since });
    }
    // A Story without a title makes a post WordPress shows as "(no title)".
    let saved = match client.save_draft(&job.api, &job.credentials, post_id, &draft) {
        Ok(saved) => saved,
        // Made but not answered: found by its title, never sent again.
        Err(failure) if failure.outcome_unknown() && post_id.is_none() => {
            let since = crate::types::unix_now() - CLOCK_SLACK;
            let Some(id) = lost_draft(&client, job, &draft.title, since, uploaded)? else { return Err(site(failure, uploaded)) };
            let found = client.post_state(&job.api, &job.credentials, id).map_err(|failure| site(failure, uploaded))?;
            let (fingerprint, status) = found.map(|found| (found.fingerprint, found.status)).unwrap_or_default();
            crate::wordpress::PostSaved { id, fingerprint, status }
        }
        Err(failure) => return Err(site(failure, uploaded)),
    };
    progress.done.fetch_add(1, Ordering::Relaxed);
    if job.publish {
        status = if saved.status.is_empty() { "publish".to_string() } else { saved.status.clone() };
    }
    Ok(Saved { post_id: saved.id, edit_link: edit_link(&job.api, saved.id), uploaded, reused, status, fingerprint: saved.fingerprint, published: job.publish, scheduled: job.publish && job.schedule.is_some(), days: 0, media: used(&by_hash) })
}

/// Publishes the Story as an index post and a post per Day. Every post
/// already made is checked first, so one edited on the site or no longer a
/// draft stops the job before anything is sent, unless that was chosen.
/// Each part without a post then gets an empty draft, so every post has an
/// id to link to; the index lists the Days, and each Day links to the
/// index and its neighbours. Each part is then saved as one post is, and
/// reported as it is, so a job that stops resumes where it was.
pub fn run_days(http: &dyn Http, job: &Job, progress: &Progress, report: &mut dyn FnMut(Event)) -> Result<Saved, Stopped> {
    let client = Client::new(http, job.policy).cancelled_by(&progress.cancel);
    let site = |failure: Failure| Stopped::Site { failure, uploaded: 0 };
    let parts = split_days(&job.blocks);
    let mut posts = job.days.clone().unwrap_or_default();
    for part in &parts {
        let Some(known) = posts.get(&part.key).cloned() else { continue };
        let Some(found) = client.post_state(&job.api, &job.credentials, known.post_id).map_err(site)? else {
            // Gone: made again below.
            posts.remove(&part.key);
            continue;
        };
        let changed = known.fingerprint.starts_with(crate::wordpress::FINGERPRINT) && found.fingerprint != known.fingerprint;
        let post = DayPost { status: found.status.clone(), changed_on_site: changed, ..known };
        report(Event::Part { story_id: job.story.id, revision: job.story.revision, key: part.key.clone(), post: post.clone() });
        if found.status == "trash" {
            return Err(Stopped::NotDraft(found.status));
        }
        if changed && !job.overwrite {
            return Err(Stopped::ChangedOnSite);
        }
        if found.status != "draft" && !job.update_published {
            return Err(Stopped::NotDraft(found.status));
        }
        posts.insert(part.key.clone(), post);
    }
    let title = |part: &DayPart| match part.heading.as_str() {
        "" => job.story.title.trim().to_string(),
        heading => format!("{} – {heading}", job.story.title.trim()),
    };
    for part in &parts {
        if posts.contains_key(&part.key) {
            continue;
        }
        let since = crate::types::unix_now() - CLOCK_SLACK;
        let empty = DraftPost { title: title(part), ..DraftPost::default() };
        let made = match client.save_draft(&job.api, &job.credentials, None, &empty) {
            Ok(made) => made.id,
            // Made but not answered: found by its title, never made twice.
            Err(failure) if failure.outcome_unknown() => match client.find_drafts(&job.api, &job.credentials, &empty.title, since).map_err(site)?.as_slice() {
                [id] => *id,
                [] => return Err(site(failure)),
                found => return Err(Stopped::Uncertain { drafts: found.len() }),
            },
            Err(failure) => return Err(site(failure)),
        };
        let post = DayPost { post_id: made, status: "draft".into(), ..DayPost::default() };
        report(Event::Part { story_id: job.story.id, revision: job.story.revision, key: part.key.clone(), post: post.clone() });
        posts.insert(part.key.clone(), post);
    }
    let link = |key: &str| format!("{}/?p={}", job.api.site.to_url().trim_end_matches('/'), posts[key].post_id);
    let mut uploads = job.uploads.clone();
    let (mut uploaded, mut reused) = (0, 0);
    let mut index = None;
    for (at, part) in parts.iter().enumerate() {
        let mut blocks = part.blocks.clone();
        let mut story = job.story.clone();
        if part.key == INDEX {
            let days: Vec<String> = parts[1..].iter().map(|day| format!("[{}]({})", escape_marks(&day.heading), link(&day.key))).collect();
            if !days.is_empty() {
                blocks.push(StoryBlock::new(Block::List { ordered: false, items: days }));
            }
        } else {
            story = Story { title: title(part), subtitle: String::new(), introduction: String::new(), conclusion: String::new(), cover: None, ..story };
            let mut links = vec![format!("[{}]({})", escape_marks(job.story.title.trim()), link(INDEX))];
            if at > 1 {
                links.insert(0, format!("← [{}]({})", escape_marks(&parts[at - 1].heading), link(&parts[at - 1].key)));
            }
            if let Some(next) = parts.get(at + 1) {
                links.push(format!("[{}]({}) →", escape_marks(&next.heading), link(&next.key)));
            }
            blocks.push(StoryBlock::new(Block::Separator));
            blocks.push(StoryBlock::new(Block::Paragraph(links.join(" · "))));
        }
        let wanted: std::collections::HashSet<i64> = blocks.iter().flat_map(|block| block.block.media()).map(|item| item.media_id).chain(story.cover).collect();
        let sources = job.sources.iter().filter(|(id, _)| wanted.contains(id)).map(|(id, source)| (*id, source.clone())).collect();
        let post = &posts[&part.key];
        let one = Job {
            story,
            blocks,
            sources,
            uploads: uploads.clone(),
            post_id: Some(post.post_id),
            fingerprint: post.fingerprint.clone(),
            // Already checked above, for every post at once.
            overwrite: true,
            update_published: true,
            pending_since: None,
            allow_new: true,
            days: None,
            ..job.clone()
        };
        let saved = run(http, &one, progress, &mut |event| {
            if let Event::Uploaded { hash, media_id, url, .. } = &event {
                uploads.insert(hash.clone(), (*media_id, url.clone()));
            }
            report(event);
        })
        .map_err(|stopped| match stopped {
            Stopped::Cancelled { uploaded: now } => Stopped::Cancelled { uploaded: uploaded + now },
            Stopped::Site { failure, uploaded: now } => Stopped::Site { failure, uploaded: uploaded + now },
            Stopped::Photo { media_id, uploaded: now } => Stopped::Photo { media_id, uploaded: uploaded + now },
            other => other,
        })?;
        uploaded += saved.uploaded;
        reused += saved.reused;
        report(Event::Part { story_id: job.story.id, revision: job.story.revision, key: part.key.clone(), post: DayPost { post_id: saved.post_id, status: saved.status.clone(), fingerprint: saved.fingerprint.clone(), changed_on_site: false } });
        if part.key == INDEX {
            index = Some(saved);
        }
    }
    let index = index.expect("the index is saved first");
    Ok(Saved { uploaded, reused, days: parts.len() - 1, ..index })
}

/// `text` as a link's words: square brackets, which would end the link,
/// made round, and the marks of inline styles taken as letters.
fn escape_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '[' => out.push('('),
            ']' => out.push(')'),
            '\\' | '*' | '~' | '`' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// A folder for one job's rendered photos, removed when the job ends.
struct Staging(std::path::PathBuf);

impl Staging {
    fn new(story_id: i64) -> Self {
        static JOBS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let job = JOBS.fetch_add(1, Ordering::Relaxed);
        Staging::remove_abandoned(&std::env::temp_dir());
        let dir = std::env::temp_dir().join(format!("picasso-publish-{}-{story_id}-{job}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        Staging(dir)
    }

    fn path(&self, hash: &str) -> std::path::PathBuf {
        self.0.join(format!("{hash}.jpg"))
    }

    /// Removes the folders in `temp` of jobs cut short by a crash or a power
    /// cut; those of another running Picasso are left alone.
    fn remove_abandoned(temp: &Path) {
        let Ok(entries) = std::fs::read_dir(temp) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let pid = name.to_str().and_then(|name| name.strip_prefix("picasso-publish-")).and_then(|rest| rest.split('-').next()).and_then(|pid| pid.parse::<u32>().ok());
            if pid.is_some_and(|pid| pid != std::process::id() && crate::story_html::abandoned(pid, &entry.path())) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The site's media ids in `by_hash`, each once, in order.
fn used(by_hash: &HashMap<String, Uploaded>) -> Vec<i64> {
    let mut ids: Vec<i64> = by_hash.values().map(|item| item.id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// How far this computer's clock may be ahead of the site's when looking
/// for a draft made since a moment.
const CLOCK_SLACK: i64 = 15 * 60;

/// The draft titled `title` made since `since` whose answer was lost, if
/// the site has exactly one; with more, it stops rather than guess.
fn lost_draft(client: &Client, job: &Job, title: &str, since: i64, uploaded: usize) -> Result<Option<i64>, Stopped> {
    let found = client.find_drafts(&job.api, &job.credentials, title, since).map_err(|failure| Stopped::Site { failure, uploaded })?;
    match found.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(*id)),
        _ => Err(Stopped::Uncertain { drafts: found.len() }),
    }
}

/// The post as WordPress takes it, with the chosen terms.
fn draft_post(post: gutenberg::Post, terms: &crate::db::PostTerms) -> DraftPost {
    let ids = |list: &[(i64, String)]| (!list.is_empty()).then(|| list.iter().map(|(id, _)| *id).collect());
    DraftPost { title: post.title, excerpt: post.excerpt, content: post.content, featured: post.featured, categories: ids(&terms.categories), tags: ids(&terms.tags), publish: false, schedule: None }
}

/// The post a save goes to, as the publishing dialog settled it.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// The post changed in place; none makes a new draft.
    pub earlier: Option<crate::db::Publication>,
    /// Changing it was chosen though it is edited on the site or public.
    pub update: bool,
    /// A new draft may be made though a post with the Story's title is on the site.
    pub allow_new: bool,
}

/// Where a save of Story `story_id` on site `site_id` goes. `earlier` is the
/// post the library knows; `update` is the user's answer when it is edited
/// on the site or public: change it (`Some(true)`) or save a new draft
/// beside it, which then takes its place in the library and never changes
/// it. `found` is a post with the Story's title the library has no record
/// of, and `adopt` the answer to taking it up.
pub fn target(story_id: i64, site_id: i64, size: UploadSize, earlier: Option<crate::db::Publication>, update: Option<bool>, found: Option<i64>, adopt: Option<bool>) -> Target {
    let update = update == Some(true);
    let earlier = earlier.filter(|publication| (publication.remote_status == "draft" && !publication.changed_on_site) || update);
    let allow_new = found.is_none() || adopt == Some(false);
    match found.filter(|_| adopt == Some(true)) {
        // Taken up: changed in place, keeping its status.
        Some(post_id) => Target {
            earlier: Some(crate::db::Publication { story_id, site_id, post_id, remote_status: "publish".into(), size: size.code().into(), revision: 0, synced_at: 0, fingerprint: String::new(), changed_on_site: false }),
            update: true,
            allow_new,
        },
        None => Target { earlier, update, allow_new },
    }
}

/// An upload's name, without its extension: the Story's title and the
/// photo's file name, so the site's media library is readable, then part of
/// the bytes' digest, so the same bytes are always named the same and others
/// never are. Only lowercase letters, digits and dashes are kept, as
/// WordPress keeps them unchanged in the upload's title.
pub fn upload_name(title: &str, file: &str, hash: &str) -> String {
    let words = |text: &str, most: usize| {
        let mut out = String::new();
        for ch in text.to_lowercase().chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
            } else if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        }
        let whole = out.trim_end_matches('-');
        if whole.len() <= most {
            return whole.to_string();
        }
        // Cut between words where one ends in time, so none is left half.
        let cut = &whole[..most];
        let cut = if whole.as_bytes()[most] == b'-' { cut } else { cut.rsplit_once('-').map_or(cut, |(kept, _)| kept) };
        cut.trim_end_matches('-').to_string()
    };
    let mut parts: Vec<String> = [words(title, 24), words(file, 20)].into_iter().filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        parts.push("picasso".into());
    }
    parts.push(hash[..8].to_string());
    parts.join("-")
}

/// The hash and length of what photo `media_id` rendered to before, when the
/// site has those bytes, so it needn't be rendered again.
fn rendered_before(job: &Job, media_id: i64) -> Option<(String, u64)> {
    let (path, settings, rotation) = job.sources.get(&media_id.abs()).cloned().flatten()?;
    let crop = (media_id < 0).then_some(job.story.cover_crop);
    let key = render_key(&path, &settings, rotation, job.size, job.exif, crop)?;
    job.renders.get(&key).filter(|(hash, _)| job.uploads.contains_key(hash))
}

/// Renders photo `media_id` into `staging` under the hash of its bytes, and
/// keeps what it became in the job's [`Renders`]. Returns the hash and length.
fn render_photo(job: &Job, media_id: i64, staging: &Staging, uploaded: usize) -> Result<(String, u64), Stopped> {
    let failed = || Stopped::Photo { media_id: media_id.abs(), uploaded };
    let (path, settings, rotation) = job.sources.get(&media_id.abs()).cloned().flatten().ok_or_else(failed)?;
    let crop = (media_id < 0).then_some(job.story.cover_crop);
    let key = render_key(&path, &settings, rotation, job.size, job.exif, crop);
    let photo = upload::render(&path, &settings, rotation, job.size, job.exif, crop).map_err(|_| failed())?;
    // The same bytes from two photos are written by whichever comes first.
    match std::fs::OpenOptions::new().write(true).create_new(true).open(staging.path(&photo.hash)) {
        Ok(mut file) => std::io::Write::write_all(&mut file, &photo.bytes).map_err(|_| failed())?,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(failed()),
    }
    let bytes = photo.bytes.len() as u64;
    if let Some(key) = key {
        job.renders.keep(key, &photo.hash, bytes);
    }
    Ok((photo.hash, bytes))
}

/// A confirmed upload of these bytes that is still on the site, or the
/// bytes uploaded now, reported as each happens. Only an upload made now is
/// given `alt`: one already on the site may be in other posts. `stem` names
/// it, and must be the same for the same bytes, as a lost upload is found by it.
/// `bytes` is asked for only when they are sent.
#[allow(clippy::too_many_arguments)]
fn place(client: &Client, job: &Job, bytes: &mut dyn FnMut() -> Result<Vec<u8>, Stopped>, hash: &str, stem: &str, alt: &str, known: &HashMap<i64, Media>, uploaded: &mut usize, reused: &mut usize, report: &mut dyn FnMut(Event)) -> Result<Uploaded, Stopped> {
    let site = |failure: Failure, uploaded: usize| Stopped::Site { failure, uploaded };
    if let Some((id, url)) = job.uploads.get(hash) {
        let now = match known.get(id) {
            Some(now) => Some(Some(now.clone())),
            None => client.media_now(&job.api, &job.credentials, *id).map_err(|failure| site(failure, *uploaded))?,
        };
        if let Some(now) = now {
            *reused += 1;
            // An address the site has changed since, as after a move, is the one used and kept.
            if let Some(now) = now.filter(|now| now.url != *url) {
                report(Event::Uploaded { hash: hash.to_string(), size: job.size, media_id: now.id, url: now.url.clone(), details: now.details });
                return Ok(Uploaded { id: now.id, url: now.url });
            }
            return Ok(Uploaded { id: *id, url: url.clone() });
        }
        report(Event::Gone { hash: hash.to_string() });
    }
    let bytes = bytes()?;
    let sent = match client.upload_media(&job.api, &job.credentials, &bytes, &format!("{stem}.jpg")) {
        Ok(sent) => sent,
        // Sent but not answered: the upload is found by its name, as the same
        // bytes are always named the same; any of them is these bytes.
        Err(failure) if failure.outcome_unknown() => {
            let found = client.find_media(&job.api, &job.credentials, stem).map_err(|failure| site(failure, *uploaded))?;
            found.into_iter().next().ok_or_else(|| site(failure, *uploaded))?
        }
        Err(failure) => return Err(site(failure, *uploaded)),
    };
    *uploaded += 1;
    // The post carries the description too, so a refusal here costs nothing.
    if !alt.is_empty() {
        let _ = client.set_media_alt(&job.api, &job.credentials, sent.id, alt);
    }
    report(Event::Uploaded { hash: hash.to_string(), size: job.size, media_id: sent.id, url: sent.url.clone(), details: sent.details.clone() });
    Ok(Uploaded { id: sent.id, url: sent.url })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Settings;
    use crate::trip::{Block, MediaUse};
    use crate::wordpress::tests::{local_api, Fake};
    use image::{Rgba, RgbaImage};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso-publish-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn job(dir: &std::path::Path, post_id: Option<i64>, uploads: HashMap<String, (i64, String)>) -> Job {
        let path = dir.join("a.png");
        RgbaImage::from_fn(300, 200, |x, y| Rgba([x as u8, y as u8, 90, 255])).save(&path).unwrap();
        let story = Story {
            id: 3,
            trip_id: 1,
            title: "Spain".into(),
            subtitle: String::new(),
            author: String::new(),
            show_date: false,
            introduction: String::new(),
            conclusion: String::new(),
            cover: Some(11),
            cover_crop: crate::trip::CoverCrop::default(),
            template: crate::trip::DEFAULT_TEMPLATE.into(),
            archived: false,
            finished: false,
            revision: 1,
            updated_at: 0,
        };
        let blocks = vec![StoryBlock::new(Block::Photo(MediaUse::new(11))), StoryBlock::new(Block::Paragraph("Hola".into()))];
        let sources = HashMap::from([(11, Some((path, Settings::default(), 0)))]);
        Job {
            api: local_api(),
            credentials: Credentials { user: "ana".into(), password: "abcd efgh".into() },
            policy: Policy::default(),
            story,
            blocks,
            sources,
            size: UploadSize::Low,
            exif: true,
            uploads,
            post_id,
            fingerprint: String::new(),
            overwrite: false,
            pending_since: None,
            publish: false,
            schedule: None,
            days: None,
            allow_new: true,
            map: None,
            profiles: HashMap::new(),
            update_published: false,
            terms: crate::db::PostTerms::default(),
            renders: Renders::default(),
        }
    }

    const MEDIA: &str = "https://example.com/blog/wp-json/wp/v2/media";
    const POSTS: &str = "https://example.com/blog/wp-json/wp/v2/posts";

    #[test]
    fn a_story_becomes_a_draft_with_its_photo_uploaded_once() {
        let dir = temp_dir("new");
        let job = job(&dir, None, HashMap::new());
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        let saved = run(&fake, &job, &Progress::default(), &mut |event| events.push(event)).unwrap();
        assert_eq!((saved.post_id, saved.uploaded, saved.reused), (7, 1, 0), "the cover and the photo are one upload");
        assert_eq!(saved.media, [41], "the post's media is kept with it");
        assert_eq!(saved.edit_link, "https://example.com/blog/wp-admin/post.php?post=7&action=edit");
        assert!(matches!(&events[..], [Event::Prepared { photos: 1, bytes }, Event::Uploaded { media_id: 41, size: UploadSize::Low, .. }, Event::Creating { story_id: 3, .. }] if *bytes > 0), "the bytes are known before any is sent: {events:?}");
        let bodies = fake.bodies.borrow();
        let post: serde_json::Value = serde_json::from_slice(&bodies.last().unwrap().1).unwrap();
        assert_eq!(post["status"], "draft");
        assert_eq!(post["excerpt"], "", "the subtitle is the excerpt, sent even when empty");
        assert_eq!(post["featured_media"], 41);
        assert!(post["content"].as_str().unwrap().contains("wp-image-41"));
        assert!(post.get("categories").is_none() && post.get("tags").is_none(), "none chosen, none sent");
        drop(bodies);

        // Chosen categories go with the post; tags left unchosen aren't sent.
        let chosen = Job { terms: crate::db::PostTerms { categories: vec![(5, "Spain".into())], tags: Vec::new() }, ..job };
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":8}"#);
        run(&fake, &chosen, &Progress::default(), &mut |_| {}).unwrap();
        let post: serde_json::Value = serde_json::from_slice(&fake.bodies.borrow().last().unwrap().1).unwrap();
        assert_eq!(post["categories"], serde_json::json!([5]));
        assert!(post.get("tags").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_story_splits_at_its_headings_into_an_index_and_days() {
        let blocks = [Block::Paragraph("Why".into()), Block::Heading(" Granada ".into()), Block::Photo(MediaUse::new(11)), Block::Subheading("Evening".into()), Block::Heading("Toledo".into())];
        let blocks: Vec<StoryBlock> = blocks.into_iter().map(StoryBlock::new).collect();
        let parts = split_days(&blocks);
        let shape: Vec<(&str, usize)> = parts.iter().map(|part| (part.heading.as_str(), part.blocks.len())).collect();
        assert_eq!(shape, [("", 1), ("Granada", 2), ("Toledo", 0)], "a subheading stays in its Day");
        assert_eq!((parts[0].key.as_str(), parts[1].key.clone()), (INDEX, day_key(&blocks[1].uid)), "a Day is keyed by its heading, not its place");
    }

    #[test]
    fn a_story_by_day_makes_each_post_once_and_links_them() {
        let dir = temp_dir("days");
        let blocks = vec![
            StoryBlock::new(Block::Paragraph("Why we went".into())),
            StoryBlock::new(Block::Heading("Granada [old]".into())),
            StoryBlock::new(Block::Photo(MediaUse::new(11))),
            StoryBlock::new(Block::Heading("Toledo".into())),
            StoryBlock::new(Block::Paragraph("Rain".into())),
        ];
        let known = HashMap::from([(INDEX.to_string(), DayPost { post_id: 5, status: "draft".into(), ..DayPost::default() })]);
        let job = Job { blocks, days: Some(known), ..job(&dir, None, HashMap::new()) };
        let fake = Fake::default()
            .answer("https://example.com/blog/wp-json/wp/v2/posts/5?context=edit&_fields=id,status,title,excerpt,content,featured_media", 200, &[], r#"{"id":5,"status":"draft"}"#)
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":8,"status":"draft"}"#)
            .answer_to("GET", &format!("{POSTS}/8*"), 200, &[], r#"{"id":8,"status":"draft"}"#)
            .answer_to("GET", &format!("{POSTS}/5*"), 200, &[], r#"{"id":5,"status":"draft"}"#)
            .answer_to("POST", &format!("{POSTS}/8"), 200, &[], r#"{"id":8,"status":"draft"}"#)
            .answer_to("POST", &format!("{POSTS}/5"), 200, &[], r#"{"id":5,"status":"draft"}"#)
            .answer_to("GET", &format!("{MEDIA}/41*"), 200, &[], r#"{"id":41}"#);
        let mut events = Vec::new();
        let saved = run_days(&fake, &job, &Progress::default(), &mut |event| events.push(event)).unwrap();
        assert_eq!((saved.post_id, saved.days, saved.uploaded, saved.reused), (5, 2, 1, 1), "the index is the post named; the cover, also in a Day, is sent once");
        let bodies = fake.bodies.borrow();
        let made: Vec<serde_json::Value> = bodies.iter().filter(|(key, _)| *key == format!("POST {POSTS}")).map(|(_, body)| serde_json::from_slice(body).unwrap()).collect();
        let titles: Vec<&str> = made.iter().map(|post| post["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Spain – Granada [old]", "Spain – Toledo"], "only the Days without a post get one, empty");
        let index: serde_json::Value = serde_json::from_slice(&bodies.iter().find(|(key, _)| *key == format!("POST {POSTS}/5")).unwrap().1).unwrap();
        let content = index["content"].as_str().unwrap();
        assert!(content.contains("Why we went") && content.contains(r#"<a href="https://example.com/blog/?p=8">Granada (old)</a>"#), "{content}");
        let day: serde_json::Value = serde_json::from_slice(&bodies.iter().find(|(key, _)| *key == format!("POST {POSTS}/8")).unwrap().1).unwrap();
        assert!(day["content"].as_str().unwrap().contains(r#"<a href="https://example.com/blog/?p=5">Spain</a>"#), "a Day links back to the index");
        drop(bodies);
        let parts = events.iter().filter(|event| matches!(event, Event::Part { .. })).count();
        assert_eq!(parts, 1 + 2 + 3, "the index checked, two posts made, three saved: each kept as it happens");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_time_already_past_on_the_sites_clock_sends_nothing() {
        let dir = temp_dir("soon");
        let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(9, 3, 0).unwrap();
        let job = Job { publish: true, schedule: Some(at), ..job(&dir, None, HashMap::new()) };
        // 08:00 UTC is 09:00 on a site an hour ahead: 09:03 is too close.
        let fake = Fake::default().answer("https://example.com/blog/wp-json/", 200, &[("date", "Sun, 01 Nov 2026 08:00:00 GMT")], r#"{"gmt_offset":1}"#);
        assert_eq!(run(&fake, &job, &Progress::default(), &mut |_| {}).err(), Some(Stopped::TooSoon));
        assert!(fake.bodies.borrow().is_empty(), "nothing was sent");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_upload_is_named_by_its_story_and_file() {
        let hash = "3fa2b1c94d07a1b2c3d4";
        assert_eq!(upload_name("Spain – 2025-07-01", "IMG_1234", hash), "spain-2025-07-01-img-1234-3fa2b1c9");
        assert_eq!(upload_name("Spain – 2025-07-01", "2026.10.17-08h52m37s", hash), "spain-2025-07-01-2026-10-17-08h52m37s-3fa2b1c9");
        assert_eq!(upload_name("京都", "DSC 0001 (2)", hash), "dsc-0001-2-3fa2b1c9", "letters WordPress might change are left out");
        assert_eq!(upload_name("", "", hash), "picasso-3fa2b1c9");
        assert_eq!(upload_name("A long walk from Granada to the sea", "a-very-long-file-name-from-a-phone", hash), "a-long-walk-from-granada-a-very-long-file-3fa2b1c9", "each part is kept short, cut between words");
    }

    #[test]
    fn a_new_upload_carries_the_photo_description_and_a_reused_one_is_left_alone() {
        let dir = temp_dir("alt");
        let photo = MediaUse { alt: "  A red gate ".into(), ..MediaUse::new(11) };
        let first = Job { blocks: vec![StoryBlock::new(Block::Photo(photo))], ..job(&dir, None, HashMap::new()) };
        let alt = format!("POST {MEDIA}/41");
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", &format!("{MEDIA}/41"), 200, &[], r#"{"id":41}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        run(&fake, &first, &Progress::default(), &mut |event| events.push(event)).unwrap();
        let bodies = fake.bodies.borrow();
        let (_, sent) = bodies.iter().find(|(key, _)| *key == alt).expect("the description is set on the upload");
        assert_eq!(serde_json::from_slice::<serde_json::Value>(sent).unwrap(), serde_json::json!({ "alt_text": "A red gate" }));
        drop(bodies);

        let Some(Event::Uploaded { hash, .. }) = events.into_iter().find(|event| matches!(event, Event::Uploaded { .. })) else { panic!() };
        let known = HashMap::from([(hash, (41, "https://example.com/blog/a.jpg".to_string()))]);
        let again = Job { blocks: first.blocks.clone(), ..job(&dir, Some(7), known) };
        let fake = Fake::default()
            .answer_to("GET", &format!("{MEDIA}/41*"), 200, &[], r#"{"id":41}"#)
            .answer_to("GET", &format!("{POSTS}/7*"), 200, &[], r#"{"id":7,"status":"draft"}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        run(&fake, &again, &Progress::default(), &mut |_| {}).unwrap();
        assert!(!fake.bodies.borrow().iter().any(|(key, _)| *key == alt), "an upload already on the site may be in other posts");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn photos_staged_by_a_job_cut_short_are_cleared_by_the_next() {
        let temp = temp_dir("staging-left");
        let mut ended = std::process::Command::new("true").spawn().unwrap();
        ended.wait().unwrap();
        let crashed = temp.join(format!("picasso-publish-{}-3-0", ended.id()));
        let running = temp.join(format!("picasso-publish-{}-3-0", std::os::unix::process::parent_id()));
        let other = temp.join("picasso-publish-notes");
        for dir in [&crashed, &running, &other] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("a.jpg"), "rendered").unwrap();
        }
        Staging::remove_abandoned(&temp);
        assert!(!crashed.exists(), "a stopped job's photos are cleared");
        assert!(running.join("a.jpg").is_file(), "a running Picasso's are left alone");
        assert!(other.join("a.jpg").is_file(), "anything else is left alone");
        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn a_photo_the_site_has_is_not_rendered_again_until_it_must_be_sent() {
        let dir = temp_dir("renders");
        let first = job(&dir, None, HashMap::new());
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        run(&fake, &first, &Progress::default(), &mut |event| events.push(event)).unwrap();
        let Some(Event::Uploaded { hash, .. }) = events.iter().find(|event| matches!(event, Event::Uploaded { .. })).cloned() else { panic!("{events:?}") };
        let known = HashMap::from([(hash.clone(), (41, "https://example.com/blog/a.jpg".to_string()))]);
        let again = Job { uploads: known, post_id: Some(7), ..first.clone() };

        // The file can no longer be read as a photo, though it looks the same:
        // publishing again works only because it isn't rendered.
        let path = dir.join("a.png");
        let original = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let keep_looks = |bytes: &[u8]| {
            std::fs::write(&path, bytes).unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();
        };
        keep_looks(&vec![0; original.len()]);
        let still_there = || {
            Fake::default()
                .answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"draft"}"#)
                .answer(&format!("{MEDIA}?include=41&per_page=100&context=edit&_fields=id,source_url,mime_type,media_details"), 200, &[], r#"[{"id":41,"source_url":"https://example.com/blog/a.jpg"}]"#)
                .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#)
        };
        let mut events = Vec::new();
        let saved = run(&still_there(), &again, &Progress::default(), &mut |event| events.push(event)).unwrap();
        assert_eq!((saved.uploaded, saved.reused, saved.media.clone()), (0, 1, vec![41]));
        assert!(matches!(&events[..], [Event::Prepared { photos: 1, bytes }] if *bytes > 0), "its size is still known: {events:?}");

        // Another edit is another picture, which must be rendered.
        let mut edited = again.clone();
        let mut brighter = Settings::default();
        brighter.set(crate::edit::Control::from_key("light.exposure").unwrap(), 0.5);
        edited.sources.insert(11, Some((path.clone(), brighter, 0)));
        assert!(matches!(run(&still_there(), &edited, &Progress::default(), &mut |_| {}), Err(Stopped::Photo { media_id: 11, .. })));

        // Gone from the site: rendered now, the same bytes as before, and sent.
        keep_looks(&original);
        let lost = Fake::default()
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"draft"}"#)
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":52,"source_url":"https://example.com/blog/b.jpg"}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        let saved = run(&lost, &again, &Progress::default(), &mut |event| events.push(event)).unwrap();
        assert_eq!((saved.uploaded, saved.reused, saved.media.clone()), (1, 0, vec![52]));
        assert!(events.iter().any(|event| matches!(event, Event::Uploaded { hash: sent, media_id: 52, .. } if *sent == hash)), "{events:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_confirmed_upload_is_reused_and_a_removed_one_sent_again() {
        let dir = temp_dir("reuse");
        let first = job(&dir, None, HashMap::new());
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        run(&fake, &first, &Progress::default(), &mut |event| events.push(event)).unwrap();
        let Some(Event::Uploaded { hash, .. }) = events.iter().find(|event| matches!(event, Event::Uploaded { .. })).cloned() else { panic!("{events:?}") };

        // Still on the site: nothing is sent again, and the draft is changed in place.
        let known = HashMap::from([(hash.clone(), (41, "https://example.com/blog/a.jpg".to_string()))]);
        let again = job(&dir, Some(7), known.clone());
        let fake = Fake::default()
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"draft"}"#)
            .answer(&format!("{MEDIA}/41?context=edit&_fields=id,source_url,mime_type,media_details"), 200, &[], r#"{"id":41}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        let saved = run(&fake, &again, &Progress::default(), &mut |_| {}).unwrap();
        assert_eq!((saved.post_id, saved.uploaded, saved.reused), (7, 0, 1));
        assert!(!fake.bodies.borrow().iter().any(|(key, _)| key.starts_with("POST") && key.ends_with("/media")));

        // Moved on the site meanwhile: its address today is used and kept.
        let fake = Fake::default()
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"draft"}"#)
            .answer(&format!("{MEDIA}/41?context=edit&_fields=id,source_url,mime_type,media_details"), 200, &[], r#"{"id":41,"source_url":"https://new.example.com/a.jpg"}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        let saved = run(&fake, &again, &Progress::default(), &mut |event| events.push(event)).unwrap();
        assert_eq!((saved.uploaded, saved.reused), (0, 1));
        assert!(events.iter().any(|event| matches!(event, Event::Uploaded { url, .. } if url == "https://new.example.com/a.jpg")), "{events:?}");
        assert!(fake.bodies.borrow().iter().any(|(key, body)| key.starts_with("POST") && String::from_utf8_lossy(body).contains("new.example.com/a.jpg")));

        // Checked with the others in one read, with no read of its own.
        let fake = Fake::default()
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"draft"}"#)
            .answer(&format!("{MEDIA}?include=41&per_page=100&context=edit&_fields=id,source_url,mime_type,media_details"), 200, &[], r#"[{"id":41,"source_url":"https://example.com/blog/a.jpg"}]"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        let saved = run(&fake, &again, &Progress::default(), &mut |_| {}).unwrap();
        assert_eq!((saved.uploaded, saved.reused), (0, 1));
        assert!(!fake.asked.borrow().iter().any(|(url, _)| url.contains("/media/41")));

        // Removed from the site meanwhile: forgotten and sent again.
        let fake = Fake::default()
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"draft"}"#)
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":52,"source_url":"https://example.com/blog/b.jpg"}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        let saved = run(&fake, &job(&dir, Some(7), known), &Progress::default(), &mut |event| events.push(event)).unwrap();
        assert_eq!(saved.uploaded, 1);
        assert!(events.contains(&Event::Gone { hash }), "{events:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_published_post_is_updated_when_chosen_and_keeps_its_status() {
        let dir = temp_dir("update");
        let fake = Fake::default()
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"publish"}"#)
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        let mut chosen = job(&dir, Some(7), HashMap::new());
        chosen.update_published = true;
        let saved = run(&fake, &chosen, &Progress::default(), &mut |_| {}).unwrap();
        assert_eq!((saved.post_id, saved.status.as_str()), (7, "publish"));
        let sent = fake.bodies.borrow().last().cloned().unwrap();
        assert!(!String::from_utf8_lossy(&sent.1).contains("\"status\""), "the post's status is not changed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_post_that_is_no_longer_a_draft_is_left_alone() {
        let dir = temp_dir("published");
        let fake = Fake::default().answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"publish"}"#);
        let result = run(&fake, &job(&dir, Some(7), HashMap::new()), &Progress::default(), &mut |_| {});
        assert_eq!(result, Err(Stopped::NotDraft("publish".into())));
        assert!(fake.bodies.borrow().is_empty(), "nothing was sent");
        let trashed = Fake::default().answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"trash"}"#);
        let mut chosen = job(&dir, Some(7), HashMap::new());
        chosen.update_published = true;
        assert_eq!(run(&trashed, &chosen, &Progress::default(), &mut |_| {}), Err(Stopped::NotDraft("trash".into())), "a trashed post is never changed");
        let gone = Fake::default();
        assert_eq!(run(&gone, &job(&dir, Some(7), HashMap::new()), &Progress::default(), &mut |_| {}), Err(Stopped::PostGone));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_post_edited_on_the_site_is_left_alone_unless_replacing_was_chosen() {
        let dir = temp_dir("edited");
        let saved = r#"{"id":7,"status":"draft","title":{"raw":"Spain"},"content":{"raw":"<p>ours</p>"}}"#;
        let edited = r#"{"id":7,"status":"draft","title":{"raw":"Spain"},"content":{"raw":"<p>theirs</p>"}}"#;
        let fake = Fake::default()
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], saved)
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], saved);
        let first = run(&fake, &job(&dir, Some(7), HashMap::new()), &Progress::default(), &mut |_| {}).unwrap();
        let mut again = job(&dir, Some(7), HashMap::new());
        again.fingerprint = first.fingerprint.clone();
        assert!(run(&fake, &again, &Progress::default(), &mut |_| {}).is_ok(), "unchanged since: saved again");

        let mut older = job(&dir, Some(7), HashMap::new());
        older.fingerprint = "a digest from an earlier Picasso".into();
        let edited_since = Fake::default()
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], edited)
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], saved);
        assert!(run(&edited_since, &older, &Progress::default(), &mut |_| {}).is_ok(), "one made otherwise can't be compared");

        let changed = Fake::default().answer(&format!("{POSTS}/7?context=edit"), 200, &[], edited);
        assert_eq!(run(&changed, &again, &Progress::default(), &mut |_| {}), Err(Stopped::ChangedOnSite));
        assert!(changed.bodies.borrow().is_empty(), "nothing was sent");
        let replacing = changed
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], saved);
        again.overwrite = true;
        assert_eq!(run(&replacing, &again, &Progress::default(), &mut |_| {}).unwrap().fingerprint, first.fingerprint);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_post_the_library_lost_track_of_is_found_before_another_is_made() {
        let dir = temp_dir("restored");
        let fake = Fake::default().answer(&format!("{POSTS}?search=Spain&status=draft,publish*"), 200, &[], r#"[{"id":9,"title":{"raw":"Spain"}}]"#);
        let mut restored = job(&dir, None, HashMap::new());
        restored.allow_new = false;
        assert_eq!(run(&fake, &restored, &Progress::default(), &mut |_| {}), Err(Stopped::TitleOnSite { post_id: 9 }));
        assert!(fake.bodies.borrow().is_empty(), "nothing was sent");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_posts_sync_state_comes_from_the_librarys_record() {
        let mut publication = crate::db::Publication { story_id: 3, site_id: 1, post_id: 7, remote_status: "publish".into(), size: "low".into(), revision: 4, synced_at: 1, fingerprint: String::new(), changed_on_site: false };
        assert_eq!(sync_state(None, 4, false), SyncState::New);
        assert_eq!(sync_state(None, 4, true), SyncState::Unconfirmed);
        assert_eq!(sync_state(Some(&publication), 4, false), SyncState::Clean);
        assert_eq!(sync_state(Some(&publication), 5, false), SyncState::ChangedHere);
        publication.changed_on_site = true;
        assert_eq!(sync_state(Some(&publication), 4, false), SyncState::Conflict, "the site's edit comes first");
    }

    #[test]
    fn a_post_is_made_public_only_when_that_was_confirmed() {
        let dir = temp_dir("publish");
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":7}"#);
        let mut chosen = job(&dir, None, HashMap::new());
        chosen.publish = true;
        assert_eq!(run(&fake, &chosen, &Progress::default(), &mut |_| {}).unwrap().status, "publish");
        let sent: serde_json::Value = serde_json::from_slice(&fake.bodies.borrow().last().unwrap().1).unwrap();
        assert_eq!(sent["status"], "publish");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_draft_whose_answer_was_lost_is_found_instead_of_made_again() {
        let dir = temp_dir("lost");
        let found = r#"[{"id":9,"title":{"raw":"Spain"}}]"#;
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 504, &[], "")
            .answer(&format!("{POSTS}?search=Spain&status=draft,publish*"), 200, &[], found)
            .answer(&format!("{POSTS}/9?context=edit"), 200, &[], r#"{"id":9,"status":"draft"}"#);
        let mut events = Vec::new();
        let saved = run(&fake, &job(&dir, None, HashMap::new()), &Progress::default(), &mut |event| events.push(event)).unwrap();
        assert_eq!(saved.post_id, 9);
        assert!(matches!(events.last(), Some(Event::Creating { story_id: 3, .. })), "the intent is kept before the draft is made");
        assert_eq!(fake.bodies.borrow().iter().filter(|(key, _)| key == &format!("POST {POSTS}")).count(), 1, "never made twice");

        // After a restart, a draft made before is changed instead of another made.
        let restarted = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer(&format!("{POSTS}?search=Spain&status=draft,publish*"), 200, &[], found)
            .answer_to("POST", &format!("{POSTS}/9"), 200, &[], r#"{"id":9}"#);
        let mut again = job(&dir, None, HashMap::new());
        again.pending_since = Some(1_000);
        assert_eq!(run(&restarted, &again, &Progress::default(), &mut |_| {}).unwrap().post_id, 9);

        let two = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer(&format!("{POSTS}?search=Spain&status=draft,publish*"), 200, &[], r#"[{"id":9,"title":{"raw":"Spain"}},{"id":10,"title":{"raw":"Spain"}}]"#);
        assert_eq!(run(&two, &again, &Progress::default(), &mut |_| {}), Err(Stopped::Uncertain { drafts: 2 }), "more than one could be it: none is chosen");
        assert!(!two.bodies.borrow().iter().any(|(key, _)| key.contains("/posts")), "and no other is made");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unavailable_photo_or_a_cancel_stops_before_the_post() {
        let dir = temp_dir("stops");
        let mut missing = job(&dir, None, HashMap::new());
        missing.sources.insert(11, None);
        let fake = Fake::default();
        assert_eq!(run(&fake, &missing, &Progress::default(), &mut |_| {}), Err(Stopped::Photo { media_id: 11, uploaded: 0 }));
        let cancelled = Progress::default();
        cancelled.cancel.store(true, Ordering::Relaxed);
        assert_eq!(run(&fake, &job(&dir, None, HashMap::new()), &cancelled, &mut |_| {}), Err(Stopped::Cancelled { uploaded: 0 }));
        assert!(fake.bodies.borrow().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_draft_beside_an_edited_or_public_post_never_changes_it() {
        let earlier = |status: &str, changed_on_site: bool| crate::db::Publication {
            story_id: 3,
            site_id: 1,
            post_id: 7,
            remote_status: status.into(),
            size: "low".into(),
            revision: 1,
            synced_at: 1,
            fingerprint: "ab".into(),
            changed_on_site,
        };
        let to = |earlier: crate::db::Publication, update: Option<bool>| target(3, 1, UploadSize::Low, Some(earlier), update, None, None);
        assert_eq!(to(earlier("draft", false), None).earlier.map(|post| post.post_id), Some(7), "an untouched draft is changed in place");
        for (status, changed) in [("draft", true), ("publish", false)] {
            let new = to(earlier(status, changed), Some(false));
            assert_eq!((new.earlier, new.update, new.allow_new), (None, false, true), "{status}: a new draft");
            let replace = to(earlier(status, changed), Some(true));
            assert_eq!((replace.earlier.map(|post| post.post_id), replace.update), (Some(7), true), "{status}: changed as chosen");
        }
        let adopted = target(3, 1, UploadSize::Low, None, None, Some(30), Some(true));
        assert_eq!((adopted.earlier.map(|post| (post.post_id, post.remote_status)), adopted.update, adopted.allow_new), (Some((30, "publish".to_string())), true, false));
        let beside = target(3, 1, UploadSize::Low, None, None, Some(30), Some(false));
        assert_eq!((beside.earlier, beside.allow_new), (None, true));

        // The new draft is made without a word to post 7.
        let dir = temp_dir("copy");
        let chosen = to(earlier("draft", true), Some(false));
        let copy = Job { overwrite: chosen.update, ..job(&dir, chosen.earlier.map(|post| post.post_id), HashMap::new()) };
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":9}"#);
        assert_eq!(run(&fake, &copy, &Progress::default(), &mut |_| {}).map(|saved| saved.post_id), Ok(9));
        assert!(!fake.asked.borrow().iter().any(|(url, _)| url.contains("/posts/7")));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The photos the job sent, decoded.
    fn sent_photos(fake: &Fake) -> Vec<(u32, u32)> {
        let bodies = fake.bodies.borrow();
        let sent = bodies.iter().filter(|(key, _)| key.starts_with("POST") && key.ends_with("/media"));
        sent.map(|(_, body)| image::load_from_memory(body).unwrap()).map(|image| (image.width(), image.height())).collect()
    }

    #[test]
    fn a_photo_at_another_size_is_uploaded_again_never_reused() {
        let dir = temp_dir("sizes");
        let large = dir.join("large.jpg");
        image::RgbImage::from_fn(3000, 2000, |x, y| image::Rgb([(x / 12) as u8, (y / 8) as u8, 90])).save(&large).unwrap();
        let sized = |size: UploadSize, post_id: Option<i64>, uploads: HashMap<String, (i64, String)>| {
            let mut job = job(&dir, post_id, uploads);
            job.sources.insert(11, Some((large.clone(), Settings::default(), 0)));
            Job { size, ..job }
        };
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/standard.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        run(&fake, &sized(UploadSize::Standard, None, HashMap::new()), &Progress::default(), &mut |event| events.push(event)).unwrap();
        assert_eq!(sent_photos(&fake), [(2048, 1365)]);
        let Some(Event::Uploaded { hash, .. }) = events.into_iter().find(|event| matches!(event, Event::Uploaded { .. })) else { panic!() };

        // The same Story at Low resolution: its own upload, and the post uses it.
        let known = HashMap::from([(hash, (41, "https://example.com/blog/standard.jpg".to_string()))]);
        let fake = Fake::default()
            .answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"draft"}"#)
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":42,"source_url":"https://example.com/blog/low.jpg"}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        let saved = run(&fake, &sized(UploadSize::Low, Some(7), known), &Progress::default(), &mut |_| {}).unwrap();
        assert_eq!((saved.uploaded, saved.reused, saved.media), (1, 0, vec![42]));
        assert_eq!(sent_photos(&fake), [(1280, 853)]);
        let bodies = fake.bodies.borrow();
        let post: serde_json::Value = serde_json::from_slice(&bodies.last().unwrap().1).unwrap();
        assert_eq!(post["featured_media"], 42);
        assert!(!post["content"].as_str().unwrap().contains("standard.jpg"), "the larger upload isn't used");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_photo_that_fails_to_render_stops_before_anything_is_sent() {
        let dir = temp_dir("render");
        let damaged = dir.join("damaged.jpg");
        std::fs::write(&damaged, b"\xFF\xD8\xFF\xE0 not a whole photo").unwrap();
        let mut failing = job(&dir, Some(7), HashMap::from([("an-earlier-render".to_string(), (41, "https://example.com/blog/a.jpg".to_string()))]));
        failing.sources.insert(11, Some((damaged, Settings::default(), 0)));
        let fake = Fake::default().answer(&format!("{POSTS}/7?context=edit"), 200, &[], r#"{"status":"draft"}"#);
        assert_eq!(run(&fake, &failing, &Progress::default(), &mut |_| {}), Err(Stopped::Photo { media_id: 11, uploaded: 0 }));
        assert!(fake.bodies.borrow().is_empty(), "no upload, no earlier upload in its place, and the draft unchanged");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The quantization tables of a JPEG: what its quality was set to.
    fn quantization(jpeg: &[u8]) -> Vec<u8> {
        let (mut at, mut tables) = (2, Vec::new());
        while at + 4 <= jpeg.len() && jpeg[at] == 0xFF && jpeg[at + 1] != 0xDA {
            let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
            if jpeg[at + 1] == 0xDB {
                tables.extend(&jpeg[at + 4..at + 2 + len]);
            }
            at += 2 + len;
        }
        tables
    }

    #[test]
    fn each_size_sends_every_kind_of_photo_upright_fitted_at_its_quality_with_only_how_it_was_taken() {
        use exif::{Field, In, Tag, Value};
        let dir = temp_dir("both-sizes");
        let gradient = |w: u32, h: u32| image::RgbImage::from_fn(w, h, move |x, y| image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, 90]));

        // Turned in the camera: stored wide, shown tall, with where it was
        // taken and the camera's serial number.
        let field = |tag, value| Field { tag, ifd_num: In::PRIMARY, value };
        let ascii = |text: &str| Value::Ascii(vec![text.as_bytes().to_vec()]);
        let mut writer = exif::experimental::Writer::new();
        let fields = [
            field(Tag::Orientation, Value::Short(vec![6])),
            field(Tag::Model, ascii("X-T5")),
            field(Tag::BodySerialNumber, ascii("SERIAL-123")),
            field(Tag::GPSLatitudeRef, ascii("N")),
            field(Tag::GPSLatitude, Value::Rational(vec![(37, 1).into(), (10, 1).into(), (0, 1).into()])),
        ];
        fields.iter().for_each(|field| writer.push_field(field));
        let mut tiff = std::io::Cursor::new(Vec::new());
        writer.write(&mut tiff, false).unwrap();
        let tiff = tiff.into_inner();
        let mut app1 = vec![0xFF, 0xE1];
        app1.extend(u16::try_from(2 + 6 + tiff.len()).unwrap().to_be_bytes());
        app1.extend(b"Exif\0\0");
        app1.extend(tiff);
        let stored = crate::image_io::encode_jpeg(&image::DynamicImage::ImageRgb8(gradient(3000, 2000)).to_rgba8(), 92).unwrap();
        let turned = dir.join("turned.jpg");
        std::fs::write(&turned, [&stored[..2], &app1, &stored[2..]].concat()).unwrap();

        let tall = dir.join("tall.png");
        gradient(1500, 2600).save(&tall).unwrap();
        let small = dir.join("small.png");
        gradient(900, 600).save(&small).unwrap();
        let wide = dir.join("wide.jpg");
        gradient(3000, 2000).save(&wide).unwrap();
        let mut left_half = Settings::default();
        left_half.set_crop([0.0, 0.0, 0.5, 1.0]);
        let preview = crate::image_io::encode_jpeg(&image::DynamicImage::ImageRgb8(gradient(2400, 1600)).to_rgba8(), 90).unwrap();
        let raw = dir.join("raw.3fr");
        std::fs::write(&raw, crate::story_html::tests::raw_with_preview(&preview, 2400, 1600)).unwrap();

        let source = crate::photo_info::read_exif(&turned).unwrap();
        assert!(source.get_field(Tag::GPSLatitude, In::PRIMARY).is_some() && source.get_field(Tag::BodySerialNumber, In::PRIMARY).is_some());
        let at = |quality| quantization(&crate::image_io::encode_jpeg(&RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255])), quality).unwrap());
        assert!(!at(85).is_empty() && at(85) != at(80), "the two qualities can be told apart in the bytes");

        let originals = [&turned, &tall, &small, &wide, &raw];
        let checksums = |paths: &[&std::path::PathBuf]| paths.iter().map(|path| crate::upload::hash(&std::fs::read(path).unwrap())).collect::<Vec<_>>();
        let before = checksums(&originals);
        let top_half = crate::trip::CoverCrop([0, 0, 10_000, 5_000]);

        for size in UploadSize::ALL {
            let mut job = job(&dir, None, HashMap::new());
            job.sources = HashMap::from([
                (1, Some((turned.clone(), Settings::default(), 0))),
                (2, Some((tall.clone(), Settings::default(), 0))),
                (3, Some((small.clone(), Settings::default(), 0))),
                (4, Some((wide.clone(), left_half.clone(), 0))),
                (5, Some((raw.clone(), Settings::default(), 0))),
            ]);
            job.blocks = vec![
                StoryBlock::new(Block::Photo(MediaUse::new(1))),
                StoryBlock::new(Block::Gallery(vec![MediaUse::new(2), MediaUse::new(3), MediaUse::new(4)])),
                StoryBlock::new(Block::Photo(MediaUse::new(5))),
            ];
            job.story.cover = Some(2);
            job.story.cover_crop = top_half;
            let job = Job { size, ..job };
            let fake = Fake::default()
                .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
                .answer_to("POST", POSTS, 201, &[], r#"{"id":7}"#);
            run(&fake, &job, &Progress::default(), &mut |_| {}).unwrap();

            let fit = |w, h| crate::upload::fitted(w, h, size);
            let (tall_w, tall_h) = fit(1500, 2600);
            let cover = crate::story_html::crop_cover(&RgbaImage::new(tall_w, tall_h), top_half).dimensions();
            let mut expected = vec![fit(2000, 3000), (tall_w, tall_h), (900, 600), fit(1500, 2000), fit(2400, 1600), cover];
            let mut sent = sent_photos(&fake);
            expected.sort();
            sent.sort();
            assert_eq!(sent, expected, "{size:?}: upright, within {} px, never enlarged, cropped as edited and as framed", size.edge());
            assert!(sent.iter().all(|&(w, h)| w.max(h) <= size.edge()));

            let tables = at(size.quality());
            let bodies = fake.bodies.borrow();
            let uploads: Vec<&Vec<u8>> = bodies.iter().filter(|(key, _)| key.starts_with("POST") && key.ends_with("/media")).map(|(_, body)| body).collect();
            assert_eq!(uploads.len(), 6);
            for body in &uploads {
                assert_eq!(quantization(body), tables, "{size:?}: encoded at quality {}", size.quality());
                let has = |marker: &[u8]| body.windows(marker.len()).any(|window| window == marker);
                assert!(!has(b"SERIAL-123"), "no serial number is sent");
                let exif = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(body)).ok();
                let field = |tag| exif.as_ref().and_then(|exif| exif.get_field(tag, In::PRIMARY)).is_some();
                assert!(!field(Tag::Orientation) && !field(Tag::GPSLatitude) && !field(Tag::GPSLatitudeRef) && !field(Tag::BodySerialNumber));
            }
            let camera = uploads.iter().filter(|body| body.windows(4).any(|window| window == b"X-T5")).count();
            assert_eq!(camera, 1, "the camera that took it is kept, on its photo alone");
        }
        assert_eq!(checksums(&originals), before, "the originals are never written");
        let _ = std::fs::remove_dir_all(dir);
    }
}
