//! Repeatable timings for large libraries, run with `make bench`. They are
//! ignored by `cargo test`. Sizes come from the environment:
//!
//! - `PICASSO_BENCH_PHOTOS` (default 100000): catalog rows for the queries.
//! - `PICASSO_BENCH_FILES` (default 2000): photo files for the scan and
//!   thumbnail timings, made once and kept in `target/bench-photos-<n>`.
//! - `PICASSO_BENCH_CHECK=1` fails the run when a result is past its limit.
//!   The limits are loose enough for a slow CI machine and catch only large
//!   regressions; `make bench-check` runs a small library with them.
//!
//! The scan goes through the indexer the app uses. Files read since the
//! computer started are in the system's file cache, so a second run measures
//! a warm cache. For a cold one, empty the cache first: `sudo purge` on
//! macOS, `sync; echo 3 | sudo tee /proc/sys/vm/drop_caches` on Linux.
//! `docs/PERFORMANCE.md` explains the numbers and keeps the baselines.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use image::{Rgba, RgbaImage};
use rayon::prelude::*;

use crate::db::{Database, ScanRecord};
use crate::image_io;
use crate::types::{Nav, NewPhoto, Sort};
use crate::worker::{self, Command, UiEvent};

fn size(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("picasso-bench-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Report {
    check: bool,
    failures: Vec<String>,
}

impl Report {
    fn time<T>(&mut self, label: &str, run: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let out = run();
        self.add(label, format_duration(started.elapsed()));
        out
    }

    /// [`Report::time`] that fails a checked run when it takes longer than `limit`.
    fn time_within<T>(&mut self, label: &str, limit: Duration, run: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let out = run();
        let elapsed = started.elapsed();
        self.add(label, format_duration(elapsed));
        self.limit(label, elapsed > limit, format!("took {}, the limit is {}", format_duration(elapsed), format_duration(limit)));
        out
    }

    fn rate(&mut self, label: &str, count: usize, elapsed: Duration, at_least: f64) {
        let rate = count as f64 / elapsed.as_secs_f64();
        self.add(label, format!("{} ({rate:.0} photos/s)", format_duration(elapsed)));
        self.limit(label, rate < at_least, format!("{rate:.0} photos/s, the limit is {at_least:.0}"));
    }

    fn memory(&mut self, label: &str, at_most_mb: f64) {
        if let Some(peak) = peak_memory_mb() {
            self.add(label, format!("{peak:.0} MB"));
            self.limit(label, peak > at_most_mb, format!("{peak:.0} MB, the limit is {at_most_mb:.0} MB"));
        }
    }

    fn limit(&mut self, label: &str, over: bool, detail: String) {
        if self.check && over {
            self.failures.push(format!("{label}: {detail}"));
        }
    }

    fn add(&mut self, label: &str, value: String) {
        println!("{label:<44} {value}");
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.as_secs_f64() >= 1.0 {
        format!("{:.2} s", duration.as_secs_f64())
    } else {
        format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
    }
}

/// Most memory the process has held, in MB.
fn peak_memory_mb() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: getrusage fills the struct it is given.
        let usage = unsafe {
            if libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) != 0 {
                return None;
            }
            usage.assume_init()
        };
        let bytes = if cfg!(target_os = "macos") { usage.ru_maxrss as f64 } else { usage.ru_maxrss as f64 * 1024.0 };
        Some(bytes / 1_048_576.0)
    }
    #[cfg(not(unix))]
    None
}

/// `count` photos spread over folders of 50, as a camera import makes them.
fn record(folder_id: i64, root: &Path, index: usize) -> ScanRecord {
    let dir = root.join(format!("{}", 2000 + index / 20_000)).join(format!("{:04}", index / 50));
    let filename = format!("IMG_{index:06}.jpg");
    let words = ["beach", "city", "mountain", "family", "street"];
    ScanRecord {
        photo: NewPhoto {
            folder_id,
            path: dir.join(&filename).to_string_lossy().into_owned(),
            filename,
            width: 6000,
            height: 4000,
            bytes: 8_000_000 + index as u64,
            taken_at: 946_684_800 + index as i64 * 3_600,
            modified_at: 946_684_800 + index as i64 * 3_600,
            camera: format!("Camera {} {}", index % 7, words[index % words.len()]),
            lens: String::new(),
            focal_mm: 35.0,
            gps_lat: index.is_multiple_of(3).then_some(25.0 + (index % 1000) as f64 / 1000.0),
            gps_lon: index.is_multiple_of(3).then_some(121.0),
            duration: 0.0,
        },
        existing: None,
        star: index.is_multiple_of(10).then_some(true),
        caption: None,
        rotation: None,
        pin: None,
        complete: true,
    }
}

