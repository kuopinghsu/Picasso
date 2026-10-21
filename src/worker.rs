//! Background folder indexing, face detection, and thumbnail decode.
//!
//! Thumbnails run on their own thread so a face scan never blocks the grid.
//! The indexer walks a folder a slice at a time and interleaves one face so the
//! UI gets new photos before the whole library is done.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Select, Sender, TryRecvError};
use image::RgbaImage;
use walkdir::WalkDir;

use crate::db::{Database, NewFace, Upsert};
use crate::faces::{self, Detection};
use crate::geom::iou;
use crate::image_io::{self, face_crop};
use crate::picasa_ini::{IniPhoto, PicasaIni};
use crate::types::{folder_label, is_image_path, unix_now, NewPhoto, RelRect};

pub type DisplayGate = Arc<Mutex<HashMap<i64, u64>>>;

pub enum Command {
    Scan(i64),
    Redetect(Vec<i64>),
    /// Ask the index thread to finish. Sent when the window closes.
    Stop,
}

pub enum ThumbJob {
    Thumb {
        id: i64,
        path: String,
        rotation: i32,
    },
    Display {
        id: i64,
        path: String,
        rotation: i32,
        nonce: u64,
    },
    Face {
        id: i64,
        path: String,
        rotation: i32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
}

pub enum UiEvent {
    ScanStarted(String),
    ScanFinished {
        errors: usize,
    },
    Library,
    Thumb {
        id: i64,
        w: usize,
        h: usize,
        rgba: Vec<u8>,
    },
    Display {
        id: i64,
        rotation: i32,
        nonce: u64,
        w: usize,
        h: usize,
        rgba: Vec<u8>,
    },
    FaceThumb {
        id: i64,
        w: usize,
        h: usize,
        rgba: Vec<u8>,
    },
    Faces {
        photo_id: i64,
        count: i64,
    },
}

enum FaceJob {
    Detect(i64),
    Embed(i64),
}

struct ActiveScan {
    folder_id: i64,
    iter: Box<dyn Iterator<Item = walkdir::Result<walkdir::DirEntry>>>,
    seen: Vec<String>,
    ini_cache: HashMap<PathBuf, PicasaIni>,
    errors: usize,
    pending_library: usize,
    again: bool,
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
    let mut detector = match faces::open_detector() {
        Ok(detector) => Some(detector),
        Err(err) => {
            eprintln!("picasso faces: {err}");
            None
        }
    };
    let mut scan: Option<ActiveScan> = None;
    let mut pending: VecDeque<i64> = VecDeque::new();
    let mut faces_q: VecDeque<FaceJob> = VecDeque::new();
    let mut face_ids: HashSet<i64> = HashSet::new();

