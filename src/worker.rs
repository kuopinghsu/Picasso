//! Background folder indexing and thumbnail decode.
//!
//! A folder scan runs as a small pipeline:
//!
//! 1. A walker lists the folder, one directory per task on a small pool, and
//!    compares each file's size and modification time, taken from the
//!    listing, with the catalog. Unchanged files are skipped without being
//!    opened.
//! 2. A pool of probe threads reads dimensions and metadata from the head of
//!    each file that is new or changed.
//! 3. The index thread writes the results in batched transactions, so the
//!    library fills in while the scan runs and every committed batch survives
//!    a restart.
//!
//! A folder keeps its `scan_pending` flag until a scan reaches the end of it,
//! so quitting mid-scan resumes that folder first on the next launch.
//! Thumbnails decode on a separate pool of threads. Disk thumbnails for new
//! photos, and any others still missing, are queued once the scans finish,
//! so the two don't compete for the disk; the window asks for the thumbnails
//! it shows straight away, and one thread serves only those.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::db::{Database, Known, ScanRecord};
use crate::edit::{self, Geometry, Settings};
use crate::geotag::Sidecar;
use crate::i18n::tr;
use crate::image_io::{self, Probe};
use crate::photo_info::{self, Histogram};
use crate::picasa_ini::PicasaIni;
use crate::thumbs::ThumbStore;
use crate::track::{is_track_path, MapTrack, Track};
use crate::types::{
    folder_label, is_media_path, is_video_path, unix_now, NewPhoto, Photo, Unreadable,
};
use crate::video;
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Select, Sender, TryRecvError};
use image::{DynamicImage, RgbaImage};

pub type DisplayGate = Arc<Mutex<HashMap<i64, u64>>>;

/// Photos whose thumbnails the window drew or asked for in its latest frame.
/// Requests for anything else are dropped, so after a fast scroll the photos
/// on screen don't wait behind the ones it went past.
pub type ThumbWanted = Arc<Mutex<HashSet<i64>>>;

/// Records written per transaction while a scan runs.
const BATCH_SIZE: usize = 200;
/// Longest wait before probed records are committed and shown.
const BATCH_INTERVAL: Duration = Duration::from_millis(250);

pub enum Command {
    /// Reads every file of a folder that is new or has changed.
    Scan(i64),
    /// Like [`Command::Scan`], but a subfolder whose modification time is
    /// the same as at the last complete scan isn't listed again. Sent at
    /// launch, so a large folder on a slow or network drive is checked in
    /// a fraction of the time. A file changed in place, without the folder
    /// changing, is found by the next full scan.
    Check(i64),
    RefreshTrack {
        folder_id: i64,
        path: String,
    },
    /// A folder was excluded from scanning: a scan running in its added
    /// folder stops reading it at once and goes on with the rest.
    Excluded,
    /// Ask the index thread to finish. Sent when the window closes.
    Stop,
}

/// The folders excluded from scanning, shared by a scan's walk and probes,
/// and brought up to date while it runs by [`Command::Excluded`].
type Excluded = Arc<RwLock<Vec<PathBuf>>>;

/// Whether `path` is in or under a folder of `excluded`.
fn left_out(excluded: &Excluded, path: &Path) -> bool {
    excluded
        .read()
        .is_ok_and(|dirs| crate::db::under_any(path, &dirs))
}

pub enum ThumbJob {
    /// `edit` is applied over the decoded pixels; the disk cache keeps the original.
    /// `edge` is the short edge wanted: [`THUMB_EDGE`] is the cached size, and a
    /// larger one is decoded for a big tile and kept by the window alone.
    Thumb {
        id: i64,
        path: String,
        edit: Settings,
        edge: u32,
    },
    /// `edge` caps the long side of the decoded photo; videos use their poster.
    Display {
        id: i64,
        path: String,
        nonce: u64,
        edit: Settings,
        edge: u32,
    },
    /// Writes the disk thumbnail during a scan without sending pixels to the
    /// window, which asks for the ones it shows.
    Cache { id: i64, path: String },
    /// EXIF rows only, without decoding pixels.
    Exif { id: i64, path: String },
}

/// A read Trip package, the extracted photos by key, the library photos
/// that stand for others by key, and the folder of the extracted ones.
pub type Unpacked = (
    crate::trip_package::Package,
    std::collections::HashMap<i64, std::path::PathBuf>,
    std::collections::HashMap<i64, i64>,
    std::path::PathBuf,
);

