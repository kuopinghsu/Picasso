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
//! photos are queued once the scans finish, so the two don't compete for the
//! disk; the window asks for the thumbnails it shows straight away.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Select, Sender, TryRecvError};
use image::RgbaImage;
use crate::db::{Database, Known, ScanRecord};
use crate::edit::{self, Settings};
use crate::image_io::{self, Probe};
use crate::photo_info::{self, Histogram};
use crate::picasa_ini::PicasaIni;
use crate::types::{folder_label, is_media_path, is_video_path, unix_now, NewPhoto};
use crate::video;

pub type DisplayGate = Arc<Mutex<HashMap<i64, u64>>>;

/// Records written per transaction while a scan runs.
const BATCH_SIZE: usize = 200;
/// Longest wait before probed records are committed and shown.
const BATCH_INTERVAL: Duration = Duration::from_millis(250);

pub enum Command {
    Scan(i64),
    /// Ask the index thread to finish. Sent when the window closes.
    Stop,
}

pub enum ThumbJob {
    /// `edit` is applied over the decoded pixels; the disk cache keeps the original.
    Thumb { id: i64, path: String, edit: Settings },
    Display { id: i64, path: String, nonce: u64, edit: Settings },
    /// Writes the disk thumbnail during a scan without sending pixels to the
    /// window, which asks for the ones it shows.
    Cache { id: i64, path: String },
    /// EXIF rows only, without decoding pixels.
    Exif { id: i64, path: String },
}

pub enum UiEvent {
    ScanStarted(String),
    ScanProgress {
        folder: String,
        checked: usize,
        indexed: usize,
    },
    ScanFinished {
        errors: usize,
    },
    Library,
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
    DisplayFailed {
        id: i64,
        nonce: u64,
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
}

/// Number of threads that probe files during a scan. A probe mostly waits
/// on the disk or on ffprobe, so there are more of them than cores.
pub fn probe_threads() -> usize {
    (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) * 2).clamp(4, 16)
}

/// Number of threads that decode thumbnails and full-size views.
pub fn thumb_threads() -> usize {
    (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) * 3 / 4).clamp(2, 8)
}

/// Long edge of a cached thumbnail.
const THUMB_EDGE: u32 = 256;
/// Long edge of the full-size view.
const DISPLAY_EDGE: u32 = 2560;

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
    errors: usize,
    complete: bool,
}

enum Outcome {
    Finished,
    Stopped,
}

