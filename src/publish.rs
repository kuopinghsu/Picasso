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
use std::sync::atomic::Ordering;

use crate::gutenberg::{self, RouteMap, Unsupported, Uploaded};
use crate::story_html::{MapTiles, Progress, Source};
use crate::trip::{Block, Story, StoryBlock};
use crate::upload::{self, UploadSize};
use crate::wordpress::{edit_link, DraftPost, Api, Client, Credentials, Failure, Http, Policy};

/// Everything the job needs, read on the UI thread before it starts.
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
}

/// What the job learns that the library should keep at once.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The site confirmed an upload of these bytes.
    Uploaded { hash: String, size: UploadSize, media_id: i64, url: String, details: String },
    /// An upload recorded before is no longer on the site.
    Gone { hash: String },
    /// Every photo is rendered: this many different ones, of these bytes in
    /// all, are what may be sent, before anything is.
    Prepared { photos: usize, bytes: u64 },
    /// A draft is about to be made for the Story; until it is confirmed, one
    /// made after `since` (Unix seconds) is looked for before another is made.
    Creating { story_id: i64, since: i64 },
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

/// How a job ended, in a word or two for the library's publishing log;
/// a site's failure is named by its kind, which holds nothing secret.
pub fn outcome_code(outcome: &Result<Saved, Stopped>) -> String {
    match outcome {
        Ok(saved) if saved.published && saved.status == "future" => "scheduled".into(),
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
    order.sort_unstable();
    // Every photo is rendered before any is sent, to a folder of its own,
    // so the bytes to send are known first and none is held in memory.
    progress.total.store(order.len() * 2 + 1, Ordering::Relaxed);
    let staging = Staging::new(job.story.id);
    let mut rendered: Vec<(i64, String)> = Vec::new();
    let mut sizes: HashMap<String, u64> = HashMap::new();
    for &media_id in &order {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(Stopped::Cancelled { uploaded });
        }
        let Some((path, settings, rotation)) = job.sources.get(&media_id).cloned().flatten() else {
            return Err(Stopped::Photo { media_id, uploaded });
        };
        let photo = upload::render(&path, &settings, rotation, job.size, job.exif).map_err(|_| Stopped::Photo { media_id, uploaded })?;
        if !sizes.contains_key(&photo.hash) {
            std::fs::write(staging.path(&photo.hash), &photo.bytes).map_err(|_| Stopped::Photo { media_id, uploaded })?;
            sizes.insert(photo.hash.clone(), photo.bytes.len() as u64);
        }
        rendered.push((media_id, photo.hash));
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    report(Event::Prepared { photos: sizes.len(), bytes: sizes.values().sum() });
    // Each photo's first description in the Story goes with its upload.
    let mut alts: HashMap<i64, &str> = HashMap::new();
    for item in job.blocks.iter().flat_map(|block| block.block.media()) {
        if !item.alt.trim().is_empty() {
            alts.entry(item.media_id).or_insert(item.alt.trim());
        }
    }
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
        let bytes = std::fs::read(staging.path(&hash)).map_err(|_| Stopped::Photo { media_id, uploaded })?;
        let alt = alts.get(&media_id).copied().unwrap_or_default();
        let file = job.sources.get(&media_id).cloned().flatten().and_then(|(path, ..)| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()));
        let stem = upload_name(&job.story.title, file.as_deref().unwrap_or_default(), &hash);
        let item = place(&client, job, &bytes, &hash, &stem, alt, &mut uploaded, &mut reused, report)?;
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
                None => place(&client, job, &bytes, &hash, &upload_name(&job.story.title, "route", &hash), "", &mut uploaded, &mut reused, report)?,
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
    Ok(Saved { post_id: saved.id, edit_link: edit_link(&job.api, saved.id), uploaded, reused, status, fingerprint: saved.fingerprint, published: job.publish, scheduled: job.publish && job.schedule.is_some(), media: used(&by_hash) })
}

/// A folder for one job's rendered photos, removed when the job ends.
struct Staging(std::path::PathBuf);

impl Staging {
    fn new(story_id: i64) -> Self {
        static JOBS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let job = JOBS.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("picasso-publish-{}-{story_id}-{job}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        Staging(dir)
    }

    fn path(&self, hash: &str) -> std::path::PathBuf {
        self.0.join(format!("{hash}.jpg"))
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

/// A confirmed upload of these bytes that is still on the site, or the
/// bytes uploaded now, reported as each happens. Only an upload made now is
/// given `alt`: one already on the site may be in other posts. `stem` names
/// it, and must be the same for the same bytes, as a lost upload is found by it.
#[allow(clippy::too_many_arguments)]
fn place(client: &Client, job: &Job, bytes: &[u8], hash: &str, stem: &str, alt: &str, uploaded: &mut usize, reused: &mut usize, report: &mut dyn FnMut(Event)) -> Result<Uploaded, Stopped> {
    let site = |failure: Failure, uploaded: usize| Stopped::Site { failure, uploaded };
    if let Some((id, url)) = job.uploads.get(hash) {
        if client.media_exists(&job.api, &job.credentials, *id).map_err(|failure| site(failure, *uploaded))? {
            *reused += 1;
            return Ok(Uploaded { id: *id, url: url.clone() });
        }
        report(Event::Gone { hash: hash.to_string() });
    }
    let sent = match client.upload_media(&job.api, &job.credentials, bytes, &format!("{stem}.jpg")) {
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
            allow_new: true,
            map: None,
            profiles: HashMap::new(),
            update_published: false,
            terms: crate::db::PostTerms::default(),
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
            .answer(&format!("{MEDIA}/41?context=edit"), 200, &[], r#"{"id":41}"#)
            .answer_to("POST", &format!("{POSTS}/7"), 200, &[], r#"{"id":7}"#);
        let saved = run(&fake, &again, &Progress::default(), &mut |_| {}).unwrap();
        assert_eq!((saved.post_id, saved.uploaded, saved.reused), (7, 0, 1));
        assert!(!fake.bodies.borrow().iter().any(|(key, _)| key.starts_with("POST") && key.ends_with("/media")));

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
}