pub enum UiEvent {
    ScanStarted(String),
    ScanProgress {
        folder: String,
        checked: usize,
        indexed: usize,
    },
    ScanFinished {
        folder: i64,
        unreadable: Vec<Unreadable>,
    },
    /// Photos that got their stars, captions, albums or edits back from the
    /// archive.
    Restored(Vec<i64>),
    Library,
    /// Photos whose files changed since they were last read, so what was
    /// worked out from their old pixels is out of date.
    Changed(Vec<i64>),
    Thumb {
        id: i64,
        w: usize,
        h: usize,
        rgba: Vec<u8>,
        edit: Settings,
    },
    ThumbFailed {
        id: i64,
    },
    /// Dropped because the photo left the screen before its turn.
    ThumbSkipped {
        id: i64,
    },
    /// A photo that couldn't be shown, and why, in words; `missing` when
    /// its file isn't there, as on a drive that may be plugged back in.
    DisplayFailed {
        id: i64,
        nonce: u64,
        reason: String,
        missing: bool,
    },
    Display {
        id: i64,
        nonce: u64,
        w: usize,
        h: usize,
        rgba: Vec<u8>,
        histogram: Box<Histogram>,
        exif: Vec<(&'static str, String)>,
        edit: Settings,
    },
    Exif {
        id: i64,
        rows: Vec<(&'static str, String)>,
    },
    /// Every GPS track in the catalog, sent at start and after a scan
    /// changes any.
    Tracks(Vec<MapTrack>),
    /// The photo decoded at editor preview size.
    EditBase {
        id: i64,
        image: Arc<RgbaImage>,
    },
    EditFailed {
        id: i64,
        error: String,
    },
    /// Preview with the edit applied, answering render request `seq`.
    EditPreview {
        id: i64,
        seq: u64,
        purpose: edit::Purpose,
        w: usize,
        h: usize,
        rgba: Vec<u8>,
        /// Tones of the edited preview, without overlays.
        histogram: Option<Box<Histogram>>,
    },
    Exported(Result<PathBuf, String>),
    /// A picture went on the clipboard, or why it couldn't.
    Copied(Result<(), String>),
    /// Progress of a batch export: counts so far, and the first error.
    BatchExported {
        exported: usize,
        failed: usize,
        error: Option<String>,
        finished: bool,
    },
    /// Looks of photos found for stacking look-alikes in the grid, `done`
    /// photos so far; the last batch is `finished`.
    Looks {
        run: u64,
        looks: Vec<(i64, u64)>,
        /// Photos whose look couldn't be found.
        no_look: Vec<i64>,
        /// Photos with no thumbnail yet, left until no scan is running.
        later: Vec<i64>,
        done: usize,
        finished: bool,
    },
    /// How far a search for similar photos is, and what it found once done.
    Similar {
        run: u64,
        done: usize,
        total: usize,
        found: Option<(
            Vec<crate::burst::Shot>,
            std::collections::HashMap<i64, Photo>,
        )>,
    },
    /// Progress of duplicate search `run`; `groups` once it is done.
    Duplicates {
        run: u64,
        checked: usize,
        total: usize,
        groups: Option<Vec<Vec<Photo>>>,
    },
    /// Days suggested for a Trip from its photos' capture times, or why not.
    DaysSuggested {
        trip_id: i64,
        outcome: Result<crate::trip::DaySuggestion, String>,
    },
    /// Places suggested for a Trip from its photos' positions and tracks, or why not.
    PlacesSuggested {
        trip_id: i64,
        outcome: Result<crate::trip::PlaceSuggestion, String>,
    },
    /// Names looked up for a Place being edited, or why not.
    PlaceNamed {
        place_id: i64,
        outcome: Result<crate::geocode::PlaceNames, String>,
    },
    /// A Story's preview was written and opened, or why not.
    StoryPreviewed(
        std::sync::Arc<crate::story_html::Progress>,
        Result<crate::story_html::Outcome, String>,
    ),
    /// Story `.0` was exported to a new folder by job `.1`, or why not, with
    /// the key of what the export was made from.
    StoryExported(
        i64,
        std::sync::Arc<crate::story_html::Progress>,
        String,
        Result<(std::path::PathBuf, crate::story_html::Outcome), String>,
    ),
    /// The key of what Story `.0` is made from now, for comparison `.1`.
    StoryContent(i64, u64, String),
    /// The elevation along route source `.1` of Story `.0`, read from its
    /// track files.
    RouteProfile(i64, String, Option<crate::elevation::Profile>),
    /// A Story's publishing learned something to keep at once, on site `.0`,
    /// for the job in row `.1` of the publishing log.
    PublishProgress(i64, Option<i64>, crate::publish::Event),
    /// A Story's draft was saved on a site, or why not.
    StoryPublished(Box<crate::publish::Finished>),
    /// Uploads a post stopped using were deleted from site `.0`, named `.1`,
    /// or as many as could be.
    UploadsDeleted(i64, String, crate::publish::Deleted),
    /// A Trip package was saved, with how many photos were unavailable, or why not.
    TripPackaged(Result<(std::path::PathBuf, usize), String>),
    /// A Trip package's photos were written to a folder, ready to import.
    TripUnpacked(Box<Result<Unpacked, String>>),
}

/// Number of threads that probe files during a scan. A probe mostly waits
/// on the disk or on ffprobe, so there are more of them than cores.
pub fn probe_threads() -> usize {
    (std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        * 2)
    .clamp(4, 16)
}

/// Number of threads that decode thumbnails and full-size views.
pub fn thumb_threads() -> usize {
    (std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        * 3
        / 4)
    .clamp(2, 8)
}

/// Short edge of a cached thumbnail. Tiles are squares cut from the middle;
/// one that is larger on screen gets a sharper copy decoded for it, which
/// stays out of the cache so the cache doesn't grow.
pub const THUMB_EDGE: u32 = 320;
/// Most the short edge of the photo is decoded to for a thumbnail whose
/// edit crops, turns, or tilts it.
const REFRAMED_SOURCE_EDGE: u32 = 1280;
/// JPEG quality of stored edited thumbnails. They are not made again from
/// the original, so they keep more detail than the plain ones.
const EDITED_THUMB_QUALITY: u8 = 90;
/// Long edge of the full-size view until the viewer zooms past it.
pub const DISPLAY_EDGE: u32 = 2560;

struct ProbeJob {
    path: PathBuf,
    path_text: String,
    filename: String,
    modified_at: i64,
    bytes: u64,
    existing: Option<i64>,
}

struct ProbeResult {
    job: ProbeJob,
    probe: Result<Probe, String>,
}

struct WalkSummary {
    seen: Vec<String>,
    /// Each subfolder walked, with its modification time in nanoseconds,
    /// or -1 when the next check has to list it again.
    stamps: Vec<(String, i64)>,
    unreadable: Vec<Unreadable>,
    complete: bool,
    /// Track files that are new or changed.
    tracks: Vec<TrackFile>,
    tracks_seen: Vec<String>,
}

struct TrackFile {
    path: String,
    bytes: u64,
    modified_at: i64,
}

enum Outcome {
    Finished,
    Stopped,
}

struct Indexer {
    db: Database,
    cache: PathBuf,
    thumb_store: ThumbStore,
    commands: Receiver<Command>,
    thumbs: Sender<ThumbJob>,
    events: Sender<UiEvent>,
    ctx: eframe::egui::Context,
    pending: VecDeque<i64>,
    /// Queued folders whose scan may skip unchanged subfolders: those only
    /// [`Command::Check`] asked for.
    quick: HashSet<i64>,
    /// Disk thumbnails for photos found by the scans in progress. Making
    /// them reads each whole file, which would compete with the scan for the
    /// disk, so they are queued once every pending folder is done. Photos on
    /// screen get theirs sooner because the window asks for them.
    deferred: Vec<ThumbJob>,
    /// Set once missing disk thumbnails have been queued this session.
    backfilled: bool,
    /// Size and modification time of the `.json` files found to be no
    /// track this session, so the scans don't open them again unchanged.
    not_tracks: RefCell<HashMap<String, (u64, i64)>>,
}

pub fn index_loop(
    db_path: PathBuf,
    cache: PathBuf,
    commands: Receiver<Command>,
    thumbs: Sender<ThumbJob>,
    events: Sender<UiEvent>,
    ctx: eframe::egui::Context,
) {
    let db = match Database::open(&db_path) {
        Ok(db) => db,
        Err(err) => {
            let _ = events.send(UiEvent::ScanFinished {
                folder: 0,
                unreadable: Vec::new(),
            });
            eprintln!("picasso database: {err}");
            return;
        }
    };
    // Folders whose last scan did not reach the end go first.
    let pending = db.pending_folders().into_iter().collect();
    let mut indexer = Indexer {
        db,
        thumb_store: ThumbStore::open(&cache),
        cache,
        commands,
        thumbs,
        events,
        ctx,
        pending,
        quick: HashSet::new(),
        deferred: Vec::new(),
        backfilled: false,
        not_tracks: RefCell::new(HashMap::new()),
    };
    indexer.send_tracks();
    indexer.run();
}

impl Indexer {
    fn run(&mut self) {
        loop {
            if let Some(folder_id) = self.pending.pop_front() {
                match self.scan_folder(folder_id) {
                    Outcome::Finished => continue,
                    Outcome::Stopped => return,
                }
            }
            for job in self.deferred.drain(..) {
                let _ = self.thumbs.send(job);
            }
            if !self.backfilled {
                self.backfilled = true;
                self.backfill_thumbs();
            }
            match self.commands.recv() {
                Ok(Command::Scan(id)) => self.queue(id),
                Ok(Command::Check(id)) => self.queue_check(id),
                Ok(Command::RefreshTrack { folder_id, path }) => {
                    self.refresh_track(folder_id, path)
                }
                // No scan is running; the next one reads the list itself.
                Ok(Command::Excluded) => {}
                Ok(Command::Stop) | Err(_) => return,
            }
        }
    }

    /// Queues disk thumbnails for every photo that has none, newest first, so
    /// scrolling finds them ready. This also picks up the ones a previous
    /// session queued but closed before making, and drops the thumbnails of
    /// photos no longer in the library. The thumbnail store is read on a
    /// separate thread so the index thread can take commands meanwhile.
    fn backfill_thumbs(&self) {
        let photos = self.db.photo_paths();
        let cache = self.cache.clone();
        let thumbs = self.thumbs.clone();
        let spawned = std::thread::Builder::new()
            .name("picasso-backfill".into())
            .spawn(move || {
                let store = ThumbStore::open(&cache);
                store.retain(&photos.iter().map(|(id, _)| *id).collect());
                let cached = store.ids();
                for (id, path) in photos {
                    if !cached.contains(&id) && thumbs.send(ThumbJob::Cache { id, path }).is_err() {
                        return;
                    }
                }
            });
        if let Err(err) = spawned {
            eprintln!("picasso backfill: {err}");
        }
    }

    fn queue(&mut self, id: i64) {
        self.quick.remove(&id);
        if !self.pending.contains(&id) {
            self.pending.push_back(id);
        }
    }

    /// Queues a quick check of `id`, unless a full scan of it is queued.
    fn queue_check(&mut self, id: i64) {
        if !self.pending.contains(&id) {
            self.quick.insert(id);
            self.pending.push_back(id);
        }
    }

    fn notify(&self, event: UiEvent) {
        let _ = self.events.send(event);
        self.ctx.request_repaint();
    }

    fn send_tracks(&self) {
        let tracks = self
            .db
            .tracks()
            .into_iter()
            .map(|(id, folder_id, path, track)| MapTrack::new(id, folder_id, path, &track))
            .collect();
        self.notify(UiEvent::Tracks(tracks));
    }

    fn refresh_track(&self, folder_id: i64, path: String) {
        let Ok(meta) = std::fs::metadata(&path) else {
            return;
        };
        let file = TrackFile {
            path,
            bytes: meta.len(),
            modified_at: image_io::file_mtime(&meta),
        };
        if self.read_tracks(folder_id, &[file], &mut Vec::new()) {
            self.send_tracks();
        }
    }

    /// Reads the new and changed track files a walk found. Returns true if
    /// the stored tracks changed.
    fn read_tracks(
        &self,
        folder_id: i64,
        files: &[TrackFile],
        unreadable: &mut Vec<Unreadable>,
    ) -> bool {
        let mut changed = false;
        for file in files {
            let Some(read) = Track::read_scanned(Path::new(&file.path)) else {
                changed |= self.db.forget_track(&file.path);
                self.not_tracks
                    .borrow_mut()
                    .insert(file.path.clone(), (file.bytes, file.modified_at));
                continue;
            };
            self.not_tracks.borrow_mut().remove(&file.path);
            match read {
                Ok(track) => match self.db.store_track(
                    folder_id,
                    &file.path,
                    file.bytes,
                    file.modified_at,
                    &track,
                ) {
                    Ok(()) => changed = true,
                    Err(err) => eprintln!("picasso track: {err}"),
                },
                Err(reason) => {
                    changed |= self.db.forget_track(&file.path);
                    unreadable.push(Unreadable {
                        path: file.path.clone(),
                        reason,
                        folder: false,
                        bytes: file.bytes,
                        modified_at: file.modified_at,
                    });
                }
            }
        }
        changed
    }

    fn scan_folder(&mut self, folder_id: i64) -> Outcome {
        let quick = self.quick.remove(&folder_id);
        let Some(path) = self.db.folder_path(folder_id) else {
            return Outcome::Finished;
        };
        let root = PathBuf::from(&path);
        if !root.is_dir() {
            // An unplugged drive keeps its photos and its pending flag. A
            // folder renamed in place is followed; one deleted or moved away
            // leaves the library, and its user data waits in the archive
            // for the files to turn up again.
            if !crate::db::folder_gone(&root) {
                return Outcome::Finished;
            }
            if let Some(renamed) = self.renamed_to(folder_id, &root) {
                if self.db.move_folder(folder_id, &renamed).is_ok() {
                    self.notify(UiEvent::Library);
                    return self.scan_folder(folder_id);
                }
            }
            if let Err(err) = self.db.remove_folder(folder_id) {
                eprintln!("picasso index: {err}");
                return Outcome::Finished;
            }
            self.db.relink_trip_media();
            self.send_tracks();
            self.notify(UiEvent::Library);
            return Outcome::Finished;
        }
        let label = folder_label(&path);
        self.db.set_scan_pending(folder_id, true);
        self.notify(UiEvent::ScanStarted(label.clone()));

        let snapshot = self.db.folder_snapshot(folder_id, &path);
        let mut known_tracks = self.db.track_snapshot(folder_id, &path);
        known_tracks.extend(
            self.not_tracks
                .borrow()
                .iter()
                .map(|(path, stamp)| (path.clone(), *stamp)),
        );
        let remembered = self.db.unreadable_snapshot(folder_id);
        // Kept only where the file system updates folder times.
        let times_kept = crate::files::folder_times_kept(&root);
        let stamps = if quick && times_kept {
            self.db.dir_stamps(folder_id)
        } else {
            HashMap::new()
        };
        let remembered_paths: HashSet<String> = remembered.keys().cloned().collect();
        let excluded: Excluded = Arc::new(RwLock::new(self.excluded_dirs()));
        let cancel = Arc::new(AtomicBool::new(false));
        let checked = Arc::new(AtomicUsize::new(0));
        // The walk's files pass through here on their way to the probes, so
        // a renamed or moved file takes its new path, and the folder tree
        // follows, as soon as it is listed, while the probes read the rest.
        let (list_tx, list_rx) = crossbeam_channel::unbounded::<ProbeJob>();
        let (job_tx, job_rx) = crossbeam_channel::unbounded::<ProbeJob>();
        let (done_tx, done_rx) = bounded::<ProbeResult>(512);

        let walker = {
            let cancel = cancel.clone();
            let checked = checked.clone();
            let excluded = excluded.clone();
            std::thread::Builder::new()
                .name("picasso-walk".into())
                .spawn(move || {
                    walk_folder(
                        &root,
                        snapshot,
                        known_tracks,
                        remembered,
                        stamps,
                        excluded,
                        list_tx,
                        &cancel,
                        &checked,
                    )
                })
        };
        let walker = match walker {
            Ok(handle) => handle,
            Err(err) => {
                eprintln!("picasso index: {err}");
                // The window counts the scans running, so it hears this one
                // end too; the files that failed before are still listed.
                let mut unreadable: Vec<Unreadable> = self
                    .db
                    .unreadable_snapshot(folder_id)
                    .into_values()
                    .collect();
                unreadable.sort_by(|a, b| a.path.cmp(&b.path));
                self.notify(UiEvent::ScanFinished {
                    folder: folder_id,
                    unreadable,
                });
                return Outcome::Finished;
            }
        };
        for index in 0..probe_threads() {
            let jobs = job_rx.clone();
            let done = done_tx.clone();
            let cancel = cancel.clone();
            let excluded = excluded.clone();
            let _ = std::thread::Builder::new()
                .name(format!("picasso-probe-{index}"))
                .spawn(move || probe_loop(jobs, done, &cancel, &excluded));
        }
        drop(job_rx);
        drop(done_tx);

        let mut ini_cache: HashMap<PathBuf, (PicasaIni, Sidecar)> = HashMap::new();
        let mut claimed: HashSet<i64> = HashSet::new();
        let mut batch: Vec<ScanRecord> = Vec::with_capacity(BATCH_SIZE);
        let mut moves = Vec::new();
        let mut unreadable = Vec::new();
        let mut indexed = 0usize;
        let mut again = false;
        let mut last_flush = Instant::now();
        let mut stopped = false;
        let mut walker = Some(walker);
        let mut job_tx = Some(job_tx);
        let mut summary: Option<WalkSummary> = None;

        loop {
            loop {
                match self.commands.try_recv() {
                    Ok(Command::Scan(id)) if id == folder_id => again = true,
                    Ok(Command::Scan(id)) => self.queue(id),
                    // This scan reads everything a check would.
                    Ok(Command::Check(id)) if id == folder_id => {}
                    Ok(Command::Check(id)) => self.queue_check(id),
                    Ok(Command::RefreshTrack { folder_id, path }) => {
                        self.refresh_track(folder_id, path)
                    }
                    Ok(Command::Excluded) => {
                        if let Ok(mut dirs) = excluded.write() {
                            *dirs = self.excluded_dirs();
                        }
                    }
                    Ok(Command::Stop) | Err(TryRecvError::Disconnected) => {
                        stopped = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
            if stopped {
                break;
            }
            let mut select = Select::new();
            let listing = walker.as_ref().map(|_| select.recv(&list_rx));
            let probed = select.recv(&done_rx);
            let mut finished = false;
            match select.select_timeout(BATCH_INTERVAL) {
                Ok(op) if Some(op.index()) == listing => match op.recv(&list_rx) {
                    // Listed before its folder was excluded: not read.
                    Ok(job) if left_out(&excluded, &job.path) => {}
                    Ok(job) => {
                        let moved = if job.existing.is_none() {
                            self.db
                                .moved_photo(&job.path, job.bytes, job.modified_at, &claimed)
                        } else {
                            None
                        };
                        match moved {
                            Some(id) => {
                                claimed.insert(id);
                                moves.push((id, job.path_text, job.filename, folder_id));
                            }
                            None => {
                                if let Some(tx) = &job_tx {
                                    let _ = tx.send(job);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        // The listing is done: the probes get no more files,
                        // and deleted files leave the library right away.
                        job_tx = None;
                        let walked = walker
                            .take()
                            .map(|walker| join_walk(walker, &path))
                            .unwrap_or_else(|| failed_walk(&path));
                        self.apply_moves(&mut moves);
                        if walked.complete && self.db.remove_missing(folder_id, &walked.seen) > 0 {
                            self.notify(UiEvent::Library);
                        }
                        summary = Some(walked);
                    }
                },
                Ok(op) if op.index() == probed => match op.recv(&done_rx) {
                    Ok(result) if left_out(&excluded, &result.job.path) => {}
                    Ok(result) => {
                        match self.record(folder_id, result, &mut ini_cache, &mut claimed) {
                            Ok(record) => batch.push(record),
                            Err(item) => unreadable.push(item),
                        }
                    }
                    Err(_) => finished = true,
                },
                _ => {}
            }
            let due = batch.len() >= BATCH_SIZE || last_flush.elapsed() >= BATCH_INTERVAL;
            if finished || due {
                self.apply_moves(&mut moves);
                indexed += self.flush(&mut batch);
                last_flush = Instant::now();
                self.notify(UiEvent::ScanProgress {
                    folder: label.clone(),
                    checked: checked.load(Ordering::Relaxed),
                    indexed,
                });
            }
            if finished {
                break;
            }
        }

        if stopped {
            // Keep what was probed so far; the folder stays pending and the
            // next launch continues from the files that are still missing.
            cancel.store(true, Ordering::Relaxed);
            drop(job_tx);
            drop(done_rx);
            self.apply_moves(&mut moves);
            self.flush(&mut batch);
            return Outcome::Stopped;
        }

        let summary = summary.unwrap_or_else(|| {
            walker
                .map(|walker| join_walk(walker, &path))
                .unwrap_or_else(|| failed_walk(&path))
        });
        // Folders excluded while the scan ran keep nothing it found there.
        if let Ok(mut dirs) = excluded.write() {
            *dirs = self.excluded_dirs();
        }
        let tracks: Vec<TrackFile> = summary
            .tracks
            .into_iter()
            .filter(|file| !left_out(&excluded, Path::new(&file.path)))
            .collect();
        let mut tracks_changed = self.read_tracks(folder_id, &tracks, &mut unreadable);
        unreadable.extend(summary.unreadable);
        unreadable.retain(|item| !left_out(&excluded, Path::new(&item.path)));
        unreadable.sort_by(|a, b| a.path.cmp(&b.path));
        self.db
            .store_unreadable(folder_id, &unreadable, summary.complete, &remembered_paths);
        if summary.complete {
            let stamps = if times_kept {
                listable(summary.stamps, &unreadable, &excluded)
            } else {
                Vec::new()
            };
            self.db.store_dir_stamps(folder_id, &stamps);
            self.db.remove_missing(folder_id, &summary.seen);
            tracks_changed |= self
                .db
                .remove_missing_tracks(folder_id, &summary.tracks_seen)
                > 0;
            self.db.set_scan_pending(folder_id, false);
        }
        if tracks_changed {
            self.send_tracks();
        }
        let restored = self.db.adopt_archived();
        self.db.relink_trip_media();
        if !restored.is_empty() {
            self.notify(UiEvent::Restored(restored));
        }
        self.notify(UiEvent::ScanFinished {
            folder: folder_id,
            unreadable,
        });
        self.notify(UiEvent::Library);
        if again {
            self.pending.push_front(folder_id);
        }
        Outcome::Finished
    }

    /// The folders excluded from scanning, as the library has them now.
    fn excluded_dirs(&self) -> Vec<PathBuf> {
        self.db
            .excluded_dirs()
            .into_iter()
            .map(|(dir, _)| PathBuf::from(dir))
            .collect()
    }

    /// Gives the photos of renamed and moved files their new paths. A file
    /// moved into an excluded folder isn't followed there.
    fn apply_moves(&self, moves: &mut Vec<(i64, String, String, i64)>) {
        if moves.is_empty() {
            return;
        }
        let excluded = self.excluded_dirs();
        moves.retain(|(_, path, ..)| !crate::db::under_any(Path::new(path), &excluded));
        if moves.is_empty() {
            return;
        }
        match self.db.move_photos(moves) {
            Ok(()) => self.notify(UiEvent::Library),
            Err(err) => eprintln!("picasso index: {err}"),
        }
        moves.clear();
    }

    /// The folder beside the missing `root` that holds its photos, if
    /// exactly one does: the name `root` was renamed to.
    fn renamed_to(&self, folder_id: i64, root: &Path) -> Option<PathBuf> {
        let sample = self.db.folder_sample(folder_id, 20);
        let parent = root.parent()?;
        let added: HashSet<PathBuf> = self
            .db
            .folders()
            .into_iter()
            .map(|folder| PathBuf::from(folder.path))
            .collect();
        let relative: Vec<(PathBuf, u64)> = sample
            .iter()
            .filter_map(|(path, bytes)| {
                Some((
                    Path::new(path).strip_prefix(root).ok()?.to_path_buf(),
                    *bytes,
                ))
            })
            .collect();
        if relative.is_empty() {
            return None;
        }
        let mut found = std::fs::read_dir(parent)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|dir| dir.is_dir() && !added.contains(dir))
            .filter(|dir| {
                relative.iter().all(|(rel, bytes)| {
                    std::fs::metadata(dir.join(rel)).is_ok_and(|meta| meta.len() == *bytes)
                })
            });
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    fn record(
        &self,
        folder_id: i64,
        result: ProbeResult,
        ini_cache: &mut HashMap<PathBuf, (PicasaIni, Sidecar)>,
        claimed: &mut HashSet<i64>,
    ) -> Result<ScanRecord, Unreadable> {
        let ProbeResult { mut job, probe } = result;
        // A renamed or moved file keeps the row it had.
        if job.existing.is_none() {
            job.existing = self
                .db
                .moved_photo(&job.path, job.bytes, job.modified_at, claimed);
            claimed.extend(job.existing);
        }
        let probe = probe.map_err(|reason| Unreadable {
            path: job.path_text.clone(),
            reason,
            folder: false,
            bytes: job.bytes,
            modified_at: job.modified_at,
        })?;
        let (star, caption, rotation, pin) = if job.existing.is_none() {
            let parent = job.path.parent().unwrap_or(&job.path).to_path_buf();
            let (ini, geotags) = ini_cache
                .entry(parent.clone())
                .or_insert_with(|| (PicasaIni::load(&parent), Sidecar::load(&parent)));
            let (star, caption, rotation) = ini
                .get(&job.filename)
                .map(|meta| (meta.star, meta.caption.clone(), meta.rotation))
                .unwrap_or((None, None, None));
            let pin = if probe.gps_lat.is_none() {
                geotags.place(&job.filename)
            } else {
                None
            };
            (star, caption, rotation, pin)
        } else {
            (None, None, None, None)
        };
        Ok(ScanRecord {
            photo: NewPhoto {
                folder_id,
                path: job.path_text,
                filename: job.filename,
                width: probe.width,
                height: probe.height,
                bytes: job.bytes,
                taken_at: if probe.taken_at == 0 {
                    unix_now()
                } else {
                    probe.taken_at
                },
                modified_at: job.modified_at,
                camera: probe.camera,
                lens: probe.lens,
                focal_mm: probe.focal_mm,
                gps_lat: probe.gps_lat,
                gps_lon: probe.gps_lon,
                duration: probe.duration,
            },
            existing: job.existing,
            star,
            caption,
            rotation,
            pin,
            complete: probe.complete,
        })
    }

    /// Commits the batch and queues background thumbnails for the written rows.
    fn flush(&mut self, batch: &mut Vec<ScanRecord>) -> usize {
        if batch.is_empty() {
            return 0;
        }
        let written = match self.db.write_batch(batch) {
            Ok(result) => result,
            Err(err) => {
                // The files are fine and are probed again on the next scan.
                eprintln!("picasso index: {err}");
                batch.clear();
                return 0;
            }
        };
        let mut count = 0;
        let mut changed = Vec::new();
        for (record, id) in batch.iter().zip(&written.ids) {
            // Left out: its folder was excluded while it was being read.
            if *id == 0 {
                continue;
            }
            count += 1;
            let (id, path) = (*id, record.photo.path.clone());
            // A changed file's old thumbnail may be on screen, so it is sent
            // again; new photos only need their disk thumbnail.
            let job = if record.existing.is_some() {
                self.thumb_store.remove(id);
                changed.push(id);
                ThumbJob::Thumb {
                    id,
                    path,
                    edit: Settings::default(),
                    edge: THUMB_EDGE,
                }
            } else {
                ThumbJob::Cache { id, path }
            };
            if matches!(job, ThumbJob::Cache { .. }) {
                self.deferred.push(job);
            } else {
                let _ = self.thumbs.send(job);
            }
        }
        batch.clear();
        if !changed.is_empty() {
            self.notify(UiEvent::Changed(changed));
        }
        self.notify(UiEvent::Library);
        count
    }
}

fn join_walk(walker: std::thread::JoinHandle<WalkSummary>, path: &str) -> WalkSummary {
    walker.join().unwrap_or_else(|_| failed_walk(path))
}

/// The summary of a walk that broke off: nothing may be taken as deleted.
fn failed_walk(path: &str) -> WalkSummary {
    WalkSummary {
        seen: Vec::new(),
        unreadable: vec![Unreadable {
            path: path.to_string(),
            reason: tr("The scan stopped unexpectedly").to_string(),
            folder: true,
            bytes: 0,
            modified_at: 0,
        }],
        complete: false,
        tracks: Vec::new(),
        tracks_seen: Vec::new(),
        stamps: Vec::new(),
    }
}

/// `stamps` as they are stored, with -1 for each subfolder that holds a file
/// or folder that couldn't be read, so the next check lists it again and
/// tries those once more. Those excluded during the scan are dropped.
fn listable(
    stamps: Vec<(String, i64)>,
    unreadable: &[Unreadable],
    excluded: &Excluded,
) -> Vec<(String, i64)> {
    let mut failed: HashSet<&Path> = HashSet::new();
    for item in unreadable {
        let path = Path::new(&item.path);
        failed.insert(path);
        if let Some(parent) = path.parent() {
            failed.insert(parent);
        }
    }
    stamps
        .into_iter()
        .filter(|(dir, _)| !left_out(excluded, Path::new(dir)))
        .map(|(dir, stamp)| {
            let stamp = if failed.contains(Path::new(&dir)) {
                -1
            } else {
                stamp
            };
            (dir, stamp)
        })
        .collect()
}

/// A folder's modification time in nanoseconds since 1970, when it is on
/// `device`, the drive of the added folder whose file system was checked.
/// A drive mounted below it, such as a memory card, may not keep its
/// folder times, so its folders are listed every time.
fn dir_stamp(dir: &Path, device: Option<u64>) -> Option<i64> {
    let meta = std::fs::metadata(dir).ok()?;
    if device_of(&meta) != device {
        return None;
    }
    let modified = meta.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since.as_nanos()).ok()
}

/// The drive a file is on. Windows walks into no other drive, as a drive
/// mounted in a folder is listed as a link, so it needs none.
#[cfg(unix)]
fn device_of(meta: &std::fs::Metadata) -> Option<u64> {
    Some(std::os::unix::fs::MetadataExt::dev(meta))
}

#[cfg(not(unix))]
fn device_of(_meta: &std::fs::Metadata) -> Option<u64> {
    None
}

/// A folder changed this close to the walk may change again within the
/// same tick of a coarse clock, as on a FAT drive, without its time moving.
const STAMP_MARGIN: Duration = Duration::from_secs(3);

/// What the last complete scan knew of each subfolder, for a quick check.
#[derive(Default)]
struct Stamps {
    /// Modification time of each subfolder walked, -1 to list it again.
    time: HashMap<PathBuf, i64>,
    /// The subfolders walked inside each one.
    children: HashMap<PathBuf, Vec<PathBuf>>,
    /// Photos and tracks known in each subfolder, not below it.
    files: HashMap<PathBuf, Vec<String>>,
    tracks: HashMap<PathBuf, Vec<String>>,
    /// Subfolders with a photo an older build read, to be read again.
    stale: HashSet<PathBuf>,
}

impl Stamps {
    fn new(
        stored: HashMap<String, i64>,
        snapshot: &HashMap<String, Known>,
        known_tracks: &HashMap<String, (u64, i64)>,
    ) -> Stamps {
        if stored.is_empty() {
            return Stamps::default();
        }
        let mut stamps = Stamps::default();
        for (dir, stamp) in stored {
            let dir = PathBuf::from(dir);
            if let Some(parent) = dir.parent() {
                stamps
                    .children
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(dir.clone());
            }
            stamps.time.insert(dir, stamp);
        }
        for (path, known) in snapshot {
            let Some(parent) = Path::new(path).parent() else {
                continue;
            };
            if known.version < crate::db::INDEX_VERSION {
                stamps.stale.insert(parent.to_path_buf());
            }
            stamps
                .files
                .entry(parent.to_path_buf())
                .or_default()
                .push(path.clone());
        }
        for path in known_tracks.keys() {
            if let Some(parent) = Path::new(path).parent() {
                stamps
                    .tracks
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(path.clone());
            }
        }
        stamps
    }

    /// Whether `dir` is as the last complete scan left it.
    fn unchanged(&self, dir: &Path, stamp: i64) -> bool {
        stamp >= 0 && self.time.get(dir) == Some(&stamp) && !self.stale.contains(dir)
    }
}

/// Shared state of one folder walk.
struct Walk<'a> {
    /// The drive of the added folder; see [`dir_stamp`].
    device: Option<u64>,
    /// The last complete scan's subfolders; empty unless this is a check.
    stamps: Stamps,
    /// A subfolder modified after this, in nanoseconds, is listed again
    /// next time.
    fresh_after: i64,
    /// Each subfolder walked, with the time stored for it.
    found: Mutex<Vec<(String, i64)>>,
    snapshot: HashMap<String, Known>,
    /// Size and modification time of the stored tracks, by path.
    known_tracks: HashMap<String, (u64, i64)>,
    tracks: Mutex<Vec<TrackFile>>,
    tracks_seen: Mutex<Vec<String>>,
    /// Files that failed before, skipped while they are unchanged.
    remembered: HashMap<String, Unreadable>,
    /// Folders the user left out, skipped with everything below them, as
    /// they are now: one excluded while the walk runs stops being read.
    excluded: Excluded,
    jobs: Sender<ProbeJob>,
    cancel: &'a AtomicBool,
    checked: &'a AtomicUsize,
    seen: Mutex<Vec<String>>,
    /// Cleared when the root could not be listed or the walk stopped early,
    /// so missing photos are not removed on the strength of a partial listing.
    complete: AtomicBool,
    /// Folders and files that could not be read. The photos known under
    /// them count as seen, so a drive that drops out for a moment doesn't
    /// remove them from the library.
    unreadable: Mutex<Vec<Unreadable>>,
}

impl Walk<'_> {
    /// Takes `dir`, unchanged since the last complete scan, to hold the
    /// photos and tracks known there, and walks the subfolders it had.
    fn skip<'s>(&'s self, scope: &rayon::Scope<'s>, dir: PathBuf, stamp: i64) {
        let files = self.stamps.files.get(&dir).cloned().unwrap_or_default();
        self.checked.fetch_add(files.len(), Ordering::Relaxed);
        if let Ok(mut seen) = self.seen.lock() {
            seen.extend(files);
        }
        if let Some(tracks) = self.stamps.tracks.get(&dir) {
            if let Ok(mut seen) = self.tracks_seen.lock() {
                seen.extend(tracks.iter().cloned());
            }
        }
        for child in self.stamps.children.get(&dir).into_iter().flatten() {
            let child = child.clone();
            scope.spawn(move |scope| walk_dir(scope, child, false, self));
        }
        if let Ok(mut found) = self.found.lock() {
            found.push((dir.to_string_lossy().into_owned(), stamp));
        }
    }

    fn unreadable(&self, path: &Path, reason: impl std::fmt::Display, folder: bool) {
        self.push(Unreadable {
            path: path.to_string_lossy().into_owned(),
            reason: reason.to_string(),
            folder,
            bytes: 0,
            modified_at: 0,
        });
    }

    fn push(&self, item: Unreadable) {
        if let Ok(mut list) = self.unreadable.lock() {
            list.push(item);
        }
    }
}

/// Known paths at or under any of `unreadable`.
fn known_under<'a>(known: impl Iterator<Item = &'a String>, unreadable: &[PathBuf]) -> Vec<String> {
    if unreadable.is_empty() {
        return Vec::new();
    }
    let prefixes: Vec<(String, String)> = unreadable
        .iter()
        .map(|path| {
            let exact = path.to_string_lossy().into_owned();
            let mut under = exact
                .trim_end_matches(std::path::MAIN_SEPARATOR)
                .to_string();
            under.push(std::path::MAIN_SEPARATOR);
            (exact, under)
        })
        .collect();
    known
        .filter(|path| {
            prefixes
                .iter()
                .any(|(exact, under)| *path == exact || path.starts_with(under.as_str()))
        })
        .cloned()
        .collect()
}

/// Lists `root` with one task per directory on a small private pool, so
/// directories on slow or network drives are read in parallel. Size and
/// modification time come from the directory listing itself, which on
/// Windows saves opening every file.
#[allow(clippy::too_many_arguments)]
fn walk_folder(
    root: &Path,
    snapshot: HashMap<String, Known>,
    known_tracks: HashMap<String, (u64, i64)>,
    remembered: HashMap<String, Unreadable>,
    stamps: HashMap<String, i64>,
    excluded: Excluded,
    jobs: Sender<ProbeJob>,
    cancel: &AtomicBool,
    checked: &AtomicUsize,
) -> WalkSummary {
    let fresh_after = std::time::SystemTime::now()
        .checked_sub(STAMP_MARGIN)
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_nanos()).ok())
        .unwrap_or(0);
    let walk = Walk {
        device: std::fs::metadata(root)
            .ok()
            .and_then(|meta| device_of(&meta)),
        stamps: Stamps::new(stamps, &snapshot, &known_tracks),
        fresh_after,
        found: Mutex::new(Vec::new()),
        seen: Mutex::new(Vec::with_capacity(snapshot.len())),
        snapshot,
        known_tracks,
        tracks: Mutex::new(Vec::new()),
        tracks_seen: Mutex::new(Vec::new()),
        remembered,
        excluded,
        jobs,
        cancel,
        checked,
        complete: AtomicBool::new(true),
        unreadable: Mutex::new(Vec::new()),
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(walk_threads())
        .thread_name(|index| format!("picasso-walk-{index}"))
        .build();
    match pool {
        Ok(pool) => pool.scope(|scope| walk_dir(scope, root.to_path_buf(), true, &walk)),
        Err(_) => rayon::scope(|scope| walk_dir(scope, root.to_path_buf(), true, &walk)),
    }
    let complete = walk.complete.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed);
    let unreadable = walk.unreadable.into_inner().unwrap_or_default();
    let mut seen = walk.seen.into_inner().unwrap_or_default();
    let paths: Vec<PathBuf> = unreadable
        .iter()
        .map(|item| PathBuf::from(&item.path))
        .collect();
    seen.extend(known_under(walk.snapshot.keys(), &paths));
    let mut tracks_seen = walk.tracks_seen.into_inner().unwrap_or_default();
    tracks_seen.extend(known_under(walk.known_tracks.keys(), &paths));
    WalkSummary {
        seen,
        unreadable,
        complete,
        tracks: walk.tracks.into_inner().unwrap_or_default(),
        tracks_seen,
        stamps: walk.found.into_inner().unwrap_or_default(),
    }
}

fn walk_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 4)
}

