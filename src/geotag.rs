//! Geotagging from a GPS track: each photo in a folder, and in the folders
//! below it, gets the place the track was at when the photo was taken. The
//! places go to the library and to a `.picasso-geotags.json` file in each
//! folder; the photo files stay as they are. Photos whose camera recorded GPS
//! keep it, and so do photos pinned by hand.
//!
//! GPS tracks keep time in UTC, while a camera's clock shows the local time
//! where it was set. A photo's time is turned into UTC with the offset given,
//! else the offset the photo records (`OffsetTimeOriginal`), else this
//! computer's time zone. Videos already record UTC, and so does a photo
//! without a capture time, which goes by its file's modification time.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use serde_json::{json, Map, Value};

use crate::db::Database;
use crate::i18n::{tr, trf};
use crate::image_io;
use crate::track::Track;
use crate::types::{format_utc_offset, is_media_path, is_video_path, parse_utc_offset, Nav, Photo, Sort};

/// The file in each folder that holds the places matched from tracks.
pub const SIDECAR: &str = ".picasso-geotags.json";
/// Track files the geotagger reads, lower case.
pub const TRACK_EXTENSIONS: [&str; 3] = ["gpx", "kml", "kmz"];
/// Longest stretch between two track points that a photo is placed along.
/// Tracks thinned by an app keep a point only where the path turns, and
/// one while standing still, so points can be many minutes apart. The
/// limits match ExifTool's.
const MAX_GAP: i64 = 30 * 60;
/// Farthest in time a photo may be from a track point and still take its
/// place, past the ends of the track or in a longer gap.
pub const NEAR: i64 = 30 * 60;