    loop {
        let mut stop = false;
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) => stop = true,
                Ok(Command::Scan(id)) => queue_scan(&mut scan, &mut pending, id),
                Ok(Command::Redetect(ids)) => {
                    db.reset_detection(&ids);
                    for id in ids {
                        enqueue_face(&mut faces_q, &mut face_ids, FaceJob::Detect(id));
                    }
                    let _ = events.send(UiEvent::Library);
                    ctx.request_repaint();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if stop {
            return;
        }

        if scan.is_none() {
            if let Some(id) = pending.pop_front() {
                if let Some(path) = db.folder_path(id) {
                    let root = PathBuf::from(&path);
                    if root.is_dir() {
                        let _ = events.send(UiEvent::ScanStarted(folder_label(&path)));
                        ctx.request_repaint();
                        scan = Some(ActiveScan {
                            folder_id: id,
                            iter: walk_images(&root),
                            seen: Vec::new(),
                            ini_cache: HashMap::new(),
                            errors: 0,
                            pending_library: 0,
                            again: false,
                        });
                    }
                }
            }
        }

        let mut worked = false;
        if let Some(active) = scan.as_mut() {
            let finished = step_scan(active, &db, &cache, &thumbs, &events, &mut faces_q, &mut face_ids);
            worked = true;
            if active.pending_library > 0 {
                active.pending_library = 0;
                let _ = events.send(UiEvent::Library);
                ctx.request_repaint();
            }
            if finished {
                let errors = active.errors;
                let again = active.again;
                let folder_id = active.folder_id;
                scan = None;
                let _ = events.send(UiEvent::ScanFinished { errors });
                let _ = events.send(UiEvent::Library);
                ctx.request_repaint();
                if again {
                    queue_scan(&mut scan, &mut pending, folder_id);
                }
            }
        }

        if let Some(job) = faces_q.pop_front() {
            let id = face_job_id(&job);
            face_ids.remove(&id);
            run_face(&db, &mut detector, &cache, job, &events);
            let _ = events.send(UiEvent::Faces {
                photo_id: id,
                count: db.photo(id).map(|photo| photo.face_count).unwrap_or(0),
            });
            ctx.request_repaint();
            worked = true;
        }

        if worked {
            continue;
        }
        match commands.recv_timeout(Duration::from_millis(300)) {
            Ok(command) => match command {
                Command::Stop => return,
                Command::Scan(id) => queue_scan(&mut scan, &mut pending, id),
                Command::Redetect(ids) => {
                    db.reset_detection(&ids);
                    for id in ids {
                        enqueue_face(&mut faces_q, &mut face_ids, FaceJob::Detect(id));
                    }
                }
            },
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn queue_scan(scan: &mut Option<ActiveScan>, pending: &mut VecDeque<i64>, id: i64) {
    if let Some(active) = scan.as_mut() {
        if active.folder_id == id {
            active.again = true;
            return;
        }
    }
    if !pending.contains(&id) {
        pending.push_back(id);
    }
}

fn enqueue_face(queue: &mut VecDeque<FaceJob>, ids: &mut HashSet<i64>, job: FaceJob) {
    if ids.insert(face_job_id(&job)) {
        queue.push_back(job);
    }
}

fn face_job_id(job: &FaceJob) -> i64 {
    match job {
        FaceJob::Detect(id) | FaceJob::Embed(id) => *id,
    }
}

fn walk_images(root: &Path) -> Box<dyn Iterator<Item = walkdir::Result<walkdir::DirEntry>>> {
    Box::new(
        WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
            }),
    )
}

fn step_scan(
    scan: &mut ActiveScan,
    db: &Database,
    cache: &Path,
    thumbs: &Sender<ThumbJob>,
    events: &Sender<UiEvent>,
    faces_q: &mut VecDeque<FaceJob>,
    face_ids: &mut HashSet<i64>,
) -> bool {
    let mut checked = 0;
    let mut indexed = 0;
    loop {
        if checked >= 500 || indexed >= 30 {
            break;
        }
        checked += 1;
        let entry = match scan.iter.next() {
            None => {
                finish_scan(scan, db);
                return true;
            }
            Some(Err(_)) => {
                scan.errors += 1;
                continue;
            }
            Some(Ok(entry)) => entry,
        };
        if !entry.file_type().is_file() && !entry.path_is_symlink() {
            continue;
        }
        let path = entry.path();
        if !is_image_path(path) {
            continue;
        }
        let meta = match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => meta,
            _ => continue,
        };
        indexed += 1;
        let path_text = path.to_string_lossy().into_owned();
        scan.seen.push(path_text.clone());
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_text.clone());
        let modified_at = image_io::file_mtime(&meta);
        let probe = match image_io::probe(path, modified_at) {
            Ok(probe) => probe,
            Err(_) => {
                scan.errors += 1;
                continue;
            }
        };
        let record = NewPhoto {
            folder_id: scan.folder_id,
            path: path_text,
            filename: filename.clone(),
            width: probe.width,
            height: probe.height,
            bytes: meta.len(),
            taken_at: if probe.taken_at == 0 { unix_now() } else { probe.taken_at },
            modified_at,
            camera: probe.camera,
        };
        match db.upsert_photo(&record) {
            Ok(Upsert::Unchanged(photo)) => {
                if !photo.faces_scanned {
                    let job = if db.has_manual_faces(photo.id) {
                        FaceJob::Embed(photo.id)
                    } else {
                        FaceJob::Detect(photo.id)
                    };
                    enqueue_face(faces_q, face_ids, job);
                }
            }
            Ok(Upsert::Changed(photo)) => {
                for face_id in db.clear_faces(photo.id) {
                    let _ = std::fs::remove_file(cache.join("faces").join(format!("{face_id}.jpg")));
                }
                let _ = std::fs::remove_file(cache.join("thumbs").join(format!("{}.jpg", photo.id)));
                enqueue_face(faces_q, face_ids, FaceJob::Detect(photo.id));
                let _ = thumbs.send(ThumbJob::Thumb {
                    id: photo.id,
                    path: photo.path.clone(),
                    rotation: photo.rotation,
                });
                scan.pending_library += 1;
            }
            Ok(Upsert::Inserted(id)) => {
                let parent = path.parent().unwrap_or(path);
                let ini = scan
                    .ini_cache
                    .entry(parent.to_path_buf())
                    .or_insert_with(|| PicasaIni::load(parent));
                let meta = ini.get(&filename).cloned();
                apply_new_photo(db, id, meta.as_ref(), faces_q, face_ids);
                let rotation = db.photo(id).map(|photo| photo.rotation).unwrap_or(0);
                let _ = thumbs.send(ThumbJob::Thumb {
                    id,
                    path: record.path,
                    rotation,
                });
                scan.pending_library += 1;
            }
            Err(err) => {
                eprintln!("picasso index: {err}");
                scan.errors += 1;
            }
        }
    }
    let _ = events;
    false
}