fn catalog(report: &mut Report, count: usize) {
    let dir = scratch("catalog");
    let path = dir.join("picasso.db");
    let root = dir.join("photos");
    std::fs::create_dir_all(&root).unwrap();
    let db = Database::open(&path).unwrap();
    let folder_id = db.add_folder(&root).unwrap();
    let root = root.canonicalize().unwrap();
    report.time(&format!("Write {count} catalog rows"), || {
        for start in (0..count).step_by(1000) {
            let batch: Vec<ScanRecord> = (start..(start + 1000).min(count)).map(|index| record(folder_id, &root, index)).collect();
            db.write_batch(&batch).unwrap();
        }
    });
    drop(db);
    let second = Duration::from_secs(1);
    let db = report.time_within("Open the library (startup)", second, || Database::open(&path).unwrap());
    let reader = Database::open_reader(&path).unwrap();
    let photos = report.time_within("List the library, newest first", second, || reader.photos(&Nav::Library, "", Sort::DateNewest));
    assert_eq!(photos.len(), count);
    report.time_within("List the library by name", second, || reader.photos(&Nav::Library, "", Sort::Name));
    report.time_within("List starred photos", second, || reader.photos(&Nav::Starred, "", Sort::DateNewest));
    report.time_within("Folder tree (photo_dirs)", second, || reader.photo_dirs());
    let dir_path = root.join("2000").join("0001").to_string_lossy().into_owned();
    report.time_within("Open one folder of 50", second, || reader.photos(&Nav::Dir(dir_path), "", Sort::DateNewest));
    report.time_within("Search one word (\"beach\")", second, || reader.photos(&Nav::Library, "beach", Sort::DateNewest));
    report.time_within("Search a month (\"2001-05\")", second, || reader.photos(&Nav::Library, "2001-05", Sort::DateNewest));
    report.time_within("Search with no match", second, || reader.photos(&Nav::Library, "zzzz", Sort::DateNewest));
    report.time_within("Rescan check: known files of the folder", second, || db.folder_snapshot(folder_id, &root.to_string_lossy()));
    report.time_within("Duplicate candidates", second, || db.duplicate_candidates());
    let bytes: u64 = ["", "-wal"].iter().map(|suffix| std::fs::metadata(format!("{}{suffix}", path.display())).map_or(0, |m| m.len())).sum();
    report.add("Library file size", format!("{:.1} MB", bytes as f64 / 1_048_576.0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Photo files kept between runs, so each run reads the same files.
fn photo_files(count: usize) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("bench-photos-{count}"));
    let done = root.join(".complete");
    if done.exists() {
        return root;
    }
    let _ = std::fs::remove_dir_all(&root);
    (0..count).into_par_iter().for_each(|index| {
        let dir = root.join(format!("{:03}", index / 100));
        std::fs::create_dir_all(&dir).unwrap();
        let shade = (index % 200) as u8;
        let image = RgbaImage::from_fn(1600, 1200, |x, y| Rgba([(x / 7) as u8 ^ shade, (y / 5) as u8, shade, 255]));
        std::fs::write(dir.join(format!("IMG_{index:05}.jpg")), image_io::encode_jpeg(&image, 85).unwrap()).unwrap();
    });
    std::fs::write(done, b"").unwrap();
    root
}

fn wait_for_scan(events: &crossbeam_channel::Receiver<UiEvent>) {
    loop {
        match events.recv_timeout(Duration::from_secs(600)) {
            Ok(UiEvent::ScanFinished { folder, .. }) if folder != 0 => return,
            Ok(_) => {}
            Err(err) => panic!("the scan didn't finish: {err}"),
        }
    }
}

fn scan(report: &mut Report, count: usize) {
    let photos = report.time(&format!("Make {count} test photos (first run only)"), || photo_files(count));
    let dir = scratch("scan");
    let db_path = dir.join("picasso.db");
    let cache = dir.join("cache");
    let folder_id = Database::open(&db_path).unwrap().add_folder(&photos).unwrap();
    let (commands, command_rx) = crossbeam_channel::unbounded();
    let (thumbs, _thumb_rx) = crossbeam_channel::unbounded();
    let (events_tx, events) = crossbeam_channel::unbounded();
    let ctx = eframe::egui::Context::default();
    let started = Instant::now();
    let indexer = std::thread::spawn(move || worker::index_loop(db_path, cache, command_rx, thumbs, events_tx, ctx));
    wait_for_scan(&events);
    let first = started.elapsed();
    let reader = Database::open_reader(&dir.join("picasso.db")).unwrap();
    assert_eq!(reader.photos(&Nav::Library, "", Sort::DateNewest).len(), count, "the scan missed photos");
    report.rate(&format!("First scan of {count} photos"), count, first, 200.0);
    let started = Instant::now();
    commands.send(Command::Scan(folder_id)).unwrap();
    wait_for_scan(&events);
    report.add("Rescan with nothing changed", format_duration(started.elapsed()));
    commands.send(Command::Stop).unwrap();
    let _ = indexer.join();
    report.memory("Peak memory after the scan", 400.0);

    let files: Vec<PathBuf> = walk(&photos);
    // Plain threads taking the next file, as the app's thumbnail workers do.
    let threads = worker::thumb_threads();
    let next = AtomicUsize::new(0);
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let tile = image_io::thumbnail_rgba(&image_io::load_for_tile(path, worker::THUMB_EDGE).unwrap(), worker::THUMB_EDGE);
                    image_io::encode_jpeg(&tile, 80).unwrap();
                }
            });
        }
    });
    report.rate(&format!("Thumbnails for {} photos on {threads} threads", files.len()), files.len(), started.elapsed(), 20.0);
    let one = &files[0];
    report.time_within("Open one 1600×1200 photo for the viewer", Duration::from_millis(500), || {
        image_io::load_at_least(one, worker::DISPLAY_EDGE).unwrap()
    });
    let _ = std::fs::remove_dir_all(&dir);
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|ext| ext == "jpg") {
            files.push(path);
        }
    }
    files
}

#[test]
#[ignore = "a benchmark: run it with `make bench`"]
fn bench_large_library() {
    let mut report = Report { check: std::env::var("PICASSO_BENCH_CHECK").is_ok_and(|value| value == "1"), failures: Vec::new() };
    println!(
        "Picasso {} on {} {}, {} cores",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    catalog(&mut report, size("PICASSO_BENCH_PHOTOS", 100_000));
    report.memory("Peak memory after the catalog", 400.0);
    scan(&mut report, size("PICASSO_BENCH_FILES", 2_000));
    // Making thumbnails once held on to about 1 MB per photo; this limit
    // catches that coming back.
    report.memory("Peak memory after scan and thumbnails", 400.0);
    assert!(report.failures.is_empty(), "past the limits:\n{}", report.failures.join("\n"));
}