pub fn is_geotag_track(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| TRACK_EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

pub struct Options {
    pub track: PathBuf,
    pub folder: PathBuf,
    /// Seconds the camera's clock was ahead of UTC; `None` uses what each
    /// photo records, else this computer's time zone.
    pub offset: Option<i32>,
    /// Works out the places without writing anything.
    pub dry_run: bool,
}

/// What became of one photo.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub path: PathBuf,
    /// When it was taken, in UTC.
    pub time: i64,
    pub place: Option<(f64, f64)>,
    /// Why the photo was left alone, if it was.
    pub kept: Option<Kept>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kept {
    /// The camera recorded GPS.
    Gps,
    /// It was pinned by hand, to a place no track gave it.
    Pin,
}

/// One folder's geotag file after a run.
#[derive(Clone, Debug)]
pub struct Written {
    pub path: PathBuf,
    /// Places from this track it holds.
    pub count: usize,
    /// No places were left, so the file was removed.
    pub removed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub track_name: String,
    /// First and last times on the track, in UTC.
    pub track_span: (i64, i64),
    /// Photos and videos looked at.
    pub photos: usize,
    pub tagged: usize,
    pub had_gps: usize,
    /// Photos pinned by hand, which keep their pin.
    pub hand_pinned: usize,
    /// Photos without GPS that no track point was near.
    pub unmatched: usize,
    pub unreadable: usize,
    /// Photos without GPS that recorded no offset and were given this
    /// computer's time zone, with no offset typed.
    pub zone_guessed: usize,
    /// First and last times, in UTC, of the photos without GPS.
    pub photo_span: Option<(i64, i64)>,
    /// Geotag files written or removed.
    pub sidecars: Vec<Written>,
    /// Folders whose geotag file couldn't be written, with the reason.
    pub failed: Vec<(PathBuf, String)>,
    /// Photos pinned in the library.
    pub library: usize,
    /// Why the library couldn't be updated.
    pub library_error: Option<String>,
    pub matches: Vec<Match>,
}

/// The track's timed points, per segment and in time order.
struct Timeline {
    segments: Vec<Vec<(i64, f64, f64)>>,
}

impl Timeline {
    fn new(track: &Track) -> Option<Timeline> {
        let segments: Vec<Vec<(i64, f64, f64)>> = track
            .segments
            .iter()
            .map(|segment| {
                let mut points: Vec<(i64, f64, f64)> =
                    segment.iter().filter_map(|point| Some((point.time?, point.lat, point.lon))).collect();
                points.sort_by_key(|point| point.0);
                points
            })
            .filter(|points| !points.is_empty())
            .collect();
        (!segments.is_empty()).then_some(Timeline { segments })
    }

    fn span(&self) -> (i64, i64) {
        let first = self.segments.iter().map(|segment| segment[0].0).min().unwrap_or(0);
        let last = self.segments.iter().filter_map(|segment| segment.last()).map(|point| point.0).max().unwrap_or(0);
        (first, last)
    }

    /// Where the track was at `time`: along the line between the points
    /// around it when they are at most [`MAX_GAP`] apart, otherwise at the
    /// nearest point within [`NEAR`].
    fn locate(&self, time: i64) -> Option<(f64, f64)> {
        let mut nearest: Option<(i64, f64, f64)> = None;
        for segment in &self.segments {
            let (first, last) = (segment[0].0, segment[segment.len() - 1].0);
            if time < first - NEAR || time > last + NEAR {
                continue;
            }
            let after = segment.partition_point(|point| point.0 <= time);
            if after > 0 && after < segment.len() {
                let (a, b) = (segment[after - 1], segment[after]);
                if b.0 - a.0 <= MAX_GAP {
                    let share = if b.0 == a.0 { 0.0 } else { (time - a.0) as f64 / (b.0 - a.0) as f64 };
                    return Some((a.1 + (b.1 - a.1) * share, a.2 + (b.2 - a.2) * share));
                }
            }
            for index in [after.checked_sub(1), (after < segment.len()).then_some(after)].into_iter().flatten() {
                let point = segment[index];
                let apart = (time - point.0).abs();
                if apart <= NEAR && nearest.is_none_or(|best| apart < best.0) {
                    nearest = Some((apart, point.1, point.2));
                }
            }
        }
        nearest.map(|(_, lat, lon)| (lat, lon))
    }
}

/// UTC time of a photo whose clock read `taken_at`, as the library keeps it:
/// the clock's reading taken as a time on this computer. `zone` is how far
/// the clock was ahead of UTC; without it the computer's zone stands.
fn utc_time(taken_at: i64, zone: Option<i32>) -> i64 {
    let Some(zone) = zone else { return taken_at };
    let Some(time) = chrono::DateTime::from_timestamp(taken_at, 0) else { return taken_at };
    time.with_timezone(&chrono::Local).naive_local().and_utc().timestamp() - i64::from(zone)
}

/// `2026-10-11T08:01:02Z`.
fn iso_time(time: i64) -> String {
    chrono::DateTime::from_timestamp(time, 0)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Photos and videos in `dir` and the folders below it, skipping hidden ones.
fn media_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => media_files(&path, out),
            Ok(_) if is_media_path(&path) && path.is_file() => out.push(path),
            _ => {}
        }
    }
}

/// The places in one folder's geotag file, by file name.
#[derive(Clone, Debug, Default)]
pub struct Sidecar {
    entries: Map<String, Value>,
}