fn walk_dir<'s>(scope: &rayon::Scope<'s>, dir: PathBuf, root: bool, walk: &'s Walk<'s>) {
    // A folder excluded after it was queued isn't read.
    if walk.cancel.load(Ordering::Relaxed) || left_out(&walk.excluded, &dir) {
        return;
    }
    let stamp = dir_stamp(&dir, walk.device);
    if let Some(stamp) = stamp.filter(|&stamp| walk.stamps.unchanged(&dir, stamp)) {
        walk.skip(scope, dir, stamp);
        return;
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) => {
            if root {
                walk.complete.store(false, Ordering::Relaxed);
            }
            walk.unreadable(&dir, err, true);
            return;
        }
    };
    let mut seen = Vec::new();
    let mut jobs = Vec::new();
    let mut tracks_seen = Vec::new();
    let mut tracks = Vec::new();
    let mut listed = true;
    for entry in entries {
        let Ok(entry) = entry else {
            // The listing broke off, so anything known here may be unlisted.
            listed = false;
            continue;
        };
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                walk.unreadable(&path, err, false);
                continue;
            }
        };
        if file_type.is_dir() {
            if !left_out(&walk.excluded, &path) {
                scope.spawn(move |scope| walk_dir(scope, path, false, walk));
            }
            continue;
        }
        let track = is_track_path(&path);
        if !track && !is_media_path(&path) {
            continue;
        }
        // A link's own entry describes the link, so follow it to the file.
        let meta = if file_type.is_symlink() {
            std::fs::metadata(&path)
        } else {
            entry.metadata()
        };
        let meta = match meta {
            Ok(meta) if meta.is_file() => meta,
            Ok(_) => continue,
            Err(err) => {
                walk.unreadable(&path, err, false);
                continue;
            }
        };
        walk.checked.fetch_add(1, Ordering::Relaxed);
        let path_text = path.to_string_lossy().into_owned();
        let modified_at = image_io::file_mtime(&meta);
        let bytes = meta.len();
        let known = if track {
            None
        } else {
            walk.snapshot.get(&path_text).copied()
        };
        let current = if track {
            tracks_seen.push(path_text.clone());
            walk.known_tracks.get(&path_text) == Some(&(bytes, modified_at))
        } else {
            seen.push(path_text.clone());
            known.is_some_and(|known| known.current(modified_at, bytes))
        };
        if current {
            continue;
        }
        let failed = walk.remembered.get(&path_text);
        if let Some(item) =
            failed.filter(|item| item.bytes == bytes && item.modified_at == modified_at)
        {
            walk.push(item.clone());
            continue;
        }
        if track {
            tracks.push(TrackFile {
                path: path_text,
                bytes,
                modified_at,
            });
            continue;
        }
        jobs.push(ProbeJob {
            filename: entry.file_name().to_string_lossy().into_owned(),
            path,
            path_text,
            modified_at,
            bytes,
            existing: known.map(|known| known.id),
        });
    }
    // Excluded while it was being listed: what was found stays out.
    if left_out(&walk.excluded, &dir) {
        return;
    }
    if !listed {
        if root {
            walk.complete.store(false, Ordering::Relaxed);
        }
        walk.unreadable(&dir, tr("The folder could only be listed in part"), true);
    }
    let stamp = stamp
        .filter(|&stamp| listed && stamp < walk.fresh_after)
        .unwrap_or(-1);
    if let Ok(mut found) = walk.found.lock() {
        found.push((dir.to_string_lossy().into_owned(), stamp));
    }
    if let Ok(mut all) = walk.seen.lock() {
        all.extend(seen);
    }
    if let Ok(mut all) = walk.tracks_seen.lock() {
        all.extend(tracks_seen);
    }
    if let Ok(mut all) = walk.tracks.lock() {
        all.extend(tracks);
    }
    for job in jobs {
        if walk.cancel.load(Ordering::Relaxed) || walk.jobs.send(job).is_err() {
            walk.complete.store(false, Ordering::Relaxed);
            return;
        }
    }
}