struct Indexer {
    db: Database,
    cache: PathBuf,
    commands: Receiver<Command>,
    thumbs: Sender<ThumbJob>,
    events: Sender<UiEvent>,
    ctx: eframe::egui::Context,
    pending: VecDeque<i64>,
    /// Disk thumbnails for photos found by the scans in progress. Making
    /// them reads each whole file, which would compete with the scan for the
    /// disk, so they are queued once every pending folder is done. Photos on
    /// screen get theirs sooner because the window asks for them.
    deferred: Vec<ThumbJob>,
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
            let _ = events.send(UiEvent::ScanFinished { errors: 0 });
            eprintln!("picasso database: {err}");
            return;
        }
    };
    // Folders whose last scan did not reach the end go first.
    let pending = db.pending_folders().into_iter().collect();
    let mut indexer = Indexer {
        db,
        cache,
        commands,
        thumbs,
        events,
        ctx,
        pending,
        deferred: Vec::new(),
    };
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
            match self.commands.recv() {
                Ok(Command::Scan(id)) => self.queue(id),
                Ok(Command::Stop) | Err(_) => return,
            }
        }
    }

    fn queue(&mut self, id: i64) {
        if !self.pending.contains(&id) {
            self.pending.push_back(id);
        }
    }

    fn notify(&self, event: UiEvent) {
        let _ = self.events.send(event);
        self.ctx.request_repaint();
    }

    fn scan_folder(&mut self, folder_id: i64) -> Outcome {
        let Some(path) = self.db.folder_path(folder_id) else {
            return Outcome::Finished;
        };
        let root = PathBuf::from(&path);
        if !root.is_dir() {
            // An unplugged drive keeps its photos and its pending flag.
            return Outcome::Finished;
        }
        let label = folder_label(&path);
        self.db.set_scan_pending(folder_id, true);
        self.notify(UiEvent::ScanStarted(label.clone()));

        let snapshot = self.db.folder_snapshot(folder_id, &path);
        let cancel = Arc::new(AtomicBool::new(false));
        let checked = Arc::new(AtomicUsize::new(0));
        let (job_tx, job_rx) = bounded::<ProbeJob>(512);
        let (done_tx, done_rx) = bounded::<ProbeResult>(512);

        let walker = {
            let cancel = cancel.clone();
            let checked = checked.clone();
            std::thread::Builder::new()
                .name("picasso-walk".into())
                .spawn(move || walk_folder(&root, snapshot, job_tx, &cancel, &checked))
        };
        let walker = match walker {
            Ok(handle) => handle,
            Err(err) => {
                eprintln!("picasso index: {err}");
                return Outcome::Finished;
            }
        };
        for index in 0..probe_threads() {
            let jobs = job_rx.clone();
            let done = done_tx.clone();
            let cancel = cancel.clone();
            let _ = std::thread::Builder::new()
                .name(format!("picasso-probe-{index}"))
                .spawn(move || probe_loop(jobs, done, &cancel));
        }
        drop(job_rx);
        drop(done_tx);

        let mut ini_cache: HashMap<PathBuf, PicasaIni> = HashMap::new();
        let mut batch: Vec<ScanRecord> = Vec::with_capacity(BATCH_SIZE);
        let mut errors = 0usize;
        let mut indexed = 0usize;
        let mut again = false;
        let mut last_flush = Instant::now();
        let mut stopped = false;

        loop {
            loop {
                match self.commands.try_recv() {
                    Ok(Command::Scan(id)) if id == folder_id => again = true,
                    Ok(Command::Scan(id)) => self.queue(id),
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
            let finished = match done_rx.recv_timeout(BATCH_INTERVAL) {
                Ok(result) => {
                    match self.record(folder_id, result, &mut ini_cache) {
                        Some(record) => batch.push(record),
                        None => errors += 1,
                    }
                    false
                }
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => true,
            };
            let due = batch.len() >= BATCH_SIZE || last_flush.elapsed() >= BATCH_INTERVAL;
            if finished || due {
                indexed += self.flush(&mut batch, &mut errors);
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
            drop(done_rx);
            self.flush(&mut batch, &mut errors);
            return Outcome::Stopped;
        }

        let summary = walker.join().unwrap_or(WalkSummary {
            seen: Vec::new(),
            errors: 1,
            complete: false,
        });
        errors += summary.errors;
        if summary.complete {
            self.db.remove_missing(folder_id, &summary.seen);
            self.db.set_scan_pending(folder_id, false);
        }
        self.notify(UiEvent::ScanFinished { errors });
        self.notify(UiEvent::Library);
        if again {
            self.pending.push_front(folder_id);
        }
        Outcome::Finished
    }

    fn record(
        &self,
        folder_id: i64,
        result: ProbeResult,
        ini_cache: &mut HashMap<PathBuf, PicasaIni>,
    ) -> Option<ScanRecord> {
        let ProbeResult { job, probe } = result;
        let probe = probe.ok()?;
        let (star, caption, rotation) = if job.existing.is_none() {
            let parent = job.path.parent().unwrap_or(&job.path).to_path_buf();
            let ini = ini_cache.entry(parent.clone()).or_insert_with(|| PicasaIni::load(&parent));
            ini.get(&job.filename)
                .map(|meta| (meta.star, meta.caption.clone(), meta.rotation))
                .unwrap_or((None, None, None))
        } else {
            (None, None, None)
        };
        Some(ScanRecord {
            photo: NewPhoto {
                folder_id,
                path: job.path_text,
                filename: job.filename,
                width: probe.width,
                height: probe.height,
                bytes: job.bytes,
                taken_at: if probe.taken_at == 0 { unix_now() } else { probe.taken_at },
                modified_at: job.modified_at,
                camera: probe.camera,
                gps_lat: probe.gps_lat,
                gps_lon: probe.gps_lon,
                duration: probe.duration,
            },
            existing: job.existing,
            star,
            caption,
            rotation,
            complete: probe.complete,
        })
    }

    /// Commits the batch and queues background thumbnails for the written rows.
    fn flush(&mut self, batch: &mut Vec<ScanRecord>, errors: &mut usize) -> usize {
        if batch.is_empty() {
            return 0;
        }
        let written = match self.db.write_batch(batch) {
            Ok(result) => result,
            Err(err) => {
                eprintln!("picasso index: {err}");
                *errors += batch.len();
                batch.clear();
                return 0;
            }
        };
        for (record, id) in batch.iter().zip(&written.ids) {
            let (id, path) = (*id, record.photo.path.clone());
            // A changed file's old thumbnail may be on screen, so it is sent
            // again; new photos only need their disk thumbnail.
            let job = if record.existing.is_some() {
                let _ = std::fs::remove_file(self.cache.join("thumbs").join(format!("{id}.jpg")));
                ThumbJob::Thumb { id, path, edit: Settings::default() }
            } else {
                ThumbJob::Cache { id, path }
            };
            if matches!(job, ThumbJob::Cache { .. }) {
                self.deferred.push(job);
            } else {
                let _ = self.thumbs.send(job);
            }
        }
        let count = batch.len();
        batch.clear();
        self.notify(UiEvent::Library);
        count
    }
}

/// Shared state of one folder walk.
struct Walk<'a> {
    snapshot: HashMap<String, Known>,
    jobs: Sender<ProbeJob>,
    cancel: &'a AtomicBool,
    checked: &'a AtomicUsize,
    seen: Mutex<Vec<String>>,
    errors: AtomicUsize,
    /// Cleared when the root could not be listed or the walk stopped early,
    /// so missing photos are not removed on the strength of a partial listing.
    complete: AtomicBool,
}

/// Lists `root` with one task per directory on a small private pool, so
/// directories on slow or network drives are read in parallel. Size and
/// modification time come from the directory listing itself, which on
/// Windows saves opening every file.
fn walk_folder(
    root: &Path,
    snapshot: HashMap<String, Known>,
    jobs: Sender<ProbeJob>,
    cancel: &AtomicBool,
    checked: &AtomicUsize,
) -> WalkSummary {
    let walk = Walk {
        seen: Mutex::new(Vec::with_capacity(snapshot.len())),
        snapshot,
        jobs,
        cancel,
        checked,
        errors: AtomicUsize::new(0),
        complete: AtomicBool::new(true),
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
    WalkSummary {
        seen: walk.seen.into_inner().unwrap_or_default(),
        errors: walk.errors.load(Ordering::Relaxed),
        complete,
    }
}

fn walk_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 4)
}