impl Sidecar {
    pub fn load(dir: &Path) -> Sidecar {
        let text = std::fs::read_to_string(dir.join(SIDECAR)).unwrap_or_default();
        let entries = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|doc| doc.get("photos")?.as_object().cloned())
            .unwrap_or_default();
        Sidecar { entries }
    }

    /// The place of `filename`, matched without regard to case.
    pub fn place(&self, filename: &str) -> Option<(f64, f64)> {
        let entry = self
            .entries
            .get(filename)
            .or_else(|| self.entries.iter().find(|(name, _)| name.eq_ignore_ascii_case(filename)).map(|(_, entry)| entry))?;
        entry_place(entry)
    }

    /// Writes the file, or removes it once no places are left. The text goes
    /// to a temporary name first, so a reader never sees half a file.
    fn save(&self, dir: &Path) -> Result<(), String> {
        let path = dir.join(SIDECAR);
        if self.entries.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.to_string()),
                _ => Ok(()),
            };
        }
        let doc = json!({
            "about": "Places Picasso matched from GPS tracks, by file name. The photo files are unchanged.",
            "photos": self.entries,
        });
        let text = serde_json::to_string_pretty(&doc).map_err(|err| err.to_string())?;
        let temporary = dir.join(format!("{SIDECAR}.{}.tmp", std::process::id()));
        std::fs::write(&temporary, text + "\n").map_err(|err| err.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|err| {
            let _ = std::fs::remove_file(&temporary);
            err.to_string()
        })
    }
}

fn entry_place(entry: &Value) -> Option<(f64, f64)> {
    let (lat, lon) = (entry.get("lat")?.as_f64()?, entry.get("lon")?.as_f64()?);
    (lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0).then_some((lat, lon))
}