fn probe_loop(
    jobs: Receiver<ProbeJob>,
    done: Sender<ProbeResult>,
    cancel: &AtomicBool,
    excluded: &Excluded,
) {
    for job in jobs.iter() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        // Waiting when its folder was excluded: not opened.
        if left_out(excluded, &job.path) {
            continue;
        }
        let probe = std::panic::catch_unwind(|| image_io::probe(&job.path, job.modified_at))
            .unwrap_or_else(|_| Err("decoder fault".into()));
        if done.send(ProbeResult { job, probe }).is_err() {
            return;
        }
    }
}

/// Background thumbnails read whole photos, so at most this many run at
/// once; on a hard disk more readers only seek against each other.
const BACKGROUND_SLOTS: usize = 2;
/// Background thumbnails wait this long after the window's last request, so
/// while scrolling the disk serves what is on screen.
const WINDOW_QUIET: Duration = Duration::from_millis(1500);

static BACKGROUND_RUNNING: AtomicUsize = AtomicUsize::new(0);
static WINDOW_LAST: Mutex<Option<Instant>> = Mutex::new(None);

/// Claims a slot for one background job, unless the window asked for
/// something lately or every slot is taken.
fn take_background_slot() -> bool {
    let quiet = WINDOW_LAST.lock().map_or(true, |last| {
        last.is_none_or(|at| at.elapsed() >= WINDOW_QUIET)
    });
    if !quiet {
        return false;
    }
    if BACKGROUND_RUNNING.fetch_add(1, Ordering::SeqCst) < BACKGROUND_SLOTS {
        true
    } else {
        BACKGROUND_RUNNING.fetch_sub(1, Ordering::SeqCst);
        false
    }
}