fn walk_dir<'s>(scope: &rayon::Scope<'s>, dir: PathBuf, root: bool, walk: &'s Walk<'s>) {
    if walk.cancel.load(Ordering::Relaxed) {
        return;
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => {
            walk.errors.fetch_add(1, Ordering::Relaxed);
            if root {
                walk.complete.store(false, Ordering::Relaxed);
            }
            return;
        }
    };
    let mut seen = Vec::new();
    let mut jobs = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            walk.errors.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            walk.errors.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            scope.spawn(move |scope| walk_dir(scope, path, false, walk));
            continue;
        }
        if !is_media_path(&path) {
            continue;
        }
        // A link's own entry describes the link, so follow it to the file.
        let meta = if file_type.is_symlink() { std::fs::metadata(&path) } else { entry.metadata() };
        let meta = match meta {
            Ok(meta) if meta.is_file() => meta,
            _ => continue,
        };
        walk.checked.fetch_add(1, Ordering::Relaxed);
        let path_text = path.to_string_lossy().into_owned();
        let modified_at = image_io::file_mtime(&meta);
        let bytes = meta.len();
        let known = walk.snapshot.get(&path_text).copied();
        seen.push(path_text.clone());
        if known.is_some_and(|known| known.current(modified_at, bytes)) {
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
    if let Ok(mut all) = walk.seen.lock() {
        all.extend(seen);
    }
    for job in jobs {
        if walk.cancel.load(Ordering::Relaxed) || walk.jobs.send(job).is_err() {
            walk.complete.store(false, Ordering::Relaxed);
            return;
        }
    }
}

fn probe_loop(jobs: Receiver<ProbeJob>, done: Sender<ProbeResult>, cancel: &AtomicBool) {
    for job in jobs.iter() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let probe = std::panic::catch_unwind(|| image_io::probe(&job.path, job.modified_at))
            .unwrap_or_else(|_| Err("decoder fault".into()));
        if done.send(ProbeResult { job, probe }).is_err() {
            return;
        }
    }
}