fn same_place(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

/// One file as the geotagger sees it.
struct Seen {
    path: PathBuf,
    /// Its row in the library and the pin it has there.
    id: Option<i64>,
    pin: Option<(f64, f64)>,
    /// When it was taken in UTC; `None` when the file couldn't be read.
    time: Option<i64>,
    zone: Option<i32>,
    /// The time came from a video or the file's modification time, so no time zone applies.
    exact: bool,
    had_gps: bool,
}

/// Matches the photos under `options.folder` to the track. Photos in
/// `db` are read from it and pinned there; `progress` hears how many files
/// of how many were read.
pub fn run(options: &Options, db: Option<&Database>, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<Report, String> {
    let track = Track::read(&options.track)?;
    let timeline = Timeline::new(&track).ok_or_else(|| tr("The track has no times, so photos can't be matched to it").to_string())?;
    let folder = options.folder.canonicalize().map_err(|err| trf("Can't open folder: {}", &[&err]))?;
    if !folder.is_dir() {
        return Err(tr("That path is not a folder").into());
    }
    let track_file = options.track.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();

    let mut files = Vec::new();
    media_files(&folder, &mut files);
    files.sort();
    let library: HashMap<String, Photo> = db
        .map(|db| db.photos(&Nav::Dir(folder.to_string_lossy().into_owned()), "", Sort::Name))
        .unwrap_or_default()
        .into_iter()
        .map(|photo| (photo.path.clone(), photo))
        .collect();
    let (total, done) = (files.len(), AtomicUsize::new(0));
    progress(0, total);
    let seen: Vec<Seen> = files
        .par_iter()
        .map(|path| {
            let known = library.get(path.to_string_lossy().as_ref());
            let video = is_video_path(path);
            let modified_at = std::fs::metadata(path).map(|meta| image_io::file_mtime(&meta)).ok();
            let probe = (known.is_none() || (options.offset.is_none() && !video))
                .then(|| image_io::probe(path, modified_at?).ok())
                .flatten();
            progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
            let taken_at = known.map(|photo| photo.taken_at).or_else(|| probe.as_ref().map(|probe| probe.taken_at));
            // Without a recorded capture time the library falls back to the file's modification time,
            // which already is an exact moment; only a camera clock needs its time zone.
            let clock = !video
                && taken_at.is_some_and(|taken_at| Some(taken_at) != known.map(|photo| photo.modified_at).or(modified_at));
            let zone = if clock { options.offset.or_else(|| probe.as_ref().and_then(|probe| probe.zone)) } else { None };
            Seen {
                path: path.clone(),
                id: known.map(|photo| photo.id),
                pin: known.and_then(|photo| photo.pin_lat.zip(photo.pin_lon)),
                time: taken_at.map(|taken_at| if clock { utc_time(taken_at, zone) } else { taken_at }),
                zone,
                exact: !clock,
                had_gps: known.is_some_and(|photo| photo.gps_lat.is_some())
                    || probe.as_ref().is_some_and(|probe| probe.gps_lat.is_some()),
            }
        })
        .collect();

    let mut report = Report {
        track_name: track.name.clone(),
        track_span: timeline.span(),
        photos: total,
        ..Report::default()
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    for item in &seen {
        let dir = item.path.parent().unwrap_or(&folder).to_path_buf();
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let mut sidecars: HashMap<PathBuf, Sidecar> = dirs.iter().map(|dir| (dir.clone(), Sidecar::load(dir))).collect();
    let mut found: HashMap<PathBuf, Vec<(String, Value)>> = HashMap::new();
    let mut pins = Vec::new();
    let mut by_path: HashMap<&Path, &Seen> = HashMap::new();
    for item in &seen {
        by_path.insert(item.path.as_path(), item);
        let Some(time) = item.time else {
            report.unreadable += 1;
            continue;
        };
        let dir = item.path.parent().unwrap_or(&folder).to_path_buf();
        let name = item.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        // A pin that no geotag file explains was placed by hand.
        let by_hand = item.pin.is_some_and(|pin| !sidecars[&dir].place(&name).is_some_and(|place| same_place(place, pin)));
        let kept = if item.had_gps { Some(Kept::Gps) } else { by_hand.then_some(Kept::Pin) };
        let place = if kept.is_some() { None } else { timeline.locate(time) };
        report.matches.push(Match { path: item.path.clone(), time, place, kept });
        match kept {
            Some(Kept::Gps) => {
                report.had_gps += 1;
                continue;
            }
            Some(Kept::Pin) => {
                report.hand_pinned += 1;
                continue;
            }
            None => {}
        }
        report.photo_span = Some(report.photo_span.map_or((time, time), |(first, last)| (first.min(time), last.max(time))));
        if item.zone.is_none() && !item.exact {
            report.zone_guessed += 1;
        }
        let Some((lat, lon)) = place else {
            report.unmatched += 1;
            continue;
        };
        report.tagged += 1;
        let (lat, lon) = ((lat * 1e7).round() / 1e7, (lon * 1e7).round() / 1e7);
        let mut entry = json!({ "lat": lat, "lon": lon, "time": iso_time(time), "track": track_file });
        if let Some(zone) = item.zone {
            entry["offset"] = Value::String(format_utc_offset(zone));
        }
        found.entry(dir).or_default().push((name, entry));
        if let Some(id) = item.id {
            pins.push((id, lat, lon));
        }
    }

    // Each folder's file keeps the places from other tracks; those from this
    // track are replaced, so a run with a corrected offset drops stale ones.
    // A folder that can't be written is listed, and the others go on.
    let mut cleared = Vec::new();
    for dir in &dirs {
        let Some(mut sidecar) = sidecars.remove(dir) else { continue };
        let new = found.remove(dir).unwrap_or_default();
        let names: HashSet<&str> = new.iter().map(|(name, _)| name.as_str()).collect();
        let stale: Vec<String> = sidecar
            .entries
            .iter()
            .filter(|(name, entry)| entry.get("track").and_then(Value::as_str) == Some(track_file.as_str()) && !names.contains(name.as_str()))
            .map(|(name, _)| name.clone())
            .collect();
        if new.is_empty() && stale.is_empty() {
            continue;
        }
        for name in &stale {
            let old = sidecar.entries.remove(name).as_ref().and_then(entry_place);
            let item = by_path.get(dir.join(name).as_path());
            if let (Some(old), Some(item)) = (old, item) {
                if let (Some(id), Some(pin)) = (item.id, item.pin) {
                    if same_place(old, pin) {
                        cleared.push(id);
                    }
                }
            }
        }
        let count = new.len();
        for (name, entry) in new {
            sidecar.entries.insert(name, entry);
        }
        let path = dir.join(SIDECAR);
        if !options.dry_run {
            if let Err(err) = sidecar.save(dir) {
                report.failed.push((path, err));
                continue;
            }
        }
        report.sidecars.push(Written { path, count, removed: sidecar.entries.is_empty() });
    }
    report.library = pins.len();
    if let Some(db) = db.filter(|_| !options.dry_run) {
        if let Err(err) = db.set_pin_each(&pins, &cleared) {
            report.library = 0;
            report.library_error = Some(err);
        }
    }
    Ok(report)
}

/// Track files in `folder` or the folder above it, the folder's own first.
pub fn tracks_near(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in [Some(folder), folder.parent()].into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut here: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| is_geotag_track(path) && path.is_file()).collect();
        here.sort();
        found.extend(here);
    }
    found
}

const USAGE: &str = "\
Usage: picasso geotag <track> <folder> [--offset <zone>] [--dry-run]

Matches the photos in <folder>, and in the folders below it, to the GPS
track <track> (.gpx, .kml, or .kmz) by the time each photo was taken. The
places go to a .picasso-geotags.json file in each folder and to the Picasso
library; the photo files are not changed. Photos with GPS from the camera,
and photos pinned by hand, keep their place.

Options:
  -o, --offset <zone>  The camera clock's offset from UTC, such as +01:00 or
                       -05:00. Add seconds for a clock that was off:
                       +01:02:30. Without it, the offset each photo records
                       is used, else this computer's time zone.
  -n, --dry-run        List each photo's match and write nothing.
  -h, --help           Show this help.";

/// `picasso geotag …`; returns the exit code.
pub fn cli(args: &[PathBuf]) -> i32 {
    attach_console();
    let mut positional = Vec::new();
    let (mut offset, mut dry_run) = (None, false);
    let mut args = args.iter().map(|arg| arg.to_string_lossy().into_owned());
    while let Some(arg) = args.next() {
        let value = match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "-n" | "--dry-run" => {
                dry_run = true;
                continue;
            }
            "-o" | "--offset" => args.next(),
            _ if arg.starts_with("--offset=") => Some(arg["--offset=".len()..].to_string()),
            _ if arg.starts_with('-') => {
                eprintln!("Unknown option {arg}\n\n{USAGE}");
                return 2;
            }
            _ => {
                positional.push(PathBuf::from(&arg));
                continue;
            }
        };
        match value.as_deref().and_then(parse_utc_offset) {
            Some(seconds) => offset = Some(seconds),
            None => {
                eprintln!("The offset should look like +01:00 or -05:30.");
                return 2;
            }
        }
    }
    let [track, folder] = positional.as_slice() else {
        eprintln!("{USAGE}");
        return 2;
    };
    let (data_dir, _) = crate::db::app_dirs();
    let library = data_dir.join("picasso.db");
    let db = if library.exists() { Database::open(&library).ok() } else { None };
    let options = Options { track: track.clone(), folder: folder.clone(), offset, dry_run };
    let progress = |done: usize, total: usize| {
        if done % 50 == 0 || done == total {
            eprint!("\rReading {done} of {total} files");
        }
    };
    let report = match run(&options, db.as_ref(), &progress) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("\n{err}");
            return 1;
        }
    };
    eprintln!();
    let utc = |time: i64| {
        chrono::DateTime::from_timestamp(time, 0).map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default()
    };
    println!("Track: {} ({} – {} UTC)", report.track_name, utc(report.track_span.0), utc(report.track_span.1));
    if let Some((first, last)) = report.photo_span {
        println!("Photos without GPS were taken {} – {} UTC", utc(first), utc(last));
    }
    if dry_run {
        for item in &report.matches {
            let what = match (item.kept, item.place) {
                (Some(Kept::Gps), _) => "has GPS from the camera".to_string(),
                (Some(Kept::Pin), _) => "pinned by hand".to_string(),
                (None, Some((lat, lon))) => format!("{lat:.6}, {lon:.6}"),
                (None, None) => "no track point near".to_string(),
            };
            println!("  {}  {}  {what}", utc(item.time), shown(&item.path));
        }
    }
    println!(
        "{} files: {} tagged, {} already had GPS, {}{} not near the track{}",
        report.photos,
        report.tagged,
        report.had_gps,
        if report.hand_pinned > 0 { format!("{} pinned by hand, ", report.hand_pinned) } else { String::new() },
        report.unmatched,
        if report.unreadable > 0 { format!(", {} unreadable", report.unreadable) } else { String::new() }
    );
    if dry_run {
        println!("Dry run: nothing was written.");
        return 0;
    }
    for written in &report.sidecars {
        if written.removed {
            println!("Removed {} (no places left)", shown(&written.path));
        } else {
            println!("Wrote {} ({} from this track)", shown(&written.path), written.count);
        }
    }
    for (path, err) in &report.failed {
        println!("Couldn't write {}: {err}", shown(path));
    }
    match (&db, &report.library_error) {
        (_, Some(err)) => println!("The Picasso library wasn't updated: {err}"),
        (Some(_), None) => println!("Pinned {} photos in the Picasso library.", report.library),
        (None, None) => println!("No Picasso library yet; it reads the places when the folder is added."),
    }
    if report.zone_guessed > 0 {
        println!(
            "{} photos record no time zone, so this computer's ({}) was used. If they were taken elsewhere, run again on their folder with --offset.",
            report.zone_guessed,
            local_offset()
        );
    }
    if report.tagged == 0 && report.unmatched > 0 {
        println!("No photo fell within the track. Compare the times above and try --offset.");
    }
    0
}