/// Runs thumbnail and display jobs. `normal` is `None` for a thread kept for
/// the window's requests, so they never wait behind background work.
pub fn thumb_loop(
    urgent: Receiver<ThumbJob>,
    normal: Option<Receiver<ThumbJob>>,
    events: Sender<UiEvent>,
    ctx: eframe::egui::Context,
    cache: PathBuf,
    gate: DisplayGate,
    wanted: ThumbWanted,
) {
    let store = ThumbStore::open(&cache);
    loop {
        let slot = normal.is_some() && take_background_slot();
        let next = match &normal {
            Some(normal) if slot => recv_thumb(&urgent, normal),
            // Without a slot, serve only the window, checking back now and then.
            Some(_) => match urgent.recv_timeout(Duration::from_millis(200)) {
                Ok(job) => Some((job, true)),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => None,
            },
            None => urgent.recv().ok().map(|job| (job, true)),
        };
        let release = || {
            if slot {
                BACKGROUND_RUNNING.fetch_sub(1, Ordering::SeqCst);
            }
        };
        let Some((job, from_window)) = next else {
            release();
            return;
        };
        if from_window {
            release();
        }
        if skip_stale(&job, &gate) {
            continue;
        }
        if from_window {
            if let Some(id) = unwanted_thumb(&job, &wanted) {
                let _ = events.send(UiEvent::ThumbSkipped { id });
                ctx.request_repaint();
                continue;
            }
            if let Ok(mut last) = WINDOW_LAST.lock() {
                *last = Some(Instant::now());
            }
        }
        let event = run_thumb(&store, &gate, job);
        if !from_window {
            release();
        }
        if let Some(event) = event {
            let _ = events.send(event);
            ctx.request_repaint();
        }
    }
}

fn skip_stale(job: &ThumbJob, gate: &DisplayGate) -> bool {
    let ThumbJob::Display { id, nonce, .. } = job else {
        return false;
    };
    stale_display(*id, *nonce, gate)
}

fn stale_display(id: i64, nonce: u64, gate: &DisplayGate) -> bool {
    gate.lock().ok().and_then(|guard| guard.get(&id).copied()) != Some(nonce)
}

/// The photo of a thumbnail request the window no longer shows.
fn unwanted_thumb(job: &ThumbJob, wanted: &ThumbWanted) -> Option<i64> {
    let ThumbJob::Thumb { id, .. } = job else {
        return None;
    };
    let shown = wanted.lock().map_or(true, |wanted| wanted.contains(id));
    (!shown).then_some(*id)
}