pub fn thumb_loop(
    urgent: Receiver<ThumbJob>,
    normal: Receiver<ThumbJob>,
    events: Sender<UiEvent>,
    ctx: eframe::egui::Context,
    cache: PathBuf,
    gate: DisplayGate,
) {
    loop {
        let Some(job) = recv_thumb(&urgent, &normal) else {
            return;
        };
        if skip_stale(&job, &gate) {
            continue;
        }
        if let Some(event) = run_thumb(&cache, &gate, job) {
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

fn recv_thumb(urgent: &Receiver<ThumbJob>, normal: &Receiver<ThumbJob>) -> Option<ThumbJob> {
    match urgent.try_recv() {
        Ok(job) => Some(job),
        Err(TryRecvError::Disconnected) => normal.recv().ok(),
        Err(TryRecvError::Empty) => match normal.try_recv() {
            Ok(job) => Some(job),
            Err(TryRecvError::Disconnected) => urgent.recv().ok(),
            Err(TryRecvError::Empty) => {
                let mut select = Select::new();
                let u = select.recv(urgent);
                let n = select.recv(normal);
                let ready = select.select();
                if ready.index() == u {
                    ready.recv(urgent).ok().or_else(|| normal.recv().ok())
                } else if ready.index() == n {
                    ready.recv(normal).ok().or_else(|| urgent.recv().ok())
                } else {
                    None
                }
            }
        },
    }
}

fn run_thumb(cache: &Path, gate: &DisplayGate, job: ThumbJob) -> Option<UiEvent> {
    match job {
        ThumbJob::Cache { id, path } => {
            let cached = cache.join("thumbs").join(format!("{id}.jpg"));
            if !cached.exists() {
                if let Ok(rgba) = make_thumb(Path::new(&path)) {
                    let _ = image_io::save_jpeg(&cached, &rgba, 80);
                }
            }
            None
        }
        ThumbJob::Thumb { id, path, edit } => {
            let cached = cache.join("thumbs").join(format!("{id}.jpg"));
            let mut rgba = match open_rgba(&cached) {
                Some(rgba) => rgba,
                None => {
                    let Ok(rgba) = make_thumb(Path::new(&path)) else {
                        return Some(UiEvent::ThumbFailed { id });
                    };
                    let _ = image_io::save_jpeg(&cached, &rgba, 80);
                    rgba
                }
            };
            edit::apply(&mut rgba, &edit);
            let (w, h, rgba) = raw_rgba(&rgba);
            Some(UiEvent::Thumb { id, w, h, rgba, edit })
        }
        ThumbJob::Display { id, path, nonce, edit } => {
            let file = Path::new(&path);
            let decoded = if is_video_path(file) {
                video::poster(file, DISPLAY_EDGE).map(|(rgba, info)| (rgba, video::rows(&info)))
            } else {
                image_io::load_at_least(file, DISPLAY_EDGE).map(|image| (image_io::limit_rgba(image, DISPLAY_EDGE), Vec::new()))
            };
            if stale_display(id, nonce, gate) {
                return None;
            }
            let Ok((mut rgba, rows)) = decoded else {
                return Some(UiEvent::DisplayFailed { id, nonce });
            };
            if !is_video_path(file) {
                edit::apply(&mut rgba, &edit);
            }
            let histogram = Box::new(Histogram::from_rgba(rgba.as_raw()));
            let exif = if is_video_path(file) { rows } else { photo_info::exif_rows(file) };
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
        return video::poster(path, THUMB_EDGE).map(|(rgba, _)| rgba);
    }
    std::panic::catch_unwind(|| image_io::load_at_least(path, THUMB_EDGE))
        .unwrap_or_else(|_| Err("decoder fault".into()))
        .map(|image| image_io::thumbnail_rgba(&image, THUMB_EDGE))
}

/// Reads a cached thumbnail in one read and decodes it from memory.
fn open_rgba(path: &Path) -> Option<RgbaImage> {
    let bytes = std::fs::read(path).ok()?;
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).ok().map(|image| image.to_rgba8())
}

fn raw_rgba(image: &RgbaImage) -> (usize, usize, Vec<u8>) {
    (image.width() as usize, image.height() as usize, image.as_raw().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_skips_hidden_unchanged_and_other_files() {
        let root = std::env::temp_dir().join(format!("picasso-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["a/b", ".hidden"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["top.jpg", "a/one.png", "a/b/two.heic", "a/notes.txt", ".hidden/three.jpg"] {
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

        let (tx, rx) = bounded::<ProbeJob>(4);
        let drain = std::thread::spawn(move || rx.iter().map(|job| job.filename).collect::<Vec<_>>());
        let (cancel, checked) = (AtomicBool::new(false), AtomicUsize::new(0));
        let summary = walk_folder(&root, snapshot, tx, &cancel, &checked);
        let mut probed = drain.join().unwrap();
        probed.sort();
        let _ = std::fs::remove_dir_all(&root);

        assert!(summary.complete);
        assert_eq!(summary.seen.len(), 3);
        assert_eq!(checked.load(Ordering::Relaxed), 3);
        assert_eq!(probed, ["one.png", "two.heic"]);
    }
}
