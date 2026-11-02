//! Geotagging from GPS tracks: each photo in a folder, and in the folders
//! below it, or each of a Trip's photos, gets the place the tracks were at
//! when the photo was taken. Photos with GPS, such as a phone's, can stand
//! in for a track. The places go to the library and to a
//! `.picasso-geotags.json` file in each folder; the photo files stay as they
//! are. Photos whose camera recorded GPS keep it, and so do photos pinned by
//! hand.
//!
//! GPS tracks keep time in UTC, while a camera's clock shows the local time
//! where it was set. A photo's time is turned into UTC with the offset given,
//! else the offset the photo records (`OffsetTimeOriginal`), else this
//! computer's time zone. Videos already record UTC, and so does a photo
//! without a capture time, which goes by its file's modification time.

use std::cmp::Reverse;
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

/// Where the places come from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// GPS track files, read as one: one per day, say.
    Tracks(Vec<PathBuf>),
    /// Photos with GPS from the camera, such as a phone's: those being
    /// geotagged and the library's others taken within a day of them. Each
    /// marks where it was taken, and the photos between take places along
    /// the line from one to the next, as along a track.
    Photos,
}

/// The photos to geotag.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A folder and the folders below it.
    Folder(PathBuf),
    /// These files, such as a Trip's photos.
    Files(Vec<PathBuf>),
}

#[derive(Clone)]
pub struct Options {
    pub source: Source,
    pub target: Target,
    /// Seconds the camera's clock was ahead of UTC; `None` uses what each
    /// photo records, else this computer's time zone.
    pub offset: Option<i32>,
    /// Works out the places without writing anything.
    pub dry_run: bool,
}

/// The source a geotag file names for places taken from photos with GPS.
pub const FROM_PHOTOS: &str = "photos with GPS";

/// A camera time zone that places more photos than the one used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Suggestion {
    /// Seconds ahead of UTC.
    pub offset: i32,
    /// Photos it places, of the `of` read from a camera clock.
    pub places: usize,
    pub of: usize,
    /// How many of them the zone used placed.
    pub now: usize,
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
    /// Places matched in this run it holds.
    pub count: usize,
    /// No places were left, so the file was removed.
    pub removed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    /// The tracks' names, by commas; empty when the places came from photos.
    pub track_name: String,
    /// First and last times on the tracks, or of the photos with GPS, in UTC.
    pub track_span: (i64, i64),
    /// Photos with GPS whose places the others took.
    pub gps_photos: usize,
    /// A camera time zone that would place more photos.
    pub suggestion: Option<Suggestion>,
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

/// Which track a segment came from, and its points as (time, lat, lon).
type Segment = (usize, Vec<(i64, f64, f64)>);

/// The tracks' timed points, per segment and in time order. Each segment
/// knows which of the tracks it came from.
pub(crate) struct Timeline {
    segments: Vec<Segment>,
}

impl Timeline {
    pub(crate) fn new(track: &Track) -> Option<Timeline> {
        Timeline::of(std::slice::from_ref(track))
    }

    fn of(tracks: &[Track]) -> Option<Timeline> {
        let segments: Vec<Segment> = tracks
            .iter()
            .enumerate()
            .flat_map(|(source, track)| {
                track.segments.iter().map(move |segment| {
                    let mut points: Vec<(i64, f64, f64)> =
                        segment.iter().filter_map(|point| Some((point.time?, point.lat, point.lon))).collect();
                    points.sort_by_key(|point| point.0);
                    (source, points)
                })
            })
            .filter(|(_, points)| !points.is_empty())
            .collect();
        (!segments.is_empty()).then_some(Timeline { segments })
    }

    fn of_points(mut points: Vec<(i64, f64, f64)>) -> Option<Timeline> {
        points.sort_by_key(|point| point.0);
        (!points.is_empty()).then(|| Timeline { segments: vec![(0, points)] })
    }

    fn span(&self) -> (i64, i64) {
        let first = self.segments.iter().map(|(_, segment)| segment[0].0).min().unwrap_or(0);
        let last = self.segments.iter().filter_map(|(_, segment)| segment.last()).map(|point| point.0).max().unwrap_or(0);
        (first, last)
    }

    /// Where the track was at `time`: along the line between the points
    /// around it when they are at most [`MAX_GAP`] apart, otherwise at the
    /// nearest point within [`NEAR`].
    pub(crate) fn locate(&self, time: i64) -> Option<(f64, f64)> {
        self.find(time).map(|(place, _, _)| place)
    }