/// Next job, preferring the window's. The flag is true for those.
fn recv_thumb(
    urgent: &Receiver<ThumbJob>,
    normal: &Receiver<ThumbJob>,
) -> Option<(ThumbJob, bool)> {
    let from_window = |job| (job, true);
    let background = |job| (job, false);
    match urgent.try_recv() {
        Ok(job) => Some(from_window(job)),
        Err(TryRecvError::Disconnected) => normal.recv().ok().map(background),
        Err(TryRecvError::Empty) => match normal.try_recv() {
            Ok(job) => Some(background(job)),
            Err(TryRecvError::Disconnected) => urgent.recv().ok().map(from_window),
            Err(TryRecvError::Empty) => {
                let mut select = Select::new();
                let u = select.recv(urgent);
                let n = select.recv(normal);
                let ready = select.select();
                if ready.index() == u {
                    ready
                        .recv(urgent)
                        .ok()
                        .map(from_window)
                        .or_else(|| normal.recv().ok().map(background))
                } else if ready.index() == n {
                    ready
                        .recv(normal)
                        .ok()
                        .map(background)
                        .or_else(|| urgent.recv().ok().map(from_window))
                } else {
                    None
                }
            }
        },
    }
}

fn run_thumb(store: &ThumbStore, gate: &DisplayGate, job: ThumbJob) -> Option<UiEvent> {
    match job {
        ThumbJob::Cache { id, path } => {
            if !store.contains(id) {
                if let Ok(rgba) = make_thumb(Path::new(&path)) {
                    save_thumb(store, id, &rgba);
                }
            }
            None
        }
        ThumbJob::Thumb {
            id,
            path,
            edit,
            edge,
        } => {
            let file = Path::new(&path);
            if edge > THUMB_EDGE {
                // The cached thumbnail is already shown; a failure leaves it.
                let rgba = sharp_thumb(file, &edit, edge)?;
                let (w, h, rgba) = raw_rgba(&rgba);
                return Some(UiEvent::Thumb {
                    id,
                    w,
                    h,
                    rgba,
                    edit,
                });
            }
            let rgba = if edit.is_identity() {
                store.remove_edited(id);
                match plain_thumb(store, id, file) {
                    Some(rgba) => rgba,
                    None => return Some(UiEvent::ThumbFailed { id }),
                }
            } else {
                let key = edit_key(&edit);
                match store
                    .get_edited(id, &key)
                    .and_then(|jpeg| decode_jpeg(&jpeg))
                {
                    Some(rgba) => rgba,
                    None => {
                        let Some(rgba) = edited_thumb(store, id, file, &edit) else {
                            return Some(UiEvent::ThumbFailed { id });
                        };
                        if let Ok(jpeg) = image_io::encode_jpeg(&rgba, EDITED_THUMB_QUALITY) {
                            store.put_edited(id, &key, &jpeg);
                        }
                        rgba
                    }
                }
            };
            let (w, h, rgba) = raw_rgba(&rgba);
            Some(UiEvent::Thumb {
                id,
                w,
                h,
                rgba,
                edit,
            })
        }
        ThumbJob::Display {
            id,
            path,
            nonce,
            edit,
            edge,
        } => {
            let file = Path::new(&path);
            let decoded = if is_video_path(file) {
                video::poster(file, DISPLAY_EDGE).map(|(rgba, info)| (rgba, video::rows(&info)))
            } else {
                image_io::load_at_least(file, edge)
                    .map(|image| (image_io::limit_rgba(image, edge), Vec::new()))
            };
            if stale_display(id, nonce, gate) {
                return None;
            }
            let Ok((mut rgba, rows)) = decoded else {
                // Worked out here, off the window's thread, as a file on a
                // share that stopped answering can take long to look at.
                let gone = crate::files::missing(file);
                let missing = gone.is_some();
                let reason = gone.unwrap_or_else(|| tr("This photo can't be shown").to_string());
                return Some(UiEvent::DisplayFailed {
                    id,
                    nonce,
                    reason,
                    missing,
                });
            };
            if !is_video_path(file) {
                edit::apply(&mut rgba, &edit);
            }
            let histogram = Box::new(Histogram::from_rgba(rgba.as_raw()));
            let exif = if is_video_path(file) {
                rows
            } else {
                photo_info::exif_rows(file)
            };
            let (w, h, rgba) = raw_rgba(&rgba);
            Some(UiEvent::Display {
                id,
                nonce,
                w,
                h,
                rgba,
                histogram,
                exif,
                edit,
            })
        }
        ThumbJob::Exif { id, path } => Some(UiEvent::Exif {
            id,
            rows: photo_info::exif_rows(Path::new(&path)),
        }),
    }
}

/// Decodes a photo or video poster at thumbnail size.
fn make_thumb(path: &Path) -> Result<RgbaImage, String> {
    if is_video_path(path) {
        return video::poster(path, 2 * THUMB_EDGE).map(|(rgba, _)| {
            image_io::thumbnail_rgba(&DynamicImage::ImageRgba8(rgba), THUMB_EDGE)
        });
    }
    std::panic::catch_unwind(|| image_io::load_for_tile(path, THUMB_EDGE))
        .unwrap_or_else(|_| Err("decoder fault".into()))
        .map(|image| image_io::thumbnail_rgba(&image, THUMB_EDGE))
}

/// The unedited thumbnail, made and stored first if it is missing.
/// The photo's thumbnail as stored, if there is one.
pub(crate) fn stored_thumb(store: &ThumbStore, id: i64) -> Option<RgbaImage> {
    store.get(id).and_then(|jpeg| decode_jpeg(&jpeg))
}

pub(crate) fn plain_thumb(store: &ThumbStore, id: i64, file: &Path) -> Option<RgbaImage> {
    if let Some(rgba) = stored_thumb(store, id) {
        return Some(rgba);
    }
    let rgba = make_thumb(file).ok()?;
    save_thumb(store, id, &rgba);
    Some(rgba)
}

/// The thumbnail with `edit` applied.
fn edited_thumb(store: &ThumbStore, id: i64, file: &Path, edit: &Settings) -> Option<RgbaImage> {
    let reframed = !is_video_path(file) && !Geometry::from_settings(edit).is_identity();
    if let Some(rgba) = reframed
        .then(|| reframed_thumb(file, edit, THUMB_EDGE))
        .flatten()
    {
        return Some(rgba);
    }
    let mut rgba = plain_thumb(store, id, file)?;
    edit::apply(&mut rgba, edit);
    Some(rgba)
}

/// The photo, with `edit` applied, at a short edge of `edge`, for a tile
/// larger than the cached thumbnail.
fn sharp_thumb(file: &Path, edit: &Settings, edge: u32) -> Option<RgbaImage> {
    if is_video_path(file) {
        let (rgba, _) = video::poster(file, 2 * edge).ok()?;
        return Some(image_io::thumbnail_rgba(
            &DynamicImage::ImageRgba8(rgba),
            edge,
        ));
    }
    if !Geometry::from_settings(edit).is_identity() {
        return reframed_thumb(file, edit, edge);
    }
    let image = std::panic::catch_unwind(|| image_io::load_for_tile(file, edge))
        .ok()?
        .ok()?;
    let mut rgba = image_io::thumbnail_rgba(&image, edge);
    edit::apply(&mut rgba, edit);
    Some(rgba)
}

/// What a stored edited thumbnail was made from. The app version is part of
/// it, so a release that renders edits differently makes them again.
fn edit_key(edit: &Settings) -> String {
    let changes = serde_json::Value::Object(edit.changed_from(&Settings::default()));
    format!("{}:{changes}", env!("CARGO_PKG_VERSION"))
}

/// The edited photo at thumbnail size for an edit that crops, turns, or
/// tilts it. The cached thumbnail is too small for that: the crop would
/// enlarge it and the warp would soften it a second time, so the photo is
/// decoded larger, edited, and only then shrunk to a short edge of `edge`.
fn reframed_thumb(path: &Path, edit: &Settings, edge: u32) -> Option<RgbaImage> {
    let [left, top, right, bottom] = edit.crop();
    let kept = (right - left).min(bottom - top).clamp(0.05, 1.0);
    let source = ((2 * edge) as f32 / kept).ceil() as u32;
    let source = source.min(REFRAMED_SOURCE_EDGE.max(2 * edge));
    let image = std::panic::catch_unwind(|| image_io::load_for_tile(path, source))
        .ok()?
        .ok()?;
    let mut rgba = image_io::thumbnail_rgba(&image, source);
    edit::apply(&mut rgba, edit);
    Some(image_io::thumbnail_rgba(
        &DynamicImage::ImageRgba8(rgba),
        edge,
    ))
}

fn decode_jpeg(jpeg: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .ok()
        .map(|image| image.to_rgba8())
}

/// Stores a new thumbnail and how it looks, so look-alikes can be stacked
/// without reading it again.
fn save_thumb(store: &ThumbStore, id: i64, rgba: &RgbaImage) {
    if let Ok(jpeg) = image_io::encode_jpeg(rgba, 80) {
        store.put(id, &jpeg);
        store.put_look(id, crate::burst::look(rgba));
    }
}

