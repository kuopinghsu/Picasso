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
    Uploaded { hash: String, size: UploadSize, media_id: i64, url: String },
    /// An upload recorded before is no longer on the site.
    Gone { hash: String },
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
}

/// A finished job, for the library: which Story and site, in which size,
/// from which revision, and how it ended.
#[derive(Debug)]
pub struct Finished {
    pub story_id: i64,
    pub site_id: i64,
    pub site: String,
    pub size: UploadSize,
    pub revision: i64,
    pub outcome: Result<Saved, Stopped>,
}

pub fn run(http: &dyn Http, job: &Job, progress: &Progress, report: &mut dyn FnMut(Event)) -> Result<Saved, Stopped> {
    let client = Client::new(http, job.policy);
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
        if !job.fingerprint.is_empty() && found.fingerprint != job.fingerprint && !job.overwrite {
            return Err(Stopped::ChangedOnSite);
        }
        status = found.status;
    }
    let mut order: Vec<i64> = job.sources.keys().copied().collect();
    order.sort_unstable();
    progress.total.store(order.len() + 1, Ordering::Relaxed);
    let mut media: HashMap<i64, Uploaded> = HashMap::new();
    let mut by_hash: HashMap<String, Uploaded> = HashMap::new();
    for media_id in order {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(Stopped::Cancelled { uploaded });
        }
        let Some((path, settings, rotation)) = job.sources.get(&media_id).cloned().flatten() else {
            return Err(Stopped::Photo { media_id, uploaded });
        };
        let rendered = upload::render(&path, &settings, rotation, job.size, job.exif).map_err(|_| Stopped::Photo { media_id, uploaded })?;
        // The same photo used twice in the Story is sent once.
        if let Some(same) = by_hash.get(&rendered.hash) {
            media.insert(media_id, same.clone());
            progress.done.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let item = place(&client, job, &rendered.bytes, &rendered.hash, &mut uploaded, &mut reused, report)?;
        by_hash.insert(rendered.hash, item.clone());
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
                None => place(&client, job, &bytes, &hash, &mut uploaded, &mut reused, report)?,
            };
            by_hash.insert(hash, item.clone());
            maps.insert(block.uid.clone(), RouteMap { uploaded: item, credit: tiles.source.attribution() });
        }
    }
    if progress.cancel.load(Ordering::Relaxed) {
        return Err(Stopped::Cancelled { uploaded });
    }
    let post = gutenberg::post(&job.story, &job.blocks, &|id| media.get(&id).cloned(), &|uid| maps.get(uid).cloned()).map_err(Stopped::Unsupported)?;
    // A Story without a title makes a post WordPress shows as "(no title)".
    let saved = client
        .save_draft(&job.api, &job.credentials, job.post_id, &draft_post(post, &job.terms))
        .map_err(|failure| site(failure, uploaded))?;
    progress.done.fetch_add(1, Ordering::Relaxed);
    Ok(Saved { post_id: saved.id, edit_link: edit_link(&job.api, saved.id), uploaded, reused, status, fingerprint: saved.fingerprint })
}

/// The post as WordPress takes it, with the chosen terms.
fn draft_post(post: gutenberg::Post, terms: &crate::db::PostTerms) -> DraftPost {
    let ids = |list: &[(i64, String)]| (!list.is_empty()).then(|| list.iter().map(|(id, _)| *id).collect());
    DraftPost { title: post.title, content: post.content, featured: post.featured, categories: ids(&terms.categories), tags: ids(&terms.tags) }
}

/// A confirmed upload of these bytes that is still on the site, or the
/// bytes uploaded now, reported as each happens.
fn place(client: &Client, job: &Job, bytes: &[u8], hash: &str, uploaded: &mut usize, reused: &mut usize, report: &mut dyn FnMut(Event)) -> Result<Uploaded, Stopped> {
    let site = |failure: Failure, uploaded: usize| Stopped::Site { failure, uploaded };
    if let Some((id, url)) = job.uploads.get(hash) {
        if client.media_exists(&job.api, &job.credentials, *id).map_err(|failure| site(failure, *uploaded))? {
            *reused += 1;
            return Ok(Uploaded { id: *id, url: url.clone() });
        }
        report(Event::Gone { hash: hash.to_string() });
    }
    let name = format!("picasso-{}.jpg", &hash[..16]);
    let sent = client.upload_media(&job.api, &job.credentials, bytes, &name).map_err(|failure| site(failure, *uploaded))?;
    *uploaded += 1;
    report(Event::Uploaded { hash: hash.to_string(), size: job.size, media_id: sent.id, url: sent.url.clone() });
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
        assert_eq!(saved.edit_link, "https://example.com/blog/wp-admin/post.php?post=7&action=edit");
        assert!(matches!(&events[..], [Event::Uploaded { media_id: 41, size: UploadSize::Low, .. }]));
        let bodies = fake.bodies.borrow();
        let post: serde_json::Value = serde_json::from_slice(&bodies.last().unwrap().1).unwrap();
        assert_eq!(post["status"], "draft");
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
    fn a_confirmed_upload_is_reused_and_a_removed_one_sent_again() {
        let dir = temp_dir("reuse");
        let first = job(&dir, None, HashMap::new());
        let fake = Fake::default()
            .answer_to("POST", MEDIA, 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/a.jpg"}"#)
            .answer_to("POST", POSTS, 201, &[], r#"{"id":7}"#);
        let mut events = Vec::new();
        run(&fake, &first, &Progress::default(), &mut |event| events.push(event)).unwrap();
        let Event::Uploaded { hash, .. } = events[0].clone() else { panic!() };

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
        assert_eq!(events[0], Event::Gone { hash });
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
