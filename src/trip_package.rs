//! A Trip package: one Trip with its Days, photos, and Stories in one
//! file, to carry it to another Picasso and go on editing it there
//! (`docs/TRAVEL.md`).
//!
//! The file is [`MAGIC`], the length of a JSON header as 8 bytes
//! little-endian, the header, and then each photo's bytes, one after
//! another in the header's order. Photos refer to each other by `key`, the
//! media reference they had where the package was made; the Stories'
//! blocks and covers use the same keys. The Trip's GPS track files follow
//! the photos, in the header's order too.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use chrono::NaiveDate;
use serde_json::{json, Value};

use crate::edit::Settings;
use crate::story_html::{Progress, Source, CANCELLED};
use crate::trip::{Block, MediaUse, Story, StoryBlock, DEFAULT_TEMPLATE};

/// The first bytes of every package.
const MAGIC: &[u8] = b"PICASSO-TRIP\n";
/// The header layout this build writes and the newest it reads.
const VERSION: u64 = 2;
/// The file extension of packages, without the dot.
pub const EXTENSION: &str = "picassotrip";
/// Largest header read, so a damaged file can't ask for any amount of memory.
const HEADER_MAX: u64 = 64 << 20;

/// The Trip, as carried in a package.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackTrip {
    pub title: String,
    pub description: String,
    pub destination: String,
    pub first_day: Option<NaiveDate>,
    pub last_day: Option<NaiveDate>,
    pub zone: String,
    pub trip_time: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackDay {
    pub day: Option<NaiveDate>,
    pub title: String,
    pub notes: String,
}

/// One photo of the Trip. `size` is `None` for a photo that was
/// unavailable where the package was made: its place in the Trip and its
/// Stories stays, shown as unavailable.
#[derive(Clone, Debug, PartialEq)]
pub struct PackPhoto {
    pub key: i64,
    /// File name to give it, without folders.
    pub name: String,
    /// Index of its Day in [`Package::days`].
    pub day: Option<usize>,
    pub size: Option<u64>,
    /// `false` for a photo only a Story still uses, no longer in the Trip.
    pub member: bool,
    /// SHA-256 of the carried bytes, in hex, so an import can find the
    /// same file already in its library.
    pub hash: Option<String>,
}

/// One GPS track file linked to the Trip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackTrack {
    /// The name the Trip shows for it.
    pub name: String,
    /// File name to give it, without folders.
    pub file: String,
    /// Index of its Day in [`Package::days`].
    pub day: Option<usize>,
    pub size: u64,
    /// SHA-256 of the file, in hex, so an import can find the same file
    /// already in its library.
    pub hash: String,
    /// Where the file is in this library: read from when packing, and
    /// where it was written or found when unpacking. Not carried.
    pub path: Option<PathBuf>,
}

/// Everything in a package but the photos' bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub trip: PackTrip,
    pub days: Vec<PackDay>,
    pub stories: Vec<PackStory>,
    /// In the Trip's order.
    pub photos: Vec<PackPhoto>,
    pub tracks: Vec<PackTrack>,
}

/// One Story of the Trip.
#[derive(Clone, Debug, PartialEq)]
pub struct PackStory {
    /// Its `id`, `trip_id`, and `revision` mean nothing elsewhere; its
    /// cover is a photo's key.
    pub story: Story,
    /// Blocks whose photos' `media_id` are the photos' keys.
    pub blocks: Vec<StoryBlock>,
}

/// The bytes of a photo as the package carries it: the file itself when it
/// is a JPEG without edits, so nothing is lost, or else the photo rendered
/// with its edits at full size, as a JPEG. A file that can't be rendered
/// without edits, such as a video, is carried as it is.
fn photo_bytes(source: &(PathBuf, Settings, i32)) -> Result<(Vec<u8>, String), String> {
    let (path, settings, rotation) = source;
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase();
    let unedited = *settings == Settings::default() && *rotation == 0;
    if unedited && (ext == "jpg" || ext == "jpeg") {
        return Ok((std::fs::read(path).map_err(|err| err.to_string())?, "jpg".into()));
    }
    match crate::edit::render_edited(path, settings, *rotation, None) {
        Ok(rgba) => Ok((crate::image_io::encode_jpeg(&rgba, 95)?, "jpg".into())),
        Err(_) if unedited && !ext.is_empty() => Ok((std::fs::read(path).map_err(|err| err.to_string())?, ext)),
        Err(err) => Err(err),
    }
}