fn finish_scan(scan: &mut ActiveScan, db: &Database) {
    let removed = db.remove_missing(scan.folder_id, &scan.seen);
    if !removed.is_empty() {
        scan.pending_library += 1;
    }
}

fn apply_new_photo(
    db: &Database,
    id: i64,
    meta: Option<&IniPhoto>,
    faces_q: &mut VecDeque<FaceJob>,
    face_ids: &mut HashSet<i64>,
) {
    let Some(meta) = meta else {
        enqueue_face(faces_q, face_ids, FaceJob::Detect(id));
        return;
    };
    db.set_user_meta(id, meta.star, meta.caption.as_deref(), meta.rotation);
    match &meta.faces {
        Some(faces) if faces.is_empty() => db.set_faces_scanned(id, true),
        Some(faces) => {
            for face in faces {
                let person = db.person_for_contact(&face.contact, face.name.as_deref());
                db.insert_face(
                    id,
                    &NewFace {
                        person_id: person,
                        rect: face.rect,
                        score: 5.0,
                        embedding: Vec::new(),
                        confirmed: face.name.is_some(),
                        manual: true,
                    },
                );
            }
            enqueue_face(faces_q, face_ids, FaceJob::Embed(id));
        }
        None => enqueue_face(faces_q, face_ids, FaceJob::Detect(id)),
    }
}

fn run_face(
    db: &Database,
    detector: &mut Option<Box<dyn rustface::Detector>>,
    cache: &Path,
    job: FaceJob,
    events: &Sender<UiEvent>,
) {
    let id = face_job_id(&job);
    let Some(photo) = db.photo(id) else {
        return;
    };
    if photo.faces_scanned && matches!(job, FaceJob::Detect(_)) {
        return;
    }
    let image = match image_io::load_displayed(Path::new(&photo.path), photo.rotation) {
        Ok(image) => image,
        Err(err) => {
            eprintln!("picasso decode {}: {err}", photo.filename);
            db.set_faces_scanned(id, true);
            return;
        }
    };
    match job {
        FaceJob::Embed(_) => embed_existing(db, cache, &photo.path, id, &image),
        FaceJob::Detect(_) => {
            let locked = db.faces_for_photo(id);
            let found = detect_catch(detector, &image);
            for detection in found {
                if locked.iter().any(|face| iou(face.rect(), detection.rect) > 0.45) {
                    continue;
                }
                store_detection(db, cache, id, &image, detection);
            }
            embed_existing(db, cache, &photo.path, id, &image);
        }
    }
    let _ = events;
}