    /// [`Timeline::locate`], with the index of the track the place came
    /// from and the seconds to the point taken, 0 along a line.
    fn find(&self, time: i64) -> Option<((f64, f64), usize, i64)> {
        let mut nearest: Option<(i64, (f64, f64), usize)> = None;
        for (source, segment) in &self.segments {
            let (first, last) = (segment[0].0, segment[segment.len() - 1].0);
            if time < first - NEAR || time > last + NEAR {
                continue;
            }
            let after = segment.partition_point(|point| point.0 <= time);
            if after > 0 && after < segment.len() {
                let (a, b) = (segment[after - 1], segment[after]);
                if b.0 - a.0 <= MAX_GAP {
                    let share = if b.0 == a.0 { 0.0 } else { (time - a.0) as f64 / (b.0 - a.0) as f64 };
                    return Some(((a.1 + (b.1 - a.1) * share, a.2 + (b.2 - a.2) * share), *source, 0));
                }
            }
            for index in [after.checked_sub(1), (after < segment.len()).then_some(after)].into_iter().flatten() {
                let point = segment[index];
                let apart = (time - point.0).abs();
                if apart <= NEAR && nearest.is_none_or(|best| apart < best.0) {
                    nearest = Some((apart, (point.1, point.2), *source));
                }
            }
        }
        nearest.map(|(apart, place, source)| (place, source, apart))
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

    /// Moves the places of files renamed in `dir` from the first name of each
    /// pair to the second. A folder without a geotag file is left as it is.
    pub fn rename(dir: &Path, renames: &[(String, String)]) -> Result<(), String> {
        let mut sidecar = Sidecar::load(dir);
        let mut moved = Vec::new();
        for (from, to) in renames {
            let key = match sidecar.entries.contains_key(from) {
                true => Some(from.clone()),
                false => sidecar.entries.keys().find(|name| name.eq_ignore_ascii_case(from)).cloned(),
            };
            if let Some(entry) = key.and_then(|key| sidecar.entries.remove(&key)) {
                moved.push((to.clone(), entry));
            }
        }
        if moved.is_empty() {
            return Ok(());
        }
        sidecar.entries.extend(moved);
        sidecar.save(dir)
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
    /// The time zone it was read in, and the one the photo records.
    zone: Option<i32>,
    recorded: Option<i32>,
    /// The camera clock's reading, as [`Photo::taken_at`] keeps it; `None`
    /// when the time came from a video or the file's modification time, so
    /// no time zone applies.
    clock: Option<i64>,
    had_gps: bool,
    /// When it was taken in UTC, by the offset the photo records, and its
    /// GPS from the camera.
    gps: Option<(i64, f64, f64)>,
}

/// The file name of `path`, as geotag files name their tracks.
fn file_label(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The photos and videos `target` names, and the folders to find them in
/// the library by.
fn target_files(target: &Target) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    match target {
        Target::Folder(folder) => {
            let folder = folder.canonicalize().map_err(|err| trf("Can't open folder: {}", &[&err]))?;
            if !folder.is_dir() {
                return Err(tr("That path is not a folder").into());
            }
            let mut files = Vec::new();
            media_files(&folder, &mut files);
            files.sort();
            Ok((files, vec![folder]))
        }
        Target::Files(paths) => {
            let mut files: Vec<PathBuf> = paths.iter().filter(|path| is_media_path(path) && path.is_file()).cloned().collect();
            files.sort();
            files.dedup();
            if files.is_empty() {
                return Err(tr("None of the photos can be found").into());
            }
            let mut dirs: Vec<PathBuf> = files.iter().filter_map(|path| path.parent().map(Path::to_path_buf)).collect();
            dirs.sort();
            dirs.dedup();
            Ok((files, dirs))
        }
    }
}

/// Track points from photos with GPS: those in `seen`, and the library's
/// others taken within a day of them, each at its time in UTC.
fn photo_points(seen: &[Seen], db: Option<&Database>) -> Vec<(i64, f64, f64)> {
    const DAY: i64 = 24 * 60 * 60;
    let mut points: Vec<(i64, f64, f64)> = seen.iter().filter_map(|item| item.gps).collect();
    let times = seen.iter().filter_map(|item| item.time);
    let (Some(db), Some(first), Some(last)) = (db, times.clone().min(), times.max()) else { return points };
    let ids: HashSet<i64> = seen.iter().filter_map(|item| item.id).collect();
    let others: Vec<Photo> = db.gps_photos_between(first - DAY, last + DAY).into_iter().filter(|photo| !ids.contains(&photo.id)).collect();
    points.par_extend(others.par_iter().filter_map(|photo| {
        let (lat, lon) = photo.gps_lat.zip(photo.gps_lon)?;
        let path = Path::new(&photo.path);
        let clock = !is_video_path(path) && photo.taken_at != photo.modified_at;
        if !clock {
            return Some((photo.taken_at, lat, lon));
        }
        let zone = image_io::probe(path, photo.modified_at).ok().and_then(|probe| probe.zone);
        Some((utc_time(photo.taken_at, zone), lat, lon))
    }));
    points
}

/// A camera time zone for `clocks`, camera clock readings with the zone
/// each photo records, that places more of them than the `now` placed with
/// the zone `typed`. When they all record one zone, that one, if it places
/// more. Else the zone in use from −12:00 to +14:00 that puts them nearest
/// the tracks' points, when it places at least two and a tenth more, so
/// chance doesn't name a zone; of zones that fit as well, the one nearest
/// `typed`, else the zone most photos record, else this computer's.
fn suggest(timeline: &Timeline, clocks: &[(i64, Option<i32>)], now: usize, typed: Option<i32>) -> Option<Suggestion> {
    if clocks.is_empty() || now == clocks.len() {
        return None;
    }
    let readings: Vec<i64> = clocks.iter().map(|(clock, _)| utc_time(*clock, Some(0))).collect();
    let score = |offset: i32| {
        let found: Vec<i64> =
            readings.iter().filter_map(|reading| timeline.find(reading - i64::from(offset))).map(|(_, _, apart)| apart).collect();
        // A photo placed far from the track's points, past its ends, counts for less than one along it.
        (found.len(), found.iter().map(|apart| 2 * NEAR - apart).sum::<i64>())
    };
    let suggestion = |offset, places| Suggestion { offset, places, of: clocks.len(), now };
    if let Some(Some(zone)) = clocks.iter().map(|(_, zone)| *zone).reduce(|one, other| one.filter(|_| one == other)) {
        let (places, _) = score(zone);
        if typed != Some(zone) && places > now {
            return Some(suggestion(zone, places));
        }
    }
    let mut recorded: HashMap<i32, usize> = HashMap::new();
    for zone in clocks.iter().filter_map(|(_, zone)| *zone) {
        *recorded.entry(zone).or_default() += 1;
    }
    let usual = recorded.into_iter().max_by_key(|(zone, count)| (*count, -zone.abs())).map(|(zone, _)| zone);
    let target = typed.or(usual).unwrap_or_else(local_offset_seconds);
    (-12..=14)
        .map(|hours: i32| hours * 60)
        .chain(ZONES_OFF_THE_HOUR)
        .map(|minutes| minutes * 60)
        .map(|offset| (offset, score(offset)))
        .max_by_key(|(offset, (_, fit))| (*fit, Reverse((offset - target).abs())))
        .filter(|(_, (places, _))| *places >= now + 2 && *places * 10 >= now * 11)
        .map(|(offset, (places, _))| suggestion(offset, places))
}

/// The UTC offsets, in minutes, that places keep off the whole hour, with
/// summer time: India, Nepal, Iran, Newfoundland, central Australia,
/// Chatham Islands and others.
const ZONES_OFF_THE_HOUR: [i32; 13] = [-570, -210, -150, 210, 270, 330, 345, 390, 525, 570, 630, 765, 825];

/// Matches the photos `options.target` names to the tracks, or to the
/// photos with GPS. Photos in `db` are read from it and pinned there;
/// `progress` hears how many files of how many were read.
pub fn run(options: &Options, db: Option<&Database>, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<Report, String> {
    let no_times = || tr("The track has no times, so photos can't be matched to it").to_string();
    let (tracks, labels) = match &options.source {
        Source::Tracks(paths) if paths.is_empty() => return Err(tr("Choose a GPS track").into()),
        Source::Tracks(paths) => {
            let tracks = paths
                .iter()
                .map(|path| Track::read(path).map_err(|err| format!("{}: {err}", file_label(path))))
                .collect::<Result<Vec<Track>, String>>()?;
            (tracks, paths.iter().map(|path| file_label(path)).collect::<Vec<String>>())
        }
        Source::Photos => (Vec::new(), vec![FROM_PHOTOS.to_string()]),
    };
    let from_tracks = match options.source {
        Source::Tracks(_) => Some(Timeline::of(&tracks).ok_or_else(no_times)?),
        Source::Photos => None,
    };
    let (files, roots) = target_files(&options.target)?;
    let library: HashMap<String, Photo> = db
        .map(|db| roots.iter().flat_map(|root| db.photos(&Nav::Dir(root.to_string_lossy().into_owned()), "", Sort::Name)).collect::<Vec<Photo>>())
        .unwrap_or_default()
        .into_iter()
        .map(|photo| (photo.path.clone(), photo))
        .collect();
    let from_photos = options.source == Source::Photos;
    let (total, done) = (files.len(), AtomicUsize::new(0));
    progress(0, total);
    let seen: Vec<Seen> = files
        .par_iter()
        .map(|path| {
            let known = library.get(path.to_string_lossy().as_ref());
            let video = is_video_path(path);
            let modified_at = std::fs::metadata(path).map(|meta| image_io::file_mtime(&meta)).ok();
            let known_gps = known.and_then(|photo| photo.gps_lat.zip(photo.gps_lon));
            // The recorded offset times a photo with GPS standing for a track point, and tells
            // whether a typed offset is the one the camera recorded.
            let probe = (known.is_none() || !video)
                .then(|| image_io::probe(path, modified_at?).ok())
                .flatten();
            progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
            let taken_at = known.map(|photo| photo.taken_at).or_else(|| probe.as_ref().map(|probe| probe.taken_at));
            // Without a recorded capture time the library falls back to the file's modification time,
            // which already is an exact moment; only a camera clock needs its time zone.
            let clock = !video
                && taken_at.is_some_and(|taken_at| Some(taken_at) != known.map(|photo| photo.modified_at).or(modified_at));
            let recorded = probe.as_ref().and_then(|probe| probe.zone);
            let zone = if clock { options.offset.or(recorded) } else { None };
            let gps = known_gps.or_else(|| probe.as_ref().and_then(|probe| probe.gps_lat.zip(probe.gps_lon)));
            let at = |zone: Option<i32>| taken_at.map(|taken_at| if clock { utc_time(taken_at, zone) } else { taken_at });
            Seen {
                path: path.clone(),
                id: known.map(|photo| photo.id),
                pin: known.and_then(|photo| photo.pin_lat.zip(photo.pin_lon)),
                time: at(zone),
                zone,
                recorded,
                clock: taken_at.filter(|_| clock),
                had_gps: gps.is_some(),
                gps: at(recorded).zip(gps).map(|(time, (lat, lon))| (time, lat, lon)),
            }
        })
        .collect();
    let timeline = match from_tracks {
        Some(timeline) => timeline,
        None => Timeline::of_points(photo_points(&seen, db)).ok_or_else(|| {
            tr("None of these photos has GPS, and no other photo in the library taken within a day of them has").to_string()
        })?,
    };

    let mut report = Report {
        track_name: tracks
            .iter()
            .zip(&labels)
            .map(|(track, label)| if track.name.trim().is_empty() { label.clone() } else { track.name.clone() })
            .collect::<Vec<String>>()
            .join(", "),
        track_span: timeline.span(),
        gps_photos: if from_photos { timeline.segments.iter().map(|(_, points)| points.len()).sum() } else { 0 },
        photos: total,
        ..Report::default()
    };
    let parent = |path: &Path| path.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for item in &seen {
        let dir = parent(&item.path);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let mut sidecars: HashMap<PathBuf, Sidecar> = dirs.iter().map(|dir| (dir.clone(), Sidecar::load(dir))).collect();
    let mut found: HashMap<PathBuf, Vec<(String, Value)>> = HashMap::new();
    let mut pins = Vec::new();
    let mut by_path: HashMap<&Path, &Seen> = HashMap::new();
    let (mut clocks, mut clocks_placed, mut zones_recorded) = (Vec::new(), 0, true);
    for item in &seen {
        by_path.insert(item.path.as_path(), item);
        let Some(time) = item.time else {
            report.unreadable += 1;
            continue;
        };
        let dir = parent(&item.path);
        let name = file_label(&item.path);
        // A pin that no geotag file explains was placed by hand.
        let by_hand = item.pin.is_some_and(|pin| !sidecars[&dir].place(&name).is_some_and(|place| same_place(place, pin)));
        let kept = if item.had_gps { Some(Kept::Gps) } else { by_hand.then_some(Kept::Pin) };
        let located = if kept.is_some() { None } else { timeline.find(time) };
        let place = located.map(|(place, _, _)| place);
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
        if let Some(clock) = item.clock {
            clocks.push((clock, item.recorded));
            clocks_placed += usize::from(place.is_some());
            zones_recorded &= item.recorded.is_some() && item.zone == item.recorded;
        }
        if item.zone.is_none() && item.clock.is_some() {
            report.zone_guessed += 1;
        }
        let Some(((lat, lon), source, _)) = located else {
            report.unmatched += 1;
            continue;
        };
        report.tagged += 1;
        let (lat, lon) = ((lat * 1e7).round() / 1e7, (lon * 1e7).round() / 1e7);
        let mut entry = json!({ "lat": lat, "lon": lon, "time": iso_time(time), "track": labels[source] });
        if let Some(zone) = item.zone {
            entry["offset"] = Value::String(format_utc_offset(zone));
        }
        found.entry(dir).or_default().push((name, entry));
        if let Some(id) = item.id {
            pins.push((id, lat, lon));
        }
    }
    // Photos read in the zone they record were taken at exact moments, which no other zone corrects.
    if !zones_recorded {
        report.suggestion = suggest(&timeline, &clocks, clocks_placed, options.offset);
    }

    // Each folder's file keeps the places from other tracks; those from
    // these tracks are replaced, so a run with a corrected offset drops
    // stale ones. A folder that can't be written is listed, and the others go on.
    let mut cleared = Vec::new();
    for dir in &dirs {
        let Some(mut sidecar) = sidecars.remove(dir) else { continue };
        let new = found.remove(dir).unwrap_or_default();
        let names: HashSet<&str> = new.iter().map(|(name, _)| name.as_str()).collect();
        let stale: Vec<String> = sidecar
            .entries
            .iter()
            .filter(|(name, entry)| {
                let track = entry.get("track").and_then(Value::as_str);
                track.is_some_and(|track| labels.iter().any(|label| label == track)) && !names.contains(name.as_str())
            })
            .map(|(name, _)| name.clone())
            .collect();
        // A Trip's photos are only some of the folder's: the others keep their places.
        let stale: Vec<String> = stale.into_iter().filter(|name| by_path.contains_key(dir.join(name).as_path())).collect();
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
Usage: picasso geotag <track>... <folder> [--offset <zone>] [--dry-run]
       picasso geotag --from-photos <folder> [--offset <zone>] [--dry-run]

Matches the photos in <folder>, and in the folders below it, to GPS tracks
(.gpx, .kml, or .kmz) by the time each photo was taken. Several tracks,
one per day say, are read as one. The places go to a .picasso-geotags.json
file in each folder and to the Picasso library; the photo files are not
changed. Photos with GPS from the camera, and photos pinned by hand, keep
their place.

Options:
  -p, --from-photos    Use the photos that have GPS, such as a phone's, as
                       the track: those in <folder> and the library's
                       others taken within a day of them.
  -o, --offset <zone>  The camera clock's offset from UTC, such as +01:00 or
                       -05:00. Add seconds for a clock that was off:
                       +01:02:30. Without it, the offset each photo records
                       is used, else this computer's time zone.
  -n, --dry-run        List each photo's match and write nothing.
  -h, --help           Show this help.";

/// `picasso geotag …`; returns the exit code.
pub fn cli(args: &[PathBuf]) -> i32 {
    crate::attach_console();
    let mut positional = Vec::new();
    let (mut offset, mut dry_run, mut from_photos) = (None, false, false);
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
            "-p" | "--from-photos" => {
                from_photos = true;
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
    let Some((folder, tracks)) = positional.split_last().filter(|(_, tracks)| tracks.is_empty() == from_photos) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let (data_dir, _) = crate::db::app_dirs();
    let library = data_dir.join("picasso.db");
    let db = if library.exists() { Database::open(&library).ok() } else { None };
    let source = if from_photos { Source::Photos } else { Source::Tracks(tracks.to_vec()) };
    let options = Options { source, target: Target::Folder(folder.clone()), offset, dry_run };
    let progress = |done: usize, total: usize| {
        if done.is_multiple_of(50) || done == total {
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
    let (first, last) = (utc(report.track_span.0), utc(report.track_span.1));
    match from_photos {
        true => println!("Places from {} photos with GPS ({first} – {last} UTC)", report.gps_photos),
        false => println!("Track: {} ({first} – {last} UTC)", report.track_name),
    }
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
    if let Some(better) = report.suggestion {
        println!(
            "--offset {} would place {} of the {} photos read from a camera clock, not {}.",
            format_utc_offset(better.offset),
            better.places,
            better.of,
            better.now
        );
    } else if report.tagged == 0 && report.unmatched > 0 {
        println!("No photo fell within the track. Compare the times above and try --offset.");
    }
    if dry_run {
        println!("Dry run: nothing was written.");
        return 0;
    }
    for written in &report.sidecars {
        if written.removed {
            println!("Removed {} (no places left)", shown(&written.path));
        } else {
            println!("Wrote {} ({} matched now)", shown(&written.path), written.count);
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
    0
}

/// This computer's offset from UTC now, as `+08:00`.
pub fn local_offset() -> String {
    format_utc_offset(local_offset_seconds())
}

fn local_offset_seconds() -> i32 {
    use chrono::Offset;
    chrono::Local::now().offset().fix().local_minus_utc()
}

fn shown(path: &Path) -> String {
    path.display().to_string().trim_start_matches(r"\\?\").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::Point;
    use chrono::TimeZone;

    #[test]
    fn places_follow_renamed_files() {
        let dir = std::env::temp_dir().join(format!("picasso-sidecar-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut sidecar = Sidecar::default();
        sidecar.entries.insert("A.JPG".into(), json!({ "lat": 1.0, "lon": 2.0 }));
        sidecar.entries.insert("b.jpg".into(), json!({ "lat": 3.0, "lon": 4.0 }));
        sidecar.save(&dir).unwrap();
        let pair = |from: &str, to: &str| (from.to_string(), to.to_string());
        Sidecar::rename(&dir, &[pair("a.jpg", "b.jpg"), pair("b.jpg", "c.jpg")]).unwrap();
        let sidecar = Sidecar::load(&dir);
        assert_eq!(sidecar.place("b.jpg"), Some((1.0, 2.0)));
        assert_eq!(sidecar.place("c.jpg"), Some((3.0, 4.0)));
        assert_eq!(sidecar.place("a.jpg"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        add_at(db, folder_id, folder, name, taken_at, taken_at + 86_400, gps.then_some((1.0, 2.0)))
    }

    /// When the camera's clock read `hour`:`minute` on 2026-10-`day`, as the library keeps it.
    fn clock(day: u32, hour: u32, minute: u32) -> i64 {
        let clock = chrono::NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(hour, minute, 0).unwrap();
        chrono::Local.from_local_datetime(&clock).single().unwrap().timestamp()
    }

    /// A library photo `name` under `folder` with the given times.
    fn add_at(db: &Database, folder_id: i64, folder: &Path, name: &str, taken_at: i64, modified_at: i64, gps: Option<(f64, f64)>) -> i64 {
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
                gps_lat: gps.map(|(lat, _)| lat),
                gps_lon: gps.map(|(_, lon)| lon),
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
        let saved = add_at(&db, folder_id, &folder, "saved.jpg", start + 60, start + 60, None);
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
        let options = Options {
            source: Source::Tracks(vec![track.clone()]),
            target: Target::Folder(folder.clone()),
            offset: Some(3600),
            dry_run: true,
        };
        let report = run(&options, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.photos, report.tagged, report.had_gps, report.hand_pinned, report.unmatched), (6, 3, 1, 1, 1));
        assert_eq!(report.zone_guessed, 0, "the offset was typed");
        assert_eq!(report.track_name, "Lisbon");
        assert!(!folder.join(SIDECAR).exists(), "a dry run writes nothing");
        assert_eq!(db.photo(a).unwrap().pin_lat, None);

        let report = run(&Options { dry_run: false, ..options.clone() }, Some(&db), &|_, _| {}).unwrap();
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
        let wrong = Options { offset: Some(5 * 3600), dry_run: false, ..options };
        let report = run(&wrong, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.tagged, report.unmatched), (1, 3), "the track's own pins are no hand pins");
        assert_eq!(report.suggestion, Some(Suggestion { offset: 3600, places: 2, of: 3, now: 0 }));
        assert!(!folder.join("day").join(SIDECAR).exists());
        assert!(report.sidecars.iter().any(|written| written.removed && written.path == folder.join("day").join(SIDECAR)));
        assert_eq!((pin(a), Sidecar::load(&folder).place("a.jpg")), (None, None));
        assert!(pin(saved).is_some());

        // A folder whose file can't be written is listed; the rest goes on.
        std::fs::create_dir_all(folder.join("card").join(SIDECAR)).unwrap();
        let card = add(&db, folder_id, &folder, &format!("card{}c.jpg", std::path::MAIN_SEPARATOR), 0, false);
        let right = Options { offset: Some(3600), ..wrong };
        let report = run(&right, Some(&db), &|_, _| {}).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, folder.join("card").join(SIDECAR));
        assert_eq!((report.library, report.library_error.as_deref()), (4, None));
        assert!(pin(card).is_some() && folder.join("day").join(SIDECAR).is_file());
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
    }

    fn gpx(name: &str, points: &[(i64, f64, f64)]) -> String {
        let points: String = points
            .iter()
            .map(|(time, lat, lon)| format!(r#"<trkpt lat="{lat}" lon="{lon}"><time>{}</time></trkpt>"#, iso_time(*time)))
            .collect();
        format!("<gpx><trk><name>{name}</name><trkseg>{points}</trkseg></trk></gpx>")
    }

    #[test]
    fn reads_several_tracks_and_suggests_the_camera_time_zone() {
        let (db, folder_id, folder) = library("tracks");
        let a = add_at(&db, folder_id, &folder, "a.jpg", clock(11, 10, 0), 0, None);
        let other = add_at(&db, folder_id, &folder, "other.jpg", clock(11, 10, 1), 0, None);
        let b = add_at(&db, folder_id, &folder, &format!("day{}b.jpg", std::path::MAIN_SEPARATOR), clock(12, 10, 1), 0, None);
        let utc = |day, minute| chrono::Utc.with_ymd_and_hms(2026, 10, day, 9, minute, 0).unwrap().timestamp();
        let first = folder.parent().unwrap().join("first.gpx");
        let second = folder.parent().unwrap().join("second.gpx");
        std::fs::write(&first, gpx("Lisbon", &[(utc(11, 0), 38.70, -9.10), (utc(11, 2), 38.72, -9.12)])).unwrap();
        std::fs::write(&second, gpx("", &[(utc(12, 0), 40.0, -8.0), (utc(12, 2), 40.02, -8.02)])).unwrap();
        let options = Options {
            source: Source::Tracks(vec![first.clone(), second.clone()]),
            target: Target::Folder(folder.clone()),
            offset: Some(5 * 3600),
            dry_run: true,
        };
        // A clock read in the wrong zone places nothing; the right one is suggested,
        // and of the zones placing as many, the one nearest the track points.
        let report = run(&options, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.tagged, report.track_name.as_str()), (0, "Lisbon, second"));
        assert_eq!(report.suggestion, Some(Suggestion { offset: 3600, places: 3, of: 3, now: 0 }));

        let report = run(&Options { offset: Some(3600), dry_run: false, ..options }, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.tagged, report.suggestion), (3, None));
        let pin = |id| db.photo(id).and_then(|photo| photo.pin_lat.zip(photo.pin_lon));
        assert_eq!(pin(a), Some((38.70, -9.10)));
        assert_eq!(pin(b), Some((40.01, -8.01)));
        let text = std::fs::read_to_string(folder.join("day").join(SIDECAR)).unwrap();
        assert!(text.contains("\"track\": \"second.gpx\""), "{text}");

        // A Trip's photos are only some of the folder's: the others keep their places.
        let only_a = Options {
            source: Source::Tracks(vec![first, second]),
            target: Target::Files(vec![folder.join("a.jpg")]),
            offset: Some(5 * 3600),
            dry_run: false,
        };
        let report = run(&only_a, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.photos, report.tagged, report.unmatched), (1, 0, 1));
        assert_eq!((pin(a), Sidecar::load(&folder).place("a.jpg")), (None, None));
        assert!(pin(other).is_some() && Sidecar::load(&folder).place("other.jpg").is_some());
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
    }

    #[test]
    fn photos_with_gps_stand_in_for_a_track() {
        let (db, folder_id, folder) = library("phone");
        // The phone's photos are in another folder; the camera's are in day/.
        add_at(&db, folder_id, &folder, "phone1.jpg", clock(11, 10, 0), 0, Some((10.0, 20.0)));
        add_at(&db, folder_id, &folder, "phone2.jpg", clock(11, 10, 20), 0, Some((12.0, 24.0)));
        let day = folder.join("day");
        let camera = add_at(&db, folder_id, &folder, &format!("day{}camera.jpg", std::path::MAIN_SEPARATOR), clock(11, 10, 10), 0, None);
        let late = add_at(&db, folder_id, &folder, &format!("day{}late.jpg", std::path::MAIN_SEPARATOR), clock(11, 12, 0), 0, None);
        let options = Options { source: Source::Photos, target: Target::Folder(day.clone()), offset: None, dry_run: false };
        let report = run(&options, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.gps_photos, report.tagged, report.unmatched), (2, 1, 1));
        let pin = |id| db.photo(id).and_then(|photo| photo.pin_lat.zip(photo.pin_lon));
        assert_eq!(pin(camera), Some((11.0, 22.0)));
        assert_eq!(pin(late), None);
        let text = std::fs::read_to_string(day.join(SIDECAR)).unwrap();
        assert!(text.contains(&format!("\"track\": \"{FROM_PHOTOS}\"")), "{text}");

        let empty = folder.parent().unwrap().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("x.jpg"), b"not decoded").unwrap();
        let none = Options { source: Source::Photos, target: Target::Folder(empty), offset: None, dry_run: true };
        assert!(run(&none, None, &|_, _| {}).is_err());
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
    }

    /// A small JPEG whose EXIF says the camera's clock read `clock`
    /// (`YYYY:MM:DD HH:MM:SS`) at UTC offset `offset`.
    fn camera_jpeg(clock: &str, offset: &str) -> Vec<u8> {
        use exif::{Field, In, Tag, Value};
        let mut writer = exif::experimental::Writer::new();
        let fields = [(Tag::DateTimeOriginal, clock), (Tag::OffsetTimeOriginal, offset)]
            .map(|(tag, text)| Field { tag, ifd_num: In::PRIMARY, value: Value::Ascii(vec![text.as_bytes().to_vec()]) });
        fields.iter().for_each(|field| writer.push_field(field));
        let mut tiff = std::io::Cursor::new(Vec::new());
        writer.write(&mut tiff, false).unwrap();
        let tiff = tiff.into_inner();
        let jpeg = crate::image_io::encode_jpeg(&image::RgbaImage::from_pixel(8, 8, image::Rgba([90, 120, 150, 255])), 85).unwrap();
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&u16::try_from(8 + tiff.len()).unwrap().to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend(tiff);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    #[test]
    fn photos_that_record_their_time_zone_are_not_offered_another() {
        let (db, folder_id, folder) = library("recorded");
        let utc = |minute| chrono::Utc.with_ymd_and_hms(2026, 10, 11, 9, minute, 0).unwrap().timestamp();
        let track = folder.parent().unwrap().join("walk.gpx");
        std::fs::write(&track, gpx("Walk", &[(utc(0), 38.70, -9.10), (utc(10), 38.72, -9.12)])).unwrap();
        // Read at their recorded +01:00, one photo was taken on the walk and three two hours
        // after it; +03:00 would put those three on it, but their camera said otherwise.
        for (hour, minute) in [(10, 5), (12, 0), (12, 5), (12, 10)] {
            let name = format!("p{hour}{minute:02}.jpg");
            add_at(&db, folder_id, &folder, &name, clock(11, hour, minute), 0, None);
            std::fs::write(folder.join(&name), camera_jpeg(&format!("2026:10:11 {hour}:{minute:02}:00"), "+01:00")).unwrap();
        }
        let options = Options { source: Source::Tracks(vec![track]), target: Target::Folder(folder.clone()), offset: None, dry_run: true };
        let report = run(&options, Some(&db), &|_, _| {}).unwrap();
        assert_eq!((report.tagged, report.unmatched, report.zone_guessed, report.suggestion), (1, 3, 0, None));

        // A typed zone that isn't theirs is answered with the one they record.
        let typed = |offset| run(&Options { offset: Some(offset), ..options.clone() }, Some(&db), &|_, _| {}).unwrap();
        assert_eq!(typed(3600).suggestion, None, "the offset they record, typed");
        assert_eq!(typed(5 * 3600).suggestion, Some(Suggestion { offset: 3600, places: 1, of: 4, now: 0 }));
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