/// A file name for a photo from its source: the same stem, never a path.
fn photo_name(source: &Path, ext: &str, key: i64) -> String {
    match source.file_stem().and_then(|stem| stem.to_str()).filter(|stem| !stem.trim().is_empty()) {
        Some(stem) => format!("{}.{ext}", crate::story_html::folder_name(stem)),
        None => format!("photo-{key}.{ext}"),
    }
}

/// Writes `package` to `path`, with the photos read from `sources` by key.
/// It is written beside `path` under a hidden name and renamed once
/// complete, so a failed or cancelled write leaves nothing. Photos that
/// can't be read are carried as unavailable. Returns how many those were.
pub fn write_package(path: &Path, package: &Package, sources: &HashMap<i64, Source>, progress: &Progress) -> Result<usize, String> {
    let parent = path.parent().ok_or("no folder to write in")?;
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("trip");
    let staging = parent.join(format!(".{name}.partial-{}", std::process::id()));
    let data = parent.join(format!(".{name}.data-{}", std::process::id()));
    let result = write_staged(&staging, &data, package, sources, progress).and_then(|missing| {
        std::fs::rename(&staging, path).map_err(|err| err.to_string())?;
        Ok(missing)
    });
    let _ = std::fs::remove_file(&data);
    if result.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    result
}

fn write_staged(staging: &Path, data: &Path, package: &Package, sources: &HashMap<i64, Source>, progress: &Progress) -> Result<usize, String> {
    let io = |err: std::io::Error| err.to_string();
    // The photos go to a side file first: the header, which says how long
    // each is, comes before them.
    let mut photos = package.photos.clone();
    let tracks: Vec<PackTrack>;
    let mut missing = 0;
    {
        let mut out = BufWriter::new(File::create(data).map_err(io)?);
        progress.total.store(photos.len(), Ordering::Relaxed);
        for photo in &mut photos {
            if progress.cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            progress.done.fetch_add(1, Ordering::Relaxed);
            let read = sources.get(&photo.key).and_then(Option::as_ref).and_then(|source| photo_bytes(source).ok().map(|bytes| (source, bytes)));
            match read {
                Some((source, (bytes, ext))) => {
                    out.write_all(&bytes).map_err(io)?;
                    photo.name = photo_name(&source.0, &ext, photo.key);
                    photo.size = Some(bytes.len() as u64);
                    photo.hash = Some(hex(ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref()));
                }
                None => {
                    missing += 1;
                    photo.size = None;
                }
            }
        }
        // Tracks that can't be read are left out: a link to nothing helps no one.
        tracks = package
            .tracks
            .iter()
            .filter_map(|track| {
                let bytes = std::fs::read(track.path.as_ref()?).ok()?;
                Some((track, bytes))
            })
            .map(|(track, bytes)| {
                out.write_all(&bytes).map_err(io)?;
                let hash = hex(ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref());
                Ok(PackTrack { size: bytes.len() as u64, hash, path: None, ..track.clone() })
            })
            .collect::<Result<_, String>>()?;
        out.flush().map_err(io)?;
    }
    let header = header(&Package { photos, tracks, ..package.clone() }).to_string();
    let mut out = BufWriter::new(File::create(staging).map_err(io)?);
    out.write_all(MAGIC).map_err(io)?;
    out.write_all(&(header.len() as u64).to_le_bytes()).map_err(io)?;
    out.write_all(header.as_bytes()).map_err(io)?;
    std::io::copy(&mut File::open(data).map_err(io)?, &mut out).map_err(io)?;
    out.into_inner().map_err(|err| err.to_string())?.sync_all().map_err(io)?;
    Ok(missing)
}

fn day_text(day: Option<NaiveDate>) -> Value {
    day.map_or(Value::Null, |day| Value::String(day.to_string()))
}