/// This computer's offset from UTC now, as `+08:00`.
pub fn local_offset() -> String {
    use chrono::Offset;
    format_utc_offset(chrono::Local::now().offset().fix().local_minus_utc())
}

fn shown(path: &Path) -> String {
    path.display().to_string().trim_start_matches(r"\\?\").to_string()
}

/// A release build on Windows has no console of its own; the command line
/// output goes to the console it was started from.
#[cfg(windows)]
fn attach_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: AttachConsole takes a plain process id and fails harmlessly
    // when there is no console or one is attached already.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::Point;
    use chrono::TimeZone;

    fn point(time: i64, lat: f64, lon: f64) -> Point {
        Point { lat, lon, time: Some(time) }
    }

    #[test]
    fn places_photos_along_the_track() {
        let track = Track {
            name: "run".into(),
            segments: vec![
                vec![point(1000, 10.0, 20.0), point(1060, 11.0, 22.0), point(5000, 12.0, 24.0)],
                vec![point(9000, 30.0, 40.0), Point { lat: 0.0, lon: 0.0, time: None }, point(9010, 31.0, 41.0)],
            ],
        };
        let line = Timeline::new(&track).unwrap();
        assert_eq!(line.span(), (1000, 9010));
        assert_eq!(line.locate(1030), Some((10.5, 21.0)));
        assert_eq!(line.locate(1000), Some((10.0, 20.0)));
        assert_eq!(line.locate(9010), Some((31.0, 41.0)));
        // A long gap: only near its ends.
        assert_eq!(line.locate(1100), Some((11.0, 22.0)));
        assert_eq!(line.locate(3000), None);
        assert_eq!(line.locate(4900), Some((12.0, 24.0)));
        // Past the ends, within NEAR.
        assert_eq!(line.locate(1000 - NEAR), Some((10.0, 20.0)));
        assert_eq!(line.locate(1000 - NEAR - 1), None);
        assert_eq!(line.locate(9010 + 60), Some((31.0, 41.0)));
        assert!(Timeline::new(&Track { name: String::new(), segments: vec![vec![Point { lat: 1.0, lon: 2.0, time: None }]] }).is_none());
    }

    #[test]
    fn camera_clocks_turn_into_utc() {
        let clock = chrono::NaiveDate::from_ymd_opt(2026, 10, 11).unwrap().and_hms_opt(10, 0, 0).unwrap();
        let taken_at = chrono::Local.from_local_datetime(&clock).single().unwrap().timestamp();
        let utc = clock.and_utc().timestamp();
        assert_eq!(utc_time(taken_at, Some(3600)), utc - 3600);
        assert_eq!(utc_time(taken_at, Some(-5 * 3600 - 150)), utc + 5 * 3600 + 150);
        assert_eq!(utc_time(taken_at, None), taken_at);
        assert_eq!(iso_time(utc), "2026-10-11T10:00:00Z");
    }

    fn library(name: &str) -> (Database, i64, PathBuf) {
        let dir = std::env::temp_dir().join(format!("picasso-geotag-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Database::open(&dir.join("picasso.db")).unwrap();
        let photos = dir.join("photos");
        std::fs::create_dir_all(photos.join("day")).unwrap();
        let folder_id = db.add_folder(&photos).unwrap();
        (db, folder_id, photos.canonicalize().unwrap())
    }

    /// A library photo `name` under `folder`, taken when the camera's clock
    /// read 10:00 plus `minutes` on 2026-10-11, with GPS when `gps`.
    fn add(db: &Database, folder_id: i64, folder: &Path, name: &str, minutes: u32, gps: bool) -> i64 {
        let clock = chrono::NaiveDate::from_ymd_opt(2026, 10, 11).unwrap().and_hms_opt(10, minutes, 0).unwrap();
        let taken_at = chrono::Local.from_local_datetime(&clock).single().unwrap().timestamp();
        add_at(db, folder_id, folder, name, taken_at, taken_at + 86_400, gps)
    }

    /// A library photo `name` under `folder` with the given times.
    fn add_at(db: &Database, folder_id: i64, folder: &Path, name: &str, taken_at: i64, modified_at: i64, gps: bool) -> i64 {
        let path = folder.join(name);
        std::fs::write(&path, b"not decoded").unwrap();
        let record = crate::db::ScanRecord {
            photo: crate::types::NewPhoto {
                folder_id,
                path: path.to_string_lossy().into_owned(),
                filename: name.rsplit(['/', '\\']).next().unwrap().to_string(),
                width: 1,
                height: 1,
                bytes: 11,
                taken_at,
                modified_at,
                camera: String::new(),
                lens: String::new(),
                focal_mm: 0.0,
                gps_lat: gps.then_some(1.0),
                gps_lon: gps.then_some(2.0),
                duration: 0.0,
            },
            existing: None,
            star: None,
            caption: None,
            rotation: None,
            pin: None,
            complete: true,
        };
        db.write_batch(&[record]).unwrap().ids[0]
    }

    #[test]
    fn tags_a_folder_and_writes_the_places_beside_the_photos() {
        let (db, folder_id, folder) = library("run");
        let a = add(&db, folder_id, &folder, "a.jpg", 0, false);
        let b = add(&db, folder_id, &folder, &format!("day{}b.jpg", std::path::MAIN_SEPARATOR), 1, false);
        let phone = add(&db, folder_id, &folder, "phone.jpg", 0, true);
        let late = add(&db, folder_id, &folder, "late.jpg", 50, false);
        let hand = add(&db, folder_id, &folder, "hand.jpg", 0, false);
        db.set_pins(&[hand], 1.0, 2.0);
        // The camera was set to UTC+1: 10:00 on its clock is 09:00 UTC.
        let start = chrono::Utc.with_ymd_and_hms(2026, 10, 11, 9, 0, 0).unwrap().timestamp();
        // No capture time: the library keeps the file's modification time, an exact moment.
        let saved = add_at(&db, folder_id, &folder, "saved.jpg", start + 60, start + 60, false);
        let gpx = format!(
            r#"<gpx><trk><name>Lisbon</name><trkseg>
                <trkpt lat="38.70" lon="-9.10"><time>{}</time></trkpt>
                <trkpt lat="38.72" lon="-9.12"><time>{}</time></trkpt>
            </trkseg></trk></gpx>"#,
            iso_time(start),
            iso_time(start + 120)
        );
        let track = folder.parent().unwrap().join("run.gpx");
        std::fs::write(&track, gpx).unwrap();
        let options = Options { track: track.clone(), folder: folder.clone(), offset: Some(3600), dry_run: true };
        let report = run(&options, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.photos, report.tagged, report.had_gps, report.hand_pinned, report.unmatched), (6, 3, 1, 1, 1));
        assert_eq!(report.zone_guessed, 0, "the offset was typed");
        assert_eq!(report.track_name, "Lisbon");
        assert!(!folder.join(SIDECAR).exists(), "a dry run writes nothing");
        assert_eq!(db.photo(a).unwrap().pin_lat, None);

        let report = run(&Options { dry_run: false, ..options }, Some(&db), &|_, _| {}).unwrap();
        assert_eq!(report.library, 3);
        let pin = |id| db.photo(id).and_then(|photo| photo.pin_lat.zip(photo.pin_lon));
        assert_eq!(pin(a), Some((38.70, -9.10)));
        let (lat, lon) = pin(b).unwrap();
        assert!((lat - 38.71).abs() < 1e-9 && (lon + 9.11).abs() < 1e-9);
        assert_eq!(pin(phone), None);
        assert_eq!(pin(late), None);
        assert_eq!(pin(saved), pin(b), "the offset leaves modification times alone");
        assert_eq!(pin(hand), Some((1.0, 2.0)), "a hand pin stays");
        assert_eq!(Sidecar::load(&folder).place("A.JPG"), Some((38.70, -9.10)));
        assert!(Sidecar::load(&folder.join("day")).place("b.jpg").is_some());
        let text = std::fs::read_to_string(folder.join(SIDECAR)).unwrap();
        assert!(text.contains("\"offset\": \"+01:00\"") && text.contains("\"track\": \"run.gpx\"") && !text.contains("phone.jpg"));

        // A wrong offset misses the camera photos, and drops this track's old places for them.
        let wrong = Options { track: track.clone(), folder: folder.clone(), offset: Some(5 * 3600), dry_run: false };
        let report = run(&wrong, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.tagged, report.unmatched), (1, 3), "the track's own pins are no hand pins");
        assert!(!folder.join("day").join(SIDECAR).exists());
        assert!(report.sidecars.iter().any(|written| written.removed && written.path == folder.join("day").join(SIDECAR)));
        assert_eq!((pin(a), Sidecar::load(&folder).place("a.jpg")), (None, None));
        assert!(pin(saved).is_some());

        // A folder whose file can't be written is listed; the rest goes on.
        std::fs::create_dir_all(folder.join("card").join(SIDECAR)).unwrap();
        let card = add(&db, folder_id, &folder, &format!("card{}c.jpg", std::path::MAIN_SEPARATOR), 0, false);
        let right = Options { track, folder: folder.clone(), offset: Some(3600), dry_run: false };
        let report = run(&right, Some(&db), &|_, _| {}).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, folder.join("card").join(SIDECAR));
        assert_eq!((report.library, report.library_error.as_deref()), (4, None));
        assert!(pin(card).is_some() && folder.join("day").join(SIDECAR).is_file());
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
    }

    #[test]
    fn finds_tracks_beside_the_folder() {
        let dir = std::env::temp_dir().join(format!("picasso-geotag-near-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("photos")).unwrap();
        for name in ["b.KMZ", "a.gpx", "photos/c.kml", "notes.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let names: Vec<String> = tracks_near(&dir.join("photos"))
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["c.kml", "a.gpx", "b.KMZ"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