fn raw_rgba(image: &RgbaImage) -> (usize, usize, Vec<u8>) {
    (
        image.width() as usize,
        image.height() as usize,
        image.as_raw().clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_skips_hidden_unchanged_and_other_files() {
        let root = std::env::temp_dir().join(format!("picasso-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["a/b", ".hidden", "skip"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "top.jpg",
            "a/one.png",
            "a/b/two.heic",
            "a/notes.txt",
            ".hidden/three.jpg",
            "skip/four.jpg",
            "a/ride.gpx",
            "a/b/old.KML",
            "skip/walk.kml",
        ] {
            std::fs::write(root.join(file), b"x").unwrap();
        }
        let top = root.join("top.jpg");
        let meta = std::fs::metadata(&top).unwrap();
        let known = Known {
            id: 7,
            modified_at: image_io::file_mtime(&meta),
            bytes: meta.len(),
            version: crate::db::INDEX_VERSION,
        };
        let snapshot = HashMap::from([(top.to_string_lossy().into_owned(), known)]);
        let old = root.join("a").join("b").join("old.KML");
        let old_meta = std::fs::metadata(&old).unwrap();
        let known_tracks = HashMap::from([(
            old.to_string_lossy().into_owned(),
            (old_meta.len(), image_io::file_mtime(&old_meta)),
        )]);

        let (tx, rx) = bounded::<ProbeJob>(4);
        let drain =
            std::thread::spawn(move || rx.iter().map(|job| job.filename).collect::<Vec<_>>());
        let (cancel, checked) = (AtomicBool::new(false), AtomicUsize::new(0));
        let excluded: Excluded = Arc::new(RwLock::new(vec![root.join("skip")]));
        let summary = walk_folder(
            &root,
            snapshot,
            known_tracks,
            HashMap::new(),
            HashMap::new(),
            excluded,
            tx,
            &cancel,
            &checked,
        );
        let mut probed = drain.join().unwrap();
        probed.sort();
        let _ = std::fs::remove_dir_all(&root);

        assert!(summary.complete);
        assert_eq!(summary.seen.len(), 3);
        assert_eq!(checked.load(Ordering::Relaxed), 5);
        assert_eq!(probed, ["one.png", "two.heic"]);
        assert_eq!(summary.tracks_seen.len(), 2);
        let tracks: Vec<String> = summary
            .tracks
            .iter()
            .map(|file| file.path.clone())
            .collect();
        assert_eq!(
            tracks,
            [root
                .join("a")
                .join("ride.gpx")
                .to_string_lossy()
                .into_owned()]
        );
    }

    #[test]
    fn unreadable_folders_keep_the_photos_known_under_them() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = |parts: &[&str]| parts.join(&sep.to_string());
        let known = [
            path(&["root", "lost", "a.jpg"]),
            path(&["root", "lost", "deep", "b.jpg"]),
            path(&["root", "lost2", "c.jpg"]),
            path(&["root", "file.jpg"]),
            path(&["root", "gone.jpg"]),
        ];
        let unreadable = [
            PathBuf::from(path(&["root", "lost"])),
            PathBuf::from(path(&["root", "file.jpg"])),
        ];
        let mut kept = known_under(known.iter(), &unreadable);
        kept.sort();
        assert_eq!(kept, [known[3].clone(), known[0].clone(), known[1].clone()]);
        assert!(known_under(known.iter(), &[]).is_empty());
    }

    /// A library in a fresh temporary folder, scanned by a real indexer.
    struct Library {
        dir: PathBuf,
        indexer: Indexer,
        commands: Sender<Command>,
        events: Receiver<UiEvent>,
    }

    impl Library {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("picasso-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("cache")).unwrap();
            let dir = dir.canonicalize().unwrap();
            let db = Database::open(&dir.join("picasso.db")).unwrap();
            let (command_tx, commands) = crossbeam_channel::unbounded();
            let (thumbs, _) = crossbeam_channel::unbounded();
            let (events, event_rx) = crossbeam_channel::unbounded();
            let cache = dir.join("cache");
            let indexer = Indexer {
                db,
                thumb_store: ThumbStore::open(&cache),
                cache,
                commands,
                thumbs,
                events,
                ctx: eframe::egui::Context::default(),
                pending: VecDeque::new(),
                quick: HashSet::new(),
                deferred: Vec::new(),
                backfilled: true,
                not_tracks: RefCell::new(HashMap::new()),
            };
            Library {
                dir,
                indexer,
                commands: command_tx,
                events: event_rx,
            }
        }

        fn add(&self, folder: &str, photos: &[&str]) -> i64 {
            let root = self.dir.join(folder);
            std::fs::create_dir_all(&root).unwrap();
            for photo in photos {
                let path = root.join(photo);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                image::RgbaImage::new(4, 3).save(&path).unwrap();
            }
            self.indexer.db.add_folder(&root).unwrap()
        }

        fn scan(&mut self, folder: i64) {
            assert!(matches!(
                self.indexer.scan_folder(folder),
                Outcome::Finished
            ));
        }

        /// Photo ids by path relative to the library folder.
        fn photos(&self) -> HashMap<String, i64> {
            let prefix = format!("{}{}", self.dir.display(), std::path::MAIN_SEPARATOR);
            self.indexer
                .db
                .photo_paths()
                .into_iter()
                .map(|(id, path)| (path.strip_prefix(&prefix).unwrap().replace('\\', "/"), id))
                .collect()
        }

        fn caption(&self, id: i64) -> String {
            self.indexer.db.photo(id).unwrap().caption
        }
    }

    impl Drop for Library {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_folder_excluded_during_a_scan_stops_being_read_and_the_scan_goes_on() {
        // The scan is quick, so it may be over before the folder is
        // excluded; it is tried again until the exclusion lands mid-scan.
        let mut attempt = 0;
        let (mut lib, folder) = loop {
            attempt += 1;
            let (lib, folder, written) = scan_excluding_midway(attempt);
            if written < 600 {
                break (lib, folder);
            }
            assert!(attempt < 5, "the scan always ended before the exclusion");
        };
        let photos = lib.photos();
        assert!(
            !photos.keys().any(|path| path.starts_with("pics/big/")),
            "nothing found in the excluded folder comes back: {} photos",
            photos
                .keys()
                .filter(|path| path.starts_with("pics/big/"))
                .count()
        );
        // The rest of the folder is scanned, and the scan reaches its end.
        assert!(photos.contains_key("pics/a.png"));
        assert!(photos.contains_key("pics/other/b.png"));
        assert!(lib.indexer.db.pending_folders().is_empty());
        // And the next scan leaves the folder out too.
        lib.scan(folder);
        assert_eq!(lib.photos(), photos);
    }

    /// Scans a folder of 602 photos, 600 of them in `big/`, which is
    /// excluded once the scan has written its first photos. Returns how many
    /// of `big/` were written by then.
    fn scan_excluding_midway(attempt: usize) -> (Library, i64, usize) {
        let mut lib = Library::new(&format!("excluded-mid-scan-{attempt}"));
        let many: Vec<String> = (0..600).map(|n| format!("big/{n:03}.png")).collect();
        let mut names: Vec<&str> = many.iter().map(String::as_str).collect();
        names.extend(["a.png", "other/b.png"]);
        let folder = lib.add("pics", &names);
        let big = lib.dir.join("pics/big");
        // Once the scan has written its first photos, the folder with most
        // of them is excluded, from the window's own connection.
        let helper = {
            let (events, commands) = (lib.events.clone(), lib.commands.clone());
            let db_path = lib.dir.join("picasso.db");
            let big = big.clone();
            std::thread::spawn(move || {
                for event in events.iter() {
                    if matches!(event, UiEvent::ScanProgress { indexed, .. } if indexed > 0) {
                        let db = Database::open(&db_path).unwrap();
                        let real = big.canonicalize().unwrap();
                        let written = db
                            .photos(&crate::types::Nav::Library, "", crate::types::Sort::Name)
                            .iter()
                            .filter(|photo| {
                                let path = Path::new(&photo.path);
                                path.starts_with(&big) || path.starts_with(&real)
                            })
                            .count();
                        db.exclude_dir(&big).unwrap();
                        commands.send(Command::Excluded).unwrap();
                        return written;
                    }
                }
                usize::MAX
            })
        };
        lib.scan(folder);
        let written = helper.join().unwrap();
        (lib, folder, written)
    }

    #[test]
    fn renamed_folders_keep_their_photos() {
        let mut lib = Library::new("rename-folder");
        let folder = lib.add("pics", &["trip/a.png", "trip/b.png"]);
        lib.scan(folder);
        let before = lib.photos();
        let a = before["pics/trip/a.png"];
        lib.indexer.db.set_caption(a, "beach");

        std::fs::rename(lib.dir.join("pics/trip"), lib.dir.join("pics/holiday")).unwrap();
        image::RgbaImage::new(5, 5)
            .save(lib.dir.join("pics/holiday/new.png"))
            .unwrap();
        lib.events.try_iter().for_each(drop);
        lib.scan(folder);
        // The folder tree follows the rename before the new photo is read.
        let first = lib.events.try_iter().find(|event| {
            matches!(event, UiEvent::Library)
                || matches!(event, UiEvent::ScanProgress { indexed, .. } if *indexed > 0)
        });
        assert!(matches!(first, Some(UiEvent::Library)));
        let after = lib.photos();
        assert_eq!(after.len(), 3);
        assert_eq!(after["pics/holiday/a.png"], a);
        assert_eq!(after["pics/holiday/b.png"], before["pics/trip/b.png"]);
        assert_eq!(lib.caption(a), "beach");
        assert_eq!(lib.indexer.db.photo(a).unwrap().filename, "a.png");
    }

    /// A Trip of `photos` with a Story showing the first; returns both.
    fn trip_with_story(db: &Database, photos: &[i64]) -> (i64, i64) {
        use crate::trip::{Block, MediaUse, StoryBlock, TripDraft};
        let trip = db
            .create_trip(&TripDraft::new("Coast", "Europe/Lisbon"))
            .unwrap();
        let refs = db.add_to_trip(trip, photos).unwrap();
        let story = db.create_story(trip, "Coast").unwrap();
        db.save_story_blocks(
            story,
            1,
            &[StoryBlock::new(Block::Photo(MediaUse::new(refs[0])))],
        )
        .unwrap();
        (trip, story)
    }

    /// Each member of the Trip with whether its photo is in the library, and the Story's blocks.
    fn trip_state(
        db: &Database,
        trip: i64,
        story: i64,
    ) -> (Vec<(i64, bool)>, Vec<crate::trip::Block>) {
        let members = db.trip_members(trip, &crate::trip::MemberQuery::default(), 0, 100);
        let blocks = db
            .story_blocks(story)
            .into_iter()
            .map(|block| block.block)
            .collect();
        (
            members
                .into_iter()
                .map(|member| (member.media_id, member.photo_id.is_some()))
                .collect(),
            blocks,
        )
    }

    #[test]
    fn an_unplugged_drive_keeps_its_photos_trips_and_stories_until_it_is_back() {
        let mut lib = Library::new("unplugged");
        let folder = lib.add("drive/pics", &["a.png", "sub/b.png"]);
        lib.scan(folder);
        let before = lib.photos();
        let (trip, story) = trip_with_story(
            &lib.indexer.db,
            &[before["drive/pics/a.png"], before["drive/pics/sub/b.png"]],
        );
        let state = trip_state(&lib.indexer.db, trip, story);
        assert!(state.0.iter().all(|(_, there)| *there));

        // The drive goes, with the place it was mounted, as an unplugged disk or a dropped share does.
        std::fs::rename(lib.dir.join("drive"), lib.dir.join("away")).unwrap();
        lib.scan(folder);
        assert_eq!(lib.photos(), before, "nothing leaves the library");
        assert!(
            lib.indexer.db.pending_folders().is_empty(),
            "its pending flag is left as it was"
        );
        assert_eq!(
            trip_state(&lib.indexer.db, trip, story),
            state,
            "the Trip and its Story are as they were"
        );

        std::fs::rename(lib.dir.join("away"), lib.dir.join("drive")).unwrap();
        lib.scan(folder);
        assert_eq!(lib.photos(), before, "the same photos, not new ones");
        assert!(lib.indexer.db.pending_folders().is_empty());
        assert_eq!(trip_state(&lib.indexer.db, trip, story), state);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_would_never_answer_is_passed_over_by_the_scan() {
        let mut lib = Library::new("never-answers");
        let folder = lib.add("pics", &["a.png"]);
        crate::app::headless::named_pipe(&lib.dir.join("pics/stuck.jpg"));
        lib.scan(folder);
        assert_eq!(lib.photos().into_keys().collect::<Vec<_>>(), ["pics/a.png"]);
    }

    #[test]
    fn a_scan_stopped_partway_keeps_what_it_had_not_reached() {
        let mut lib = Library::new("scan-stopped");
        let folder = lib.add("pics", &["a.png", "b.png"]);
        lib.scan(folder);
        let before = lib.photos();
        let (trip, story) = trip_with_story(
            &lib.indexer.db,
            &[before["pics/a.png"], before["pics/b.png"]],
        );
        let state = trip_state(&lib.indexer.db, trip, story);

        // A photo deleted on disk, then Picasso closed as the scan starts.
        std::fs::remove_file(lib.dir.join("pics/b.png")).unwrap();
        lib.indexer.db.set_scan_pending(folder, true);
        lib.commands.send(Command::Stop).unwrap();
        assert!(matches!(lib.indexer.scan_folder(folder), Outcome::Stopped));
        assert_eq!(
            lib.photos(),
            before,
            "a scan that didn't finish removes nothing"
        );
        assert_eq!(
            lib.indexer.db.pending_folders(),
            [folder],
            "and continues next time"
        );
        assert_eq!(trip_state(&lib.indexer.db, trip, story), state);

        // The next whole scan sees the deletion; the Trip keeps the photo as unavailable.
        lib.scan(folder);
        assert_eq!(lib.photos().into_keys().collect::<Vec<_>>(), ["pics/a.png"]);
        assert!(lib.indexer.db.pending_folders().is_empty());
        let (members, blocks) = trip_state(&lib.indexer.db, trip, story);
        assert_eq!(
            members.iter().map(|(_, there)| *there).collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!(blocks, state.1, "the Story keeps its blocks");
    }

    #[test]
    fn deleted_folders_leave_the_library() {
        let mut lib = Library::new("delete-folder");
        let folder = lib.add("pics", &["keep.png", "old/a.png"]);
        let other = lib.add("more", &["b.png"]);
        lib.scan(folder);
        lib.scan(other);
        assert_eq!(lib.photos().len(), 3);

        std::fs::remove_dir_all(lib.dir.join("pics/old")).unwrap();
        lib.scan(folder);
        let mut left: Vec<String> = lib.photos().into_keys().collect();
        left.sort();
        assert_eq!(left, ["more/b.png", "pics/keep.png"]);

        // A whole library folder can go too, while an unplugged drive can't.
        std::fs::remove_dir_all(lib.dir.join("more")).unwrap();
        lib.scan(other);
        assert_eq!(lib.photos().keys().collect::<Vec<_>>(), ["pics/keep.png"]);
        assert!(lib.indexer.db.folder_path(other).is_none());
        assert!(crate::db::folder_gone(&lib.dir.join("more")));
        assert!(!crate::db::folder_gone(
            &lib.dir.join("missing-drive").join("pics")
        ));
        assert!(!crate::db::folder_gone(Path::new("/Volumes/Card")));
        assert!(!crate::db::folder_gone(&lib.dir));
    }

    #[test]
    fn renamed_added_folders_are_followed() {
        let mut lib = Library::new("rename-added");
        let folder = lib.add("pics", &["a.png", "sub/b.png"]);
        lib.add("other", &[]);
        lib.scan(folder);
        let a = lib.photos()["pics/a.png"];
        lib.indexer.db.set_caption(a, "home");

        std::fs::rename(lib.dir.join("pics"), lib.dir.join("photos")).unwrap();
        lib.scan(folder);
        let photos = lib.photos();
        assert_eq!(
            lib.indexer.db.folder_path(folder),
            Some(lib.dir.join("photos").to_string_lossy().into_owned())
        );
        assert_eq!(photos.len(), 2);
        assert_eq!(photos["photos/a.png"], a);
        assert!(photos.contains_key("photos/sub/b.png"));
        assert_eq!(lib.caption(a), "home");
    }

    /// Sets the modification time of folder `dir` to `when`.
    fn set_dir_time(dir: &Path, when: std::time::SystemTime) {
        let mut options = std::fs::OpenOptions::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_WRITE_ATTRIBUTES, and FILE_FLAG_BACKUP_SEMANTICS to open a folder.
            options.access_mode(0x100).custom_flags(0x0200_0000);
        }
        #[cfg(not(windows))]
        options.read(true);
        options.open(dir).unwrap().set_modified(when).unwrap();
    }

    #[test]
    fn a_launch_check_skips_unchanged_folders_and_reads_changed_ones() {
        let mut lib = Library::new("check");
        let folder = lib.add(
            "pics",
            &["a.png", "trip/b.png", "trip/day/c.png", "keep/d.png"],
        );
        if !crate::files::folder_times_kept(&lib.dir) {
            eprintln!("skipped: this file system's folder times aren't trusted");
            return;
        }
        lib.scan(folder);
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        let dirs = ["pics", "pics/trip", "pics/trip/day", "pics/keep"];
        for dir in dirs {
            set_dir_time(&lib.dir.join(dir), long_ago);
        }
        // Folders changed moments before a scan are listed again next time;
        // these were changed long ago, so this scan keeps their times.
        lib.scan(folder);
        let stamps = lib.indexer.db.dir_stamps(folder);
        assert_eq!(stamps.len(), dirs.len());
        assert!(stamps.values().all(|&stamp| stamp > 0), "{stamps:?}");

        // A file written in place leaves its folder's time as it was, so a
        // check doesn't see it; a file added or deleted changes the folder.
        image::RgbaImage::new(4, 3)
            .save(lib.dir.join("pics/trip/hidden.png"))
            .unwrap();
        set_dir_time(&lib.dir.join("pics/trip"), long_ago);
        image::RgbaImage::new(4, 3)
            .save(lib.dir.join("pics/trip/day/new.png"))
            .unwrap();
        std::fs::remove_file(lib.dir.join("pics/keep/d.png")).unwrap();
        lib.indexer.quick.insert(folder);
        lib.scan(folder);
        let mut found: Vec<String> = lib.photos().into_keys().collect();
        found.sort();
        assert_eq!(
            found,
            [
                "pics/a.png",
                "pics/trip/b.png",
                "pics/trip/day/c.png",
                "pics/trip/day/new.png"
            ]
        );

        // A full scan reads every file.
        lib.scan(folder);
        assert!(lib.photos().contains_key("pics/trip/hidden.png"));
    }

    #[test]
    fn a_folder_on_another_drive_than_its_added_folder_has_no_time_kept() {
        let dir = std::env::temp_dir();
        let device = device_of(&std::fs::metadata(&dir).unwrap());
        assert!(dir_stamp(&dir, device).is_some());
        #[cfg(unix)]
        assert_eq!(dir_stamp(&dir, device.map(|device| device + 1)), None);
    }

    #[test]
    fn a_folder_with_a_file_that_could_not_be_read_is_listed_by_every_check() {
        let mut lib = Library::new("check-unreadable");
        let folder = lib.add("pics", &["a.png", "bad/b.png"]);
        if !crate::files::folder_times_kept(&lib.dir) {
            eprintln!("skipped: this file system's folder times aren't trusted");
            return;
        }
        std::fs::write(lib.dir.join("pics/bad/broken.jpg"), b"not a photo").unwrap();
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        set_dir_time(&lib.dir.join("pics"), long_ago);
        set_dir_time(&lib.dir.join("pics/bad"), long_ago);
        lib.scan(folder);
        let stamps = lib.indexer.db.dir_stamps(folder);
        let path = |dir: &str| lib.dir.join(dir).to_string_lossy().into_owned();
        assert!(stamps[&path("pics")] > 0);
        assert_eq!(stamps[&path("pics/bad")], -1);
    }

    #[test]
    fn renamed_files_keep_their_photos() {
        let mut lib = Library::new("rename-file");
        let folder = lib.add("pics", &["a.png"]);
        lib.scan(folder);
        let a = lib.photos()["pics/a.png"];
        lib.indexer.db.set_caption(a, "dog");

        std::fs::rename(lib.dir.join("pics/a.png"), lib.dir.join("pics/rex.png")).unwrap();
        lib.scan(folder);
        let photos = lib.photos();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos["pics/rex.png"], a);
        assert_eq!(lib.indexer.db.photo(a).unwrap().filename, "rex.png");
        assert_eq!(lib.caption(a), "dog");
    }

    #[test]
    fn moved_folders_keep_their_photos() {
        let mut lib = Library::new("move-folder");
        let from = lib.add("from", &["trip/a.png", "stay.png"]);
        let to = lib.add("to", &["other.png"]);
        lib.scan(from);
        lib.scan(to);
        let a = lib.photos()["from/trip/a.png"];
        lib.indexer.db.set_caption(a, "summit");

        // Into another folder of the same library folder...
        std::fs::create_dir_all(lib.dir.join("from/2024")).unwrap();
        std::fs::rename(lib.dir.join("from/trip"), lib.dir.join("from/2024/trip")).unwrap();
        lib.scan(from);
        assert_eq!(lib.photos()["from/2024/trip/a.png"], a);

        // ...and into another library folder, found by that folder's scan
        // before or after the scan of the folder it left.
        std::fs::rename(lib.dir.join("from/2024/trip"), lib.dir.join("to/trip")).unwrap();
        lib.scan(to);
        lib.scan(from);
        let photos = lib.photos();
        assert_eq!(photos.len(), 3);
        assert_eq!(photos["to/trip/a.png"], a);
        assert_eq!(lib.caption(a), "summit");
        assert_eq!(lib.indexer.db.photo(a).unwrap().folder_id, to);

        std::fs::rename(lib.dir.join("to/trip"), lib.dir.join("from/trip")).unwrap();
        lib.scan(from);
        lib.scan(to);
        assert_eq!(lib.photos()["from/trip/a.png"], a);
        assert_eq!(lib.caption(a), "summit");
    }
}