fn header(package: &Package) -> Value {
    let Package { trip, days, stories, photos, tracks } = package;
    json!({
        "package": VERSION,
        "trip": {
            "title": trip.title, "description": trip.description, "destination": trip.destination,
            "first_day": day_text(trip.first_day), "last_day": day_text(trip.last_day),
            "zone": trip.zone, "trip_time": trip.trip_time,
        },
        "days": days.iter().map(|day| json!({ "day": day_text(day.day), "title": day.title, "notes": day.notes })).collect::<Vec<_>>(),
        "stories": stories.iter().map(|PackStory { story, blocks }| json!({
            "title": story.title, "subtitle": story.subtitle, "author": story.author, "show_date": story.show_date,
            "introduction": story.introduction, "conclusion": story.conclusion,
            "template": story.template, "archived": story.archived, "finished": story.finished, "cover": story.cover,
            "cover_crop": story.cover_crop.to_text(),
            "blocks": blocks.iter().map(|block| json!({
                "uid": block.uid,
                "kind": block.block.kind(),
                "version": block.block.version(),
                "payload": block.block.payload(),
                "media": block.block.media().iter().map(|media| json!({ "key": media.media_id, "caption": media.caption, "alt": media.alt })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "photos": photos.iter().map(|photo| json!({ "key": photo.key, "name": photo.name, "day": photo.day, "size": photo.size, "member": photo.member, "hash": photo.hash })).collect::<Vec<_>>(),
        "tracks": tracks.iter().map(|track| json!({ "name": track.name, "file": track.file, "day": track.day, "size": track.size, "hash": track.hash })).collect::<Vec<_>>(),
    })
}

/// Reads the header of the package at `path`. Returns it, and where the
/// photos' bytes start.
pub fn read_package(path: &Path) -> Result<(Package, u64), String> {
    let damaged = || crate::i18n::tr("This file isn't a Trip package, or it is damaged.").to_string();
    let mut file = BufReader::new(File::open(path).map_err(|err| err.to_string())?);
    let mut magic = vec![0; MAGIC.len()];
    file.read_exact(&mut magic).map_err(|_| damaged())?;
    if magic != MAGIC {
        return Err(damaged());
    }
    let mut len = [0; 8];
    file.read_exact(&mut len).map_err(|_| damaged())?;
    let len = u64::from_le_bytes(len);
    if len > HEADER_MAX {
        return Err(damaged());
    }
    let mut text = vec![0; len as usize];
    file.read_exact(&mut text).map_err(|_| damaged())?;
    let value: Value = serde_json::from_slice(&text).map_err(|_| damaged())?;
    if value["package"].as_u64().is_none_or(|version| version > VERSION) {
        return Err(crate::i18n::tr("This Trip package was made by a newer Picasso.").into());
    }
    let package = parse(&value).ok_or_else(damaged)?;
    // Every photo's bytes must be there, and nothing more.
    let start = (MAGIC.len() + 8) as u64 + len;
    let total: u64 = package.photos.iter().filter_map(|photo| photo.size).sum::<u64>() + package.tracks.iter().map(|track| track.size).sum::<u64>();
    let file_len = std::fs::metadata(path).map_err(|err| err.to_string())?.len();
    if file_len != start + total {
        return Err(damaged());
    }
    Ok((package, start))
}

fn parse(value: &Value) -> Option<Package> {
    let text = |value: &Value| value.as_str().map(str::to_string);
    let day = |value: &Value| match value {
        Value::Null => Some(None),
        Value::String(text) => NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().map(Some),
        _ => None,
    };
    let trip = &value["trip"];
    let trip = PackTrip {
        title: text(&trip["title"])?,
        description: text(&trip["description"])?,
        destination: text(&trip["destination"])?,
        first_day: day(&trip["first_day"])?,
        last_day: day(&trip["last_day"])?,
        zone: text(&trip["zone"])?,
        trip_time: trip["trip_time"].as_bool()?,
    };
    let days = value["days"]
        .as_array()?
        .iter()
        .map(|item| Some(PackDay { day: day(&item["day"])?, title: text(&item["title"])?, notes: text(&item["notes"])? }))
        .collect::<Option<Vec<_>>>()?;
    let photos = value["photos"]
        .as_array()?
        .iter()
        .map(|item| {
            let name = text(&item["name"])?;
            let day = match &item["day"] {
                Value::Null => None,
                index => Some(index.as_u64().filter(|&index| (index as usize) < days.len())? as usize),
            };
            let size = match &item["size"] {
                Value::Null => None,
                size => Some(size.as_u64()?),
            };
            // A photo carried has a file name, never a way out of the folder.
            if size.is_some() && (name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.')) {
                return None;
            }
            Some(PackPhoto { key: item["key"].as_i64()?, name, day, size, member: item["member"].as_bool().unwrap_or(true), hash: text(&item["hash"]) })
        })
        .collect::<Option<Vec<_>>>()?;
    let day_index = |value: &Value| match value {
        Value::Null => Some(None),
        index => Some(Some(index.as_u64().filter(|&index| (index as usize) < days.len())? as usize)),
    };
    // Packages made before tracks were carried have none.
    let tracks = match &value["tracks"] {
        Value::Null => Vec::new(),
        tracks => tracks
            .as_array()?
            .iter()
            .map(|item| {
                let file = text(&item["file"])?;
                if !crate::track::is_track_path(Path::new(&file)) || file.contains(['/', '\\']) || file.starts_with('.') {
                    return None;
                }
                Some(PackTrack { name: text(&item["name"])?, file, day: day_index(&item["day"])?, size: item["size"].as_u64()?, hash: text(&item["hash"])?, path: None })
            })
            .collect::<Option<Vec<_>>>()?,
    };
    let keys: std::collections::HashSet<i64> = photos.iter().map(|photo| photo.key).collect();
    if keys.len() != photos.len() {
        return None;
    }
    let stories = value["stories"]
        .as_array()?
        .iter()
        .map(|item| {
            let blocks = item["blocks"]
                .as_array()?
                .iter()
                .map(|item| {
                    let media = item["media"]
                        .as_array()?
                        .iter()
                        .map(|use_| {
                            let key = use_["key"].as_i64().filter(|key| keys.contains(key))?;
                            Some(MediaUse { media_id: key, caption: text(&use_["caption"])?, alt: text(&use_["alt"])? })
                        })
                        .collect::<Option<Vec<_>>>()?;
                    let block = Block::read(item["kind"].as_str()?, item["version"].as_i64()?, item["payload"].as_str()?, media);
                    Some(StoryBlock { uid: text(&item["uid"])?, block })
                })
                .collect::<Option<Vec<_>>>()?;

            let story = Story {
                id: 0,
                trip_id: 0,
                title: text(&item["title"])?,
                subtitle: text(&item["subtitle"])?,
                introduction: text(&item["introduction"]).unwrap_or_default(),
                conclusion: text(&item["conclusion"]).unwrap_or_default(),
                author: text(&item["author"])?,
                show_date: item["show_date"].as_bool()?,
                cover: match &item["cover"] {
                    Value::Null => None,
                    cover => Some(cover.as_i64().filter(|key| keys.contains(key))?),
                },
                cover_crop: crate::trip::CoverCrop::from_text(item["cover_crop"].as_str().unwrap_or_default()),
                template: text(&item["template"]).unwrap_or_else(|| DEFAULT_TEMPLATE.into()),
                archived: item["archived"].as_bool()?,
                finished: item["finished"].as_bool()?,
                revision: 0,
                updated_at: 0,
            };
            Some(PackStory { story, blocks })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Package { trip, days, stories, photos, tracks })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// SHA-256 of the file at `path`, in hex, as [`PackPhoto::hash`] has it.
pub fn file_hash(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => context.update(&buffer[..read]),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(hex(context.finish().as_ref()))
}

/// Writes the photos of the package at `path` into `folder`, which is made
/// if needed, all but those in `skip`, and then its tracks, all but those
/// with a [`PackTrack::path`] already, setting it for the others. A name
/// already taken there gets " 2", " 3", … so nothing is replaced. Returns
/// each written photo's path by key.
pub fn extract_photos(
    path: &Path,
    package: &mut Package,
    start: u64,
    folder: &Path,
    skip: &HashSet<i64>,
    progress: &Progress,
) -> Result<HashMap<i64, PathBuf>, String> {
    let io = |err: std::io::Error| err.to_string();
    std::fs::create_dir_all(folder).map_err(io)?;
    let mut file = File::open(path).map_err(io)?;
    file.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut written = HashMap::new();
    progress.total.store(package.photos.len(), Ordering::Relaxed);
    for photo in &package.photos {
        progress.done.fetch_add(1, Ordering::Relaxed);
        let Some(size) = photo.size else { continue };
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        if skip.contains(&photo.key) {
            file.seek(SeekFrom::Current(size as i64)).map_err(io)?;
            continue;
        }
        let target = free_name(folder, &photo.name);
        let mut out = BufWriter::new(File::create(&target).map_err(io)?);
        let copied = std::io::copy(&mut (&mut file).take(size), &mut out).map_err(io)?;
        out.flush().map_err(io)?;
        if copied != size {
            return Err(crate::i18n::tr("This file isn't a Trip package, or it is damaged.").into());
        }
        written.insert(photo.key, target);
    }
    for track in &mut package.tracks {
        if track.path.is_some() {
            file.seek(SeekFrom::Current(track.size as i64)).map_err(io)?;
            continue;
        }
        let target = free_name(folder, &track.file);
        let mut out = BufWriter::new(File::create(&target).map_err(io)?);
        let copied = std::io::copy(&mut (&mut file).take(track.size), &mut out).map_err(io)?;
        out.flush().map_err(io)?;
        if copied != track.size {
            return Err(crate::i18n::tr("This file isn't a Trip package, or it is damaged.").into());
        }
        track.path = Some(target);
    }
    Ok(written)
}

fn free_name(folder: &Path, name: &str) -> PathBuf {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (1..)
        .map(|n| {
            let name = if n == 1 {
                name.to_string()
            } else if ext.is_empty() {
                format!("{stem} {n}")
            } else {
                format!("{stem} {n}.{ext}")
            };
            folder.join(name)
        })
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> Package {
        let mut cover = MediaUse::new(11);
        cover.alt = "A gate".into();
        let blocks = vec![
            StoryBlock::new(Block::Heading("Day 1".into())),
            StoryBlock::new(Block::Grid { columns: 2, rows: 1, caption: "Two".into(), media: vec![cover, MediaUse::new(12)] }),
            StoryBlock::new(Block::Photo(MediaUse::new(13))),
        ];
        let story = Story {
            id: 5,
            trip_id: 2,
            title: "Kyoto".into(),
            subtitle: "Spring".into(),
            introduction: "Cherry blossoms.".into(),
            conclusion: String::new(),
            author: "Ana".into(),
            show_date: true,
            cover: Some(11),
            cover_crop: crate::trip::CoverCrop::default(),
            template: DEFAULT_TEMPLATE.into(),
            archived: false,
            finished: true,
            revision: 9,
            updated_at: 0,
        };
        let photo = |key, day| PackPhoto { key, name: String::new(), day, size: None, member: true, hash: None };
        Package {
            trip: PackTrip { title: "Japan".into(), zone: "Asia/Tokyo".into(), first_day: NaiveDate::from_ymd_opt(2026, 4, 1), ..PackTrip::default() },
            days: vec![PackDay { day: NaiveDate::from_ymd_opt(2026, 4, 1), title: "Arrival".into(), notes: String::new() }],
            stories: vec![PackStory { story, blocks }],
            photos: vec![photo(11, Some(0)), photo(12, None), photo(13, Some(0))],
            tracks: Vec::new(),
        }
    }

    #[test]
    fn a_package_carries_the_trip_its_stories_and_its_photos() {
        let dir = std::env::temp_dir().join(format!("picasso-package-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let banner = Path::new(env!("CARGO_MANIFEST_DIR")).join("images/banner.jpg");
        let original = std::fs::read(&banner).unwrap();
        let sources: HashMap<i64, Source> =
            HashMap::from([(11, Some((banner.clone(), Settings::default(), 0))), (12, Some((banner.clone(), Settings::default(), 90))), (13, None)]);
        let mut package = package();
        let gpx = dir.join("walk.gpx");
        std::fs::write(&gpx, "<gpx></gpx>").unwrap();
        package.tracks = vec![
            PackTrack { name: "Walk".into(), file: "walk.gpx".into(), day: Some(0), path: Some(gpx.clone()), ..PackTrack::default() },
            PackTrack { name: "Gone".into(), file: "gone.gpx".into(), path: Some(dir.join("gone.gpx")), ..PackTrack::default() },
        ];
        let path = dir.join("Japan.picassotrip");
        assert_eq!(write_package(&path, &package, &sources, &Progress::default()).unwrap(), 1, "one photo was unavailable");
        std::fs::remove_file(&gpx).unwrap();
        assert!(std::fs::read_dir(&dir).unwrap().count() == 1, "nothing left beside it");

        let (mut read, start) = read_package(&path).unwrap();
        assert_eq!((&read.trip, &read.days, &read.stories[0].blocks), (&package.trip, &package.days, &package.stories[0].blocks));
        let story = &read.stories[0].story;
        assert_eq!((story.title.as_str(), story.cover, story.finished), ("Kyoto", Some(11), true));
        assert_eq!(read.photos.iter().map(|photo| photo.size.is_some()).collect::<Vec<_>>(), [true, true, false]);
        assert_eq!(read.photos[0].name, "banner.jpg");
        assert_eq!(read.photos[0].hash, file_hash(&banner), "an unedited photo's hash is its file's");
        assert_eq!(read.photos[2].hash, None);

        let folder = dir.join("photos");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("banner.jpg"), b"already here").unwrap();
        assert_eq!(read.tracks.len(), 1, "a track whose file is gone is left out");
        assert_eq!((read.tracks[0].name.as_str(), read.tracks[0].day, read.tracks[0].path.as_ref()), ("Walk", Some(0), None));
        let files = extract_photos(&path, &mut read, start, &folder, &HashSet::new(), &Progress::default()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(std::fs::read(folder.join("banner.jpg")).unwrap(), b"already here", "nothing is replaced");
        assert_eq!(std::fs::read(&files[&11]).unwrap(), original, "an unedited JPEG is carried as it is");
        assert_ne!(std::fs::read(&files[&12]).unwrap(), original, "a turned photo is carried as shown");
        assert_eq!(std::fs::read(read.tracks[0].path.as_ref().unwrap()).unwrap(), b"<gpx></gpx>", "the track comes after the photos");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_or_hostile_package_is_refused() {
        let dir = std::env::temp_dir().join(format!("picasso-package-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, header: &Value, tail: &[u8]| {
            let text = header.to_string();
            let mut bytes = MAGIC.to_vec();
            bytes.extend((text.len() as u64).to_le_bytes());
            bytes.extend(text.as_bytes());
            bytes.extend(tail);
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let mut package = package();
        package.photos[0].name = "a.jpg".into();
        package.photos[0].size = Some(3);
        let good = header(&package);
        assert!(read_package(&write("good", &good, b"abc")).is_ok());
        assert!(read_package(&write("short", &good, b"ab")).is_err(), "a photo's bytes are missing");
        let mut escape = good.clone();
        escape["photos"][0]["name"] = json!("../../evil.jpg");
        assert!(read_package(&write("escape", &escape, b"abc")).is_err(), "a name can't leave the folder");
        let mut track = good.clone();
        track["tracks"] = json!([{ "name": "Walk", "file": "../walk.gpx", "day": null, "size": 0, "hash": "" }]);
        assert!(read_package(&write("track", &track, b"abc")).is_err(), "a track's name can't leave the folder either");
        let mut old = good.clone();
        old.as_object_mut().unwrap().remove("tracks");
        old["package"] = json!(1);
        assert!(read_package(&write("old", &old, b"abc")).is_ok_and(|(read, _)| read.tracks.is_empty()), "an older package has no tracks");
        let mut newer = good.clone();
        newer["package"] = json!(VERSION + 1);
        assert!(read_package(&write("newer", &newer, b"abc")).unwrap_err().contains("newer"));
        std::fs::write(dir.join("text"), b"hello").unwrap();
        assert!(read_package(&dir.join("text")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