fn embed_existing(db: &Database, cache: &Path, _path: &str, photo_id: i64, image: &image::DynamicImage) {
    let faces = db.faces_for_photo(photo_id);
    for face in &faces {
        if db.embedding(face.id).is_none() {
            let emb = faces::embedding(image, face.rect());
            db.set_embedding(face.id, &emb);
            db.recompute_centroid(face.person_id);
        }
        write_crop(cache, image, face.id, face.rect());
    }
    db.set_faces_scanned(photo_id, true);
}

fn store_detection(db: &Database, cache: &Path, photo_id: i64, image: &image::DynamicImage, detection: Detection) {
    let person_id = db.assign_person(&detection.embedding);
    let face_id = db.insert_face(
        photo_id,
        &NewFace {
            person_id,
            rect: detection.rect,
            score: detection.score,
            embedding: detection.embedding,
            confirmed: false,
            manual: false,
        },
    );
    write_crop(cache, image, face_id, detection.rect);
}

fn write_crop(cache: &Path, image: &image::DynamicImage, face_id: i64, rect: RelRect) {
    let path = cache.join("faces").join(format!("{face_id}.jpg"));
    if path.is_file() {
        return;
    }
    let crop = face_crop(image, rect, 160);
    let _ = image_io::save_jpeg(&path, &crop, 85);
}

fn detect_catch(
    slot: &mut Option<Box<dyn rustface::Detector>>,
    image: &image::DynamicImage,
) -> Vec<Detection> {
    let Some(detector) = slot.as_mut() else {
        return Vec::new();
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| faces::detect(&mut **detector, image)));
    match outcome {
        Ok(found) => found,
        Err(_) => {
            eprintln!("picasso: face detector reset after a fault");
            *slot = faces::open_detector().ok();
            Vec::new()
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
    gate.lock().ok().and_then(|guard| guard.get(id).copied()) != Some(*nonce)
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
        ThumbJob::Thumb { id, path, rotation } => {
            let cached = cache.join("thumbs").join(format!("{id}.jpg"));
            let rgba = if let Some(rgba) = open_rgba(&cached) {
                rgba
            } else {
                let image = image_io::load_displayed(Path::new(&path), rotation).ok()?;
                let rgba = image_io::thumbnail_rgba(&image, 256);
                let _ = image_io::save_jpeg(&cached, &rgba, 80);
                rgba
            };
            let (w, h, rgba) = raw_rgba(&rgba);
            Some(UiEvent::Thumb { id, w, h, rgba })
        }
        ThumbJob::Display {
            id,
            path,
            rotation,
            nonce,
        } => {
            if skip_stale(&ThumbJob::Display { id, path: path.clone(), rotation, nonce }, gate) {
                return None;
            }
            let image = image_io::load_displayed(Path::new(&path), rotation).ok()?;
            if skip_stale(&ThumbJob::Display { id, path, rotation, nonce }, gate) {
                return None;
            }
            let rgba = image_io::limit_rgba(image, 2560);
            let (w, h, rgba) = raw_rgba(&rgba);
            Some(UiEvent::Display {
                id,
                rotation,
                nonce,
                w,
                h,
                rgba,
            })
        }
        ThumbJob::Face {
            id,
            path,
            rotation,
            x,
            y,
            w,
            h,
        } => {
            let cached = cache.join("faces").join(format!("{id}.jpg"));
            let rgba = if let Some(rgba) = open_rgba(&cached) {
                rgba
            } else {
                let image = image_io::load_displayed(Path::new(&path), rotation).ok()?;
                let rgba = face_crop(
                    &image,
                    RelRect { x, y, w, h },
                    160,
                );
                let _ = image_io::save_jpeg(&cached, &rgba, 85);
                rgba
            };
            let (w, h, rgba) = raw_rgba(&rgba);
            Some(UiEvent::FaceThumb { id, w, h, rgba })
        }
    }
}

fn open_rgba(path: &Path) -> Option<RgbaImage> {
    image::open(path).ok().map(|image| image.to_rgba8())
}

fn raw_rgba(image: &RgbaImage) -> (usize, usize, Vec<u8>) {
    (
        image.width() as usize,
        image.height() as usize,
        image.as_raw().clone(),
    )
}
