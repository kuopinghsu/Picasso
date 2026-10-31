//! Trips and Stories: travel photos grouped into a Trip and its Days, and the
//! narratives written from them. `docs/TRAVEL.md` describes the model and
//! its rules; the tables and every change to them are in `src/db/trips.rs`.

// The Trips page that uses these is not built yet; the tests do.
#![cfg_attr(not(test), allow(dead_code))]

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{json, Value};

use crate::i18n::tr;
use crate::types::{format_utc_offset, parse_utc_offset};

/// Where a Trip's calendar days are counted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TripZone {
    /// A named zone such as `Asia/Taipei`, with its daylight saving.
    Named(String),
    /// Seconds east of UTC, the same all year.
    Fixed(i32),
}

impl TripZone {
    /// A fixed offset such as `+08:00` or `UTC`, or the name of a zone in
    /// the timezone data packaged with Picasso, such as `Asia/Taipei`.
    pub fn parse(text: &str) -> Option<TripZone> {
        let text = text.trim();
        if text.starts_with(['+', '-']) || ["UTC", "GMT", "Z"].iter().any(|prefix| text.starts_with(prefix)) {
            return parse_utc_offset(text).map(TripZone::Fixed);
        }
        let zone: chrono_tz::Tz = text.parse().ok()?;
        Some(TripZone::Named(zone.name().to_string()))
    }

    /// The calendar day of `instant` (Unix seconds) in this zone.
    pub fn day_of(&self, instant: i64) -> Option<NaiveDate> {
        let utc = chrono::DateTime::from_timestamp(instant, 0)?;
        Some(match self {
            TripZone::Named(name) => utc.with_timezone(&name.parse::<chrono_tz::Tz>().ok()?).date_naive(),
            TripZone::Fixed(seconds) => utc.with_timezone(&chrono::FixedOffset::east_opt(*seconds)?).date_naive(),
        })
    }

    /// The instant a clock showing `clock` in this zone stands for. `None`
    /// when the reading happens twice or never there, around a change of
    /// daylight saving time.
    pub fn instant_of(&self, clock: NaiveDateTime) -> Option<i64> {
        use chrono::TimeZone;
        let local = match self {
            TripZone::Named(name) => name.parse::<chrono_tz::Tz>().ok()?.from_local_datetime(&clock).single()?.timestamp(),
            TripZone::Fixed(seconds) => chrono::FixedOffset::east_opt(*seconds)?.from_local_datetime(&clock).single()?.timestamp(),
        };
        Some(local)
    }

    /// The text [`TripZone::parse`] reads back, as stored in the library.
    pub fn code(&self) -> String {
        match self {
            TripZone::Named(name) => name.clone(),
            TripZone::Fixed(seconds) => format_utc_offset(*seconds),
        }
    }
}

/// What a photo's file says about when it was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    /// The file hasn't been read for it yet.
    Unread,
    /// The camera's clock reading, and its offset from UTC when the file
    /// records it (`OffsetTimeOriginal`).
    Clock { clock: NaiveDateTime, offset: Option<i32> },
    /// An instant in UTC, as videos record it.
    Utc(i64),
    /// No capture time; the library shows the file's modification time.
    Missing,
}

/// Why a member's Day can't be worked out without the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Review {
    /// The file hasn't been read yet.
    Unread,
    /// The clock reading has no offset, and the user hasn't said the
    /// camera was set to Trip time.
    NoOffset,
    /// The reading happens twice or never in the Trip's zone.
    Ambiguous,
    /// The file has no capture time.
    NoTime,
}

impl Review {
    pub const ALL: [Review; 4] = [Review::Unread, Review::NoOffset, Review::Ambiguous, Review::NoTime];

    pub fn code(self) -> i64 {
        self as i64 + 1
    }

    pub fn from_code(code: i64) -> Option<Review> {
        Review::ALL.into_iter().find(|reason| reason.code() == code)
    }
}

/// What `path` says about when it was taken. Reads the file, so it is
/// called off the window's thread.
pub fn read_capture(path: &std::path::Path, modified_at: i64) -> Capture {
    let Ok(probe) = crate::image_io::probe(path, modified_at) else {
        return Capture::Missing;
    };
    // Without a capture time the probe gives the modification time back.
    if probe.taken_at == modified_at {
        return Capture::Missing;
    }
    if crate::types::is_video_path(path) {
        return Capture::Utc(probe.taken_at);
    }
    // The probe read the clock as a time on this computer; reading it back
    // the same way gives the clock's own reading.
    match chrono::DateTime::from_timestamp(probe.taken_at, 0) {
        Some(time) => Capture::Clock { clock: time.with_timezone(&chrono::Local).naive_local(), offset: probe.zone },
        None => Capture::Missing,
    }
}

/// How suggesting Days went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DaySuggestion {
    /// Members given a suggested Day.
    pub assigned: usize,
    /// Days made for dates that had none.
    pub new_days: usize,
    /// Members whose Day the user chose, left as they are.
    pub kept: usize,
    /// Members left without a suggestion, and why.
    pub review: Vec<(Review, usize)>,
}

/// The calendar day in `zone` on which a member was taken.
///
/// `shift` is the user's **Adjust date** change in the catalog, applied
/// once. `trip_time` says the user accepted that cameras without a
/// recorded offset were set to Trip time. The computer's own time zone is
/// never used, so the same inputs give the same day anywhere.
pub fn capture_day(capture: Capture, shift: i64, zone: &TripZone, trip_time: bool) -> Result<NaiveDate, Review> {
    let instant = capture_instant(capture, zone, trip_time)?;
    zone.day_of(instant + shift).ok_or(Review::NoTime)
}

/// The instant, in Unix seconds, a member was taken, read as
/// [`capture_day`] reads it: a clock without an offset is taken in the
/// Trip's zone, daylight saving included, only when `trip_time` says so.
pub fn capture_instant(capture: Capture, zone: &TripZone, trip_time: bool) -> Result<i64, Review> {
    Ok(match capture {
        Capture::Unread => return Err(Review::Unread),
        Capture::Missing => return Err(Review::NoTime),
        Capture::Utc(instant) => instant,
        Capture::Clock { clock, offset: Some(offset) } => clock.and_utc().timestamp() - i64::from(offset),
        Capture::Clock { offset: None, .. } if !trip_time => return Err(Review::NoOffset),
        Capture::Clock { clock, offset: None } => zone.instant_of(clock).ok_or(Review::Ambiguous)?,
    })
}

/// What the user enters to create or change a Trip.
#[derive(Clone, Debug, PartialEq)]
pub struct TripDraft {
    pub title: String,
    pub description: String,
    pub destination: String,
    pub first_day: Option<NaiveDate>,
    pub last_day: Option<NaiveDate>,
    /// Text of a [`TripZone`]; it has to be chosen, there is no default.
    pub zone: String,
    /// The user says cameras that record no offset were set to Trip time.
    pub trip_time: bool,
}

impl TripDraft {
    pub fn new(title: &str, zone: &str) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            destination: String::new(),
            first_day: None,
            last_day: None,
            zone: zone.into(),
            trip_time: false,
        }
    }

    /// The zone the Trip will be stored with, or why it can't be saved.
    pub fn validate(&self) -> Result<TripZone, String> {
        if self.title.trim().is_empty() {
            return Err(tr("Trip name is empty").into());
        }
        if let (Some(first), Some(last)) = (self.first_day, self.last_day) {
            if last < first {
                return Err(tr("The trip ends before it starts").into());
            }
        }
        TripZone::parse(&self.zone).ok_or_else(|| tr("Pick the trip's time zone").into())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trip {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub destination: String,
    pub first_day: Option<NaiveDate>,
    pub last_day: Option<NaiveDate>,
    pub zone: String,
    /// See [`TripDraft::trip_time`].
    pub trip_time: bool,
    /// Media reference of the cover, one of the Trip's members.
    pub cover: Option<i64>,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// Photos and videos in the Trip, and how many of them are in the library now.
    pub members: i64,
    pub available: i64,
    pub days: i64,
    pub stories: i64,
}

/// Which Trips a list shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TripQuery {
    /// Archived Trips instead of active ones.
    pub archived: bool,
    /// Words that must all appear in the title, destination, or description.
    pub search: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TripDay {
    pub id: i64,
    pub trip_id: i64,
    pub position: i64,
    pub day: Option<NaiveDate>,
    pub title: String,
    pub notes: String,
    pub members: i64,
}

/// A spot in a Trip that its photos were taken at, suggested from their
/// positions or kept as the user named it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TripPlace {
    pub id: i64,
    pub trip_id: i64,
    pub position: i64,
    pub name: String,
    /// The name in the local language or script, when it differs.
    pub local_name: String,
    pub lat: f64,
    pub lon: f64,
    pub notes: String,
    /// Changed by the user, so suggesting Places again keeps it.
    pub kept: bool,
    pub members: i64,
}

/// A position on Earth, as a Place must have.
pub fn valid_position(lat: f64, lon: f64) -> bool {
    lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}

/// A member with a position, as Suggest places sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Located {
    pub media_id: i64,
    /// When it was taken, in Unix seconds.
    pub at: i64,
    pub lat: f64,
    pub lon: f64,
}

/// What Suggest places did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaceSuggestion {
    /// Places the Trip has now.
    pub places: usize,
    /// Members without GPS or a pin placed by where a linked track was
    /// when they were taken; their photos stay without a position.
    pub from_tracks: usize,
}

/// Members taken close together, in the order first visited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaceCluster {
    pub lat: f64,
    pub lon: f64,
    pub first: i64,
    pub members: Vec<i64>,
}

/// Groups members into Places. In time order, a visit goes on while each
/// photo is within `distance` meters of the visit so far and `gap` seconds
/// of the one before; visits within `distance` of each other are one Place,
/// so coming back to a spot doesn't make another.
pub fn cluster_places(points: &[Located], distance: f64, gap: i64) -> Vec<PlaceCluster> {
    let mut sorted: Vec<&Located> = points.iter().filter(|point| valid_position(point.lat, point.lon)).collect();
    sorted.sort_by_key(|point| (point.at, point.media_id));
    let near = |a: (f64, f64), b: (f64, f64)| {
        let point = |(lat, lon): (f64, f64)| crate::track::Point { lat, lon, time: None };
        crate::track::distance_m(&point(a), &point(b)) <= distance
    };
    // Each visit: the sum of its positions, its members, and its last time.
    let mut visits: Vec<(f64, f64, Vec<i64>, i64, i64)> = Vec::new();
    for point in sorted {
        let joins = visits.last().is_some_and(|(lat, lon, members, _, last)| {
            let n = members.len() as f64;
            point.at - last <= gap && near((lat / n, lon / n), (point.lat, point.lon))
        });
        match visits.last_mut() {
            Some((lat, lon, members, _, last)) if joins => {
                *lat += point.lat;
                *lon += point.lon;
                members.push(point.media_id);
                *last = point.at;
            }
            _ => visits.push((point.lat, point.lon, vec![point.media_id], point.at, point.at)),
        }
    }
    let mut places: Vec<(f64, f64, Vec<i64>, i64)> = Vec::new();
    for (lat, lon, members, first, _) in visits {
        let centre = (lat / members.len() as f64, lon / members.len() as f64);
        let same = places.iter().position(|(plat, plon, pmembers, _)| {
            let n = pmembers.len() as f64;
            near((plat / n, plon / n), centre)
        });
        match same {
            Some(index) => {
                let place = &mut places[index];
                place.0 += lat;
                place.1 += lon;
                place.2.extend(members);
            }
            None => places.push((lat, lon, members, first)),
        }
    }
    places
        .into_iter()
        .map(|(lat, lon, members, first)| {
            let n = members.len() as f64;
            PlaceCluster { lat: lat / n, lon: lon / n, first, members }
        })
        .collect()
}

/// Who put a member in its Day. Suggesting Days again only replaces suggestions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaySource {
    Suggested,
    User,
}

impl DaySource {
    pub(crate) fn code(self) -> i64 {
        match self {
            DaySource::Suggested => 1,
            DaySource::User => 2,
        }
    }

    pub(crate) fn from_code(code: i64) -> Option<DaySource> {
        match code {
            1 => Some(DaySource::Suggested),
            2 => Some(DaySource::User),
            _ => None,
        }
    }
}

/// A photo or video in a Trip.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// Its media reference, which outlasts the catalog row.
    pub media_id: i64,
    /// The photo in the library now; `None` while it is unavailable.
    pub photo_id: Option<i64>,
    /// Where the file is, or was last seen.
    pub path: String,
    pub filename: String,
    pub is_video: bool,
    pub day_id: Option<i64>,
    pub day_source: Option<DaySource>,
}

/// A member's Day before a change, to put it back on Undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayAssignment {
    pub media_id: i64,
    pub day_id: Option<i64>,
    pub source: Option<DaySource>,
    pub position: i64,
    /// Why Suggest days left it without a Day, until the user places it.
    pub review: Option<Review>,
}

/// Which members of a Trip a list, a count, or a batch action covers. The
/// same query serves all three, so they always agree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemberQuery {
    pub day: DayFilter,
    pub starred: bool,
    /// Neither GPS from the camera nor a place set by hand.
    pub no_location: bool,
    /// Members whose photo is not in the library now.
    pub unavailable: bool,
    /// Members whose photo has no caption. Not an error; only a filter.
    pub no_caption: bool,
    pub story: StoryFilter,
    /// Members at this Place.
    pub place: Option<i64>,
}

/// Members used by the Trip's Stories.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StoryFilter {
    #[default]
    Any,
    /// Used by any of the Trip's Stories.
    AnyStory,
    /// Used by this Story.
    Story(i64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DayFilter {
    #[default]
    Any,
    Unassigned,
    Day(i64),
    /// Members Suggest days couldn't place and the user hasn't placed since.
    Review,
}

/// A library photo as Suggest trips sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shot {
    pub id: i64,
    /// When it was taken, 0 when unknown.
    pub taken_at: i64,
    pub folder: String,
    pub camera: String,
    pub located: bool,
    /// Already a member of some Trip.
    pub in_trip: bool,
}

/// Photos taken close together, offered as a Trip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TripCandidate {
    /// In the order taken.
    pub photos: Vec<i64>,
    pub first: i64,
    pub last: i64,
    /// Folders and cameras, the most used first.
    pub folders: Vec<String>,
    pub cameras: Vec<String>,
    pub located: usize,
    /// Of `photos`, those already in a Trip.
    pub in_trip: usize,
}

/// The photos with each listed camera's clock moved by its seconds, as if
/// it had been set right. Photos with no time stay without one.
pub fn shift_cameras(shots: &[Shot], shifts: &std::collections::HashMap<String, i64>) -> Vec<Shot> {
    shots
        .iter()
        .map(|shot| match shifts.get(&shot.camera) {
            Some(by) if shot.taken_at != 0 => Shot { taken_at: shot.taken_at + by, ..shot.clone() },
            _ => shot.clone(),
        })
        .collect()
}

/// Groups photos into Trips, a new one wherever `gap` seconds pass with no
/// photo, keeping groups of at least `min` photos, the latest first. With
/// `skip_in_trip`, photos already in a Trip are left out before grouping.
/// Photos with no time can't be placed and are never in a group.
pub fn suggest_trips(shots: &[Shot], gap: i64, min: usize, skip_in_trip: bool) -> Vec<TripCandidate> {
    let mut timed: Vec<&Shot> = shots.iter().filter(|shot| shot.taken_at != 0 && !(skip_in_trip && shot.in_trip)).collect();
    timed.sort_by_key(|shot| (shot.taken_at, shot.id));
    let mut groups: Vec<Vec<&Shot>> = Vec::new();
    for shot in timed {
        match groups.last_mut() {
            Some(group) if shot.taken_at - group.last().map_or(0, |last| last.taken_at) < gap => group.push(shot),
            _ => groups.push(vec![shot]),
        }
    }
    let ranked = |names: Vec<&str>| {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for name in names.into_iter().filter(|name| !name.is_empty()) {
            match counts.iter_mut().find(|(known, _)| known == name) {
                Some((_, count)) => *count += 1,
                None => counts.push((name.to_string(), 1)),
            }
        }
        counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        counts.into_iter().map(|(name, _)| name).collect::<Vec<_>>()
    };
    groups
        .into_iter()
        .rev()
        .filter(|group| group.len() >= min.max(1))
        .map(|group| TripCandidate {
            photos: group.iter().map(|shot| shot.id).collect(),
            first: group[0].taken_at,
            last: group[group.len() - 1].taken_at,
            folders: ranked(group.iter().map(|shot| shot.folder.as_str()).collect()),
            cameras: ranked(group.iter().map(|shot| shot.camera.as_str()).collect()),
            located: group.iter().filter(|shot| shot.located).count(),
            in_trip: group.iter().filter(|shot| shot.in_trip).count(),
        })
        .collect()
}

/// What adding photos to a Trip would do, shown before it is done.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddPreview {
    pub photos: usize,
    pub videos: usize,
    /// Already in the Trip, so left as they are.
    pub already: usize,
    /// Earliest and latest date taken of the ones that would be added.
    pub taken: Option<(i64, i64)>,
    /// Of the ones that would be added: with no date taken, so they wait
    /// for review; with a location from the camera or a pin; already in
    /// another Trip; and the same size, pixel size and file date as one
    /// before them, so possibly a copy of it, still added.
    pub no_time: usize,
    pub located: usize,
    pub in_other_trip: usize,
    pub duplicates: usize,
}

/// What deleting a Trip takes with it, for the confirmation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TripImpact {
    pub days: i64,
    pub members: i64,
    pub stories: i64,
    pub tracks: i64,
}

/// A GPS track of the library linked to a Trip and, optionally, one of its
/// Days. The path and name stay when the track leaves the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TripTrack {
    pub id: i64,
    pub day_id: Option<i64>,
    /// The track in the library now; `None` while it is unavailable.
    pub track_id: Option<i64>,
    pub path: String,
    pub name: String,
}

/// What a Trip resource refers to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourceKind {
    #[default]
    Document,
    Link,
    Note,
    Track,
}

impl ResourceKind {
    pub const ALL: [ResourceKind; 4] = [ResourceKind::Document, ResourceKind::Link, ResourceKind::Note, ResourceKind::Track];

    pub fn code(self) -> &'static str {
        match self {
            ResourceKind::Document => "document",
            ResourceKind::Link => "link",
            ResourceKind::Note => "note",
            ResourceKind::Track => "track",
        }
    }

    pub fn from_code(code: &str) -> ResourceKind {
        ResourceKind::ALL.into_iter().find(|kind| kind.code() == code).unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            ResourceKind::Document => "Document",
            ResourceKind::Link => "Link",
            ResourceKind::Note => "Note",
            ResourceKind::Track => "Track file",
        }
    }

    /// Whether its reference is a file on this computer.
    pub fn is_file(self) -> bool {
        matches!(self, ResourceKind::Document | ResourceKind::Track)
    }
}

/// A document, link, note, or track file kept with a Trip for reference.
/// Private unless `publish` was approved for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TripResource {
    pub id: i64,
    pub kind: ResourceKind,
    /// A file path, a web address, or nothing for a note.
    pub reference: String,
    pub title: String,
    pub notes: String,
    pub publish: bool,
}

impl TripResource {
    /// Why it can't be saved as it is, if it can't.
    pub fn problem(&self) -> Option<&'static str> {
        let reference = self.reference.trim();
        match self.kind {
            ResourceKind::Link if !(reference.starts_with("https://") || reference.starts_with("http://")) => {
                Some("Type a web address starting with https://")
            }
            kind if kind.is_file() && reference.is_empty() => Some("Choose a file"),
            ResourceKind::Note if self.title.trim().is_empty() && self.notes.trim().is_empty() => Some("Give the note a title or some text"),
            _ => None,
        }
    }

    /// Whether its file is no longer where it was.
    pub fn missing(&self) -> bool {
        self.kind.is_file() && !std::path::Path::new(&self.reference).exists()
    }

    /// Its title, else the file's name or the address.
    pub fn shown_name(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.trim().to_string();
        }
        if self.kind.is_file() {
            if let Some(name) = std::path::Path::new(&self.reference).file_name() {
                return name.to_string_lossy().into_owned();
            }
        }
        if self.kind == ResourceKind::Note {
            return self.notes.lines().next().unwrap_or_default().chars().take(40).collect();
        }
        self.reference.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Story {
    pub id: i64,
    pub trip_id: i64,
    pub title: String,
    pub subtitle: String,
    pub author: String,
    pub show_date: bool,
    /// Text that opens the Story, before its blocks; empty for none.
    pub introduction: String,
    /// Text that closes the Story, after its blocks; empty for none.
    pub conclusion: String,
    /// Media reference of the cover, picked for this Story alone.
    pub cover: Option<i64>,
    /// The part of the cover shown, for this Story alone.
    pub cover_crop: CoverCrop,
    pub template: String,
    pub archived: bool,
    /// The author marked it finished; until then it is a draft.
    pub finished: bool,
    /// Goes up with every saved change, so a stale editor can't overwrite.
    pub revision: i64,
    pub updated_at: i64,
}

/// The part of a Story's cover shown: the left, top, right, and bottom
/// edges in ten-thousandths of the photo as edited. The photo, its edit,
/// and its use elsewhere in the Story are left as they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoverCrop(pub [u16; 4]);

impl Default for CoverCrop {
    fn default() -> Self {
        Self::FULL
    }
}

impl CoverCrop {
    /// The whole photo.
    pub const FULL: CoverCrop = CoverCrop([0, 0, 10_000, 10_000]);
    /// Frame shapes offered, as width and height; `None` is the photo's own.
    pub const SHAPES: [Option<(u32, u32)>; 5] = [None, Some((16, 9)), Some((3, 2)), Some((2, 1)), Some((1, 1))];

    pub fn is_full(self) -> bool {
        self == Self::FULL
    }

    /// The edges as fractions of the photo.
    pub fn fractions(self) -> [f32; 4] {
        self.0.map(|edge| f32::from(edge) / 10_000.0)
    }

    /// The largest frame of `shape` on a photo `photo_w` × `photo_h`,
    /// scaled by `zoom` (1 is the largest, less is tighter), centred on
    /// `centre` as fractions, kept on the photo.
    pub fn framed(shape: Option<(u32, u32)>, photo_w: u32, photo_h: u32, zoom: f32, centre: [f32; 2]) -> CoverCrop {
        let (pw, ph) = (photo_w.max(1) as f32, photo_h.max(1) as f32);
        let ratio = shape.map_or(pw / ph, |(w, h)| w.max(1) as f32 / h.max(1) as f32);
        // The frame's size as fractions of the photo's width and height.
        let (mut w, mut h) = if ratio >= pw / ph { (1.0, pw / ratio / ph) } else { (ph * ratio / pw, 1.0) };
        let zoom = zoom.clamp(0.2, 1.0);
        (w, h) = (w * zoom, h * zoom);
        let left = (centre[0] - w / 2.0).clamp(0.0, 1.0 - w);
        let top = (centre[1] - h / 2.0).clamp(0.0, 1.0 - h);
        let edge = |fraction: f32| (fraction.clamp(0.0, 1.0) * 10_000.0).round() as u16;
        let crop = CoverCrop([edge(left), edge(top), edge(left + w), edge(top + h)]);
        if crop.0[2] <= crop.0[0] || crop.0[3] <= crop.0[1] { Self::FULL } else { crop }
    }

    /// As stored: "left,top,right,bottom", or empty for the whole photo.
    pub fn to_text(self) -> String {
        if self.is_full() { String::new() } else { self.0.map(|edge| edge.to_string()).join(",") }
    }

    /// Read back from [`CoverCrop::to_text`]; anything unreadable or empty
    /// is the whole photo.
    pub fn from_text(text: &str) -> CoverCrop {
        let edges: Vec<u16> = text.split(',').filter_map(|part| part.trim().parse().ok()).collect();
        match edges[..] {
            [left, top, right, bottom] if left < right && top < bottom && right <= 10_000 && bottom <= 10_000 => CoverCrop([left, top, right, bottom]),
            _ => Self::FULL,
        }
    }
}

impl Story {
    /// The key the cover is rendered under: its media reference, or its
    /// negative when the Story crops it, so the photo uncropped in a block
    /// and cropped as the cover are two pictures.
    pub fn cover_key(&self) -> Option<i64> {
        self.cover.map(|media| if self.cover_crop.is_full() { media } else { -media })
    }
}

/// The layout a new Story starts with.
pub const DEFAULT_TEMPLATE: &str = "journal";

/// The layouts a Story can be shown in, by code. Each draws the same
/// blocks; only the page around them changes.
pub const TEMPLATES: [&str; 4] = [DEFAULT_TEMPLATE, "diary", "guide", "essay"];

/// A template's code as stored, with an unknown one read as the default.
pub fn template_code(code: &str) -> &'static str {
    TEMPLATES.into_iter().find(|known| *known == code).unwrap_or(DEFAULT_TEMPLATE)
}

/// A template's name, to translate.
pub fn template_name(code: &str) -> &'static str {
    match template_code(code) {
        "diary" => "Travel Diary",
        "guide" => "Travel Guide",
        "essay" => "Photo Essay",
        _ => "Photography Journal",
    }
}

/// A photo as used in one block, with this Story's own words for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaUse {
    pub media_id: i64,
    pub caption: String,
    pub alt: String,
}

impl MediaUse {
    pub fn new(media_id: i64) -> Self {
        Self { media_id, ..Self::default() }
    }
}

/// Version of the block payloads this build writes. A block saved by a
/// newer build is kept as [`Block::Unknown`] and written back unchanged.
pub const BLOCK_VERSION: i64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    /// A heading one level below [`Block::Heading`].
    Subheading(String),
    Paragraph(String),
    Photo(MediaUse),
    Gallery(Vec<MediaUse>),
    /// Photos in a grid of `columns` across and `rows` down, with one
    /// description for all. More photos than places make more rows.
    Grid { columns: u8, rows: u8, caption: String, media: Vec<MediaUse> },
    Separator,
    Credits(String),
    /// Items with bullets, or numbered when `ordered`.
    List { ordered: bool, items: Vec<String> },
    /// Quoted words, and who said or wrote them.
    Quote { text: String, citation: String },
    /// A GPS track drawn as a line: its stretches as [latitude, longitude]
    /// in hundred-thousandths of a degree, copied from the Trip's tracks
    /// when chosen so the Story keeps it wherever it goes. `source` names
    /// the track chosen, or is empty for all of the Trip's.
    Route { caption: String, source: String, route: Vec<Vec<[i32; 2]>> },
    /// Practical information under a title, as label and value lines, such
    /// as "Getting there" and "Train from Lisbon, 2 h".
    Info { title: String, items: Vec<(String, String)> },
    /// A web page to read more, with its title and a note.
    Link { url: String, title: String, note: String },
    /// A kind or version this build doesn't know, kept as it was read.
    Unknown { kind: String, version: i64, payload: String, media: Vec<MediaUse> },
}

impl Block {
    /// Name stored with the block.
    pub fn kind(&self) -> &str {
        match self {
            Block::Heading(_) => "heading",
            Block::Subheading(_) => "subheading",
            Block::List { .. } => "list",
            Block::Quote { .. } => "quote",
            Block::Route { .. } => "route",
            Block::Info { .. } => "info",
            Block::Link { .. } => "link",
            Block::Paragraph(_) => "paragraph",
            Block::Photo(_) => "photo",
            Block::Gallery(_) => "gallery",
            Block::Grid { .. } => "grid",
            Block::Separator => "separator",
            Block::Credits(_) => "credits",
            Block::Unknown { kind, .. } => kind,
        }
    }

    pub fn version(&self) -> i64 {
        match self {
            Block::Unknown { version, .. } => *version,
            _ => BLOCK_VERSION,
        }
    }

    /// Everything but the photos, as stored JSON. Photos are stored in their
    /// own rows, so the library knows which ones each Story uses.
    pub fn payload(&self) -> String {
        match self {
            Block::Heading(text) | Block::Subheading(text) | Block::Paragraph(text) | Block::Credits(text) => json!({ "text": text }).to_string(),
            Block::List { ordered, items } => json!({ "ordered": ordered, "items": items }).to_string(),
            Block::Quote { text, citation } => json!({ "text": text, "citation": citation }).to_string(),
            Block::Grid { columns, rows, caption, .. } => json!({ "columns": columns, "rows": rows, "caption": caption }).to_string(),
            Block::Route { caption, source, route } => json!({ "caption": caption, "source": source, "route": route }).to_string(),
            Block::Info { title, items } => json!({ "title": title, "items": items }).to_string(),
            Block::Link { url, title, note } => json!({ "url": url, "title": title, "note": note }).to_string(),
            Block::Photo(_) | Block::Gallery(_) | Block::Separator => "{}".into(),
            Block::Unknown { payload, .. } => payload.clone(),
        }
    }

    pub fn media(&self) -> &[MediaUse] {
        match self {
            Block::Photo(one) => std::slice::from_ref(one),
            Block::Gallery(media) | Block::Grid { media, .. } | Block::Unknown { media, .. } => media,
            _ => &[],
        }
    }

    /// The block stored as `kind`, `version`, `payload`, and `media`. What
    /// this build can't read whole comes back as [`Block::Unknown`].
    pub fn read(kind: &str, version: i64, payload: &str, mut media: Vec<MediaUse>) -> Block {
        let unknown = |media| Block::Unknown { kind: kind.into(), version, payload: payload.into(), media };
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            return unknown(media);
        };
        if version > BLOCK_VERSION || !value.is_object() {
            return unknown(media);
        }
        let text = value["text"].as_str().map(str::to_string);
        if kind == "grid" {
            let size = |key: &str| value[key].as_u64().filter(|n| (1..=GRID_MAX as u64).contains(n)).map(|n| n as u8);
            return match (size("columns"), size("rows"), value["caption"].as_str()) {
                (Some(columns), Some(rows), Some(caption)) => Block::Grid { columns, rows, caption: caption.into(), media },
                _ => unknown(media),
            };
        }
        if kind == "route" {
            let route = serde_json::from_value::<Vec<Vec<[i32; 2]>>>(value["route"].clone()).ok();
            return match (value["caption"].as_str(), value["source"].as_str(), route) {
                (Some(caption), Some(source), Some(route)) if media.is_empty() => Block::Route { caption: caption.into(), source: source.into(), route },
                _ => unknown(media),
            };
        }
        if kind == "info" {
            let items = serde_json::from_value::<Vec<(String, String)>>(value["items"].clone()).ok();
            return match (value["title"].as_str(), items) {
                (Some(title), Some(items)) if media.is_empty() => Block::Info { title: title.into(), items },
                _ => unknown(media),
            };
        }
        if kind == "link" {
            return match (value["url"].as_str(), value["title"].as_str(), value["note"].as_str()) {
                (Some(url), Some(title), Some(note)) if media.is_empty() => Block::Link { url: url.into(), title: title.into(), note: note.into() },
                _ => unknown(media),
            };
        }
        match (kind, text, media.len()) {
            ("heading", Some(text), 0) => Block::Heading(text),
            ("subheading", Some(text), 0) => Block::Subheading(text),
            ("quote", Some(text), 0) => match value["citation"].as_str() {
                Some(citation) => Block::Quote { text, citation: citation.into() },
                None => unknown(media),
            },
            ("list", _, 0) => {
                let items = value["items"].as_array().and_then(|items| items.iter().map(|item| item.as_str().map(str::to_string)).collect::<Option<Vec<_>>>());
                match (value["ordered"].as_bool(), items) {
                    (Some(ordered), Some(items)) => Block::List { ordered, items },
                    _ => unknown(media),
                }
            }
            ("paragraph", Some(text), 0) => Block::Paragraph(text),
            ("credits", Some(text), 0) => Block::Credits(text),
            ("separator", _, 0) => Block::Separator,
            ("photo", _, 1) => Block::Photo(media.remove(0)),
            ("gallery", _, _) => Block::Gallery(media),
            _ => unknown(media),
        }
    }
}

/// A Link block's address when it can be followed: http or https only, so
/// a page never links to a file or a script.
pub fn web_address(url: &str) -> Option<&str> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let web = lower.starts_with("https://") || lower.starts_with("http://");
    let host = url.split_once("://").is_some_and(|(_, rest)| !rest.is_empty());
    (web && host && !url.contains(char::is_whitespace)).then_some(url)
}

/// Most places across or down a grid.
pub const GRID_MAX: u8 = 4;

/// The grid layouts offered, as (columns, rows).
pub const GRID_LAYOUTS: [(u8, u8); 7] = [(2, 1), (1, 2), (3, 1), (2, 2), (3, 2), (2, 3), (3, 3)];

/// The offered layout that fits `count` photos with the fewest empty places.
pub fn grid_layout_for(count: usize) -> (u8, u8) {
    GRID_LAYOUTS
        .into_iter()
        .filter(|(columns, rows)| usize::from(columns * rows) >= count)
        .min_by_key(|(columns, rows)| (usize::from(columns * rows) - count, columns.abs_diff(*rows)))
        .unwrap_or((3, 3))
}

/// Most points a Route block keeps: enough for a line across a page.
pub const ROUTE_POINTS: usize = 800;

/// The points of `tracks` for a Route block, every so many kept so there
/// are no more than [`ROUTE_POINTS`], each stretch with its last point.
pub fn route_of(tracks: &[crate::track::Track]) -> Vec<Vec<[i32; 2]>> {
    let segments: Vec<&Vec<crate::track::Point>> = tracks.iter().flat_map(|track| &track.segments).collect();
    let total: usize = segments.iter().map(|segment| segment.len()).sum();
    let step = total.div_ceil(ROUTE_POINTS).max(1);
    let fixed = |point: &crate::track::Point| [(point.lat * 1e5).round() as i32, (point.lon * 1e5).round() as i32];
    segments
        .into_iter()
        .filter_map(|segment| {
            let mut kept: Vec<[i32; 2]> = segment.iter().step_by(step).map(fixed).collect();
            if (segment.len() - 1) % step != 0 {
                kept.extend(segment.last().map(fixed));
            }
            (kept.len() >= 2).then_some(kept)
        })
        .collect()
}

/// Closest zoom a route map shows, a few streets across.
pub const ROUTE_MAX_ZOOM: u8 = 16;

/// Where a Route block's map is: its zoom, and the middle of the map as a
/// world position in pixels at that zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RouteView {
    pub zoom: u8,
    pub x: f64,
    pub y: f64,
}

/// The closest map that shows a Route block's stretches in a `width` ×
/// `height` box, `pad` from its edges, and the stretches in that box, north
/// up. `None` for a route without points.
pub fn route_shape(route: &[Vec<[i32; 2]>], width: f32, height: f32, pad: f32) -> Option<(RouteView, Vec<Vec<[f32; 2]>>)> {
    let world = |[lat, lon]: [i32; 2]| crate::geo::latlon_to_world(f64::from(lat) / 1e5, f64::from(lon) / 1e5, 0);
    let points: Vec<Vec<(f64, f64)>> = route.iter().map(|segment| segment.iter().map(|&point| world(point)).collect()).collect();
    let (mut min, mut max) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for &(x, y) in points.iter().flatten() {
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    }
    if min.0 > max.0 {
        return None;
    }
    let (inner_w, inner_h) = (f64::from((width - 2.0 * pad).max(1.0)), f64::from((height - 2.0 * pad).max(1.0)));
    let fits = |zoom: u8| {
        let scale = f64::from(1u32 << zoom);
        (max.0 - min.0) * scale <= inner_w && (max.1 - min.1) * scale <= inner_h
    };
    let zoom = (0..=ROUTE_MAX_ZOOM).rev().find(|&zoom| fits(zoom)).unwrap_or(0);
    let scale = f64::from(1u32 << zoom);
    let view = RouteView { zoom, x: (min.0 + max.0) / 2.0 * scale, y: (min.1 + max.1) / 2.0 * scale };
    let (half_w, half_h) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let shape = points
        .iter()
        .map(|segment| segment.iter().map(|&(x, y)| [(half_w + x * scale - view.x) as f32, (half_h + y * scale - view.y) as f32]).collect())
        .collect();
    Some((view, shape))
}

/// How long a Route block's line is, in meters.
pub fn route_meters(route: &[Vec<[i32; 2]>]) -> f64 {
    let point = |[lat, lon]: [i32; 2]| crate::track::Point { lat: f64::from(lat) / 1e5, lon: f64::from(lon) / 1e5, time: None };
    route.iter().flat_map(|segment| segment.windows(2)).map(|pair| crate::track::distance_m(&point(pair[0]), &point(pair[1]))).sum()
}

/// A block and the id it keeps through edits, moves, and saves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoryBlock {
    pub uid: String,
    pub block: Block,
}

impl StoryBlock {
    pub fn new(block: Block) -> Self {
        Self { uid: new_uid(), block }
    }
}

/// A random id for a new block, unique within any Story.
pub fn new_uid() -> String {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 8];
    if ring::rand::SystemRandom::new().fill(&mut bytes).is_err() {
        // Without system randomness, the clock and a counter still keep ids apart.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        bytes = (nanos ^ COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed).rotate_left(48)).to_le_bytes();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn photos_taken_close_together_are_one_place_and_a_return_joins_it() {
        // About 111 m per 0.001 degree of latitude.
        let at = |media_id: i64, at: i64, lat: f64| Located { media_id, at, lat, lon: 2.0 };
        let points = vec![
            at(1, 0, 41.000),
            at(2, 600, 41.0005),
            at(3, 1200, 41.020),
            at(4, 1800, 41.0201),
            at(5, 9000, 41.0003),
            at(6, 9100, 99.0),
        ];
        let places = cluster_places(&points, 200.0, 3600);
        assert_eq!(places.iter().map(|place| place.members.clone()).collect::<Vec<_>>(), vec![vec![1, 2, 5], vec![3, 4]], "a bad position is left out");
        assert_eq!(places[0].first, 0);
        assert!((places[0].lat - 41.000_266).abs() < 1e-5);
        // A long gap starts a new visit, which still joins the same place.
        assert_eq!(cluster_places(&points, 200.0, 60).len(), 2);
        assert_eq!(cluster_places(&points, 5_000.0, 3600).len(), 1, "a wider distance makes one place");
        assert!(!valid_position(91.0, 0.0) && !valid_position(0.0, f64::NAN) && valid_position(-90.0, 180.0));
    }

    #[test]
    fn photos_far_apart_are_separate_trips_the_latest_first() {
        const DAY: i64 = 86_400;
        let shot = |id: i64, taken_at: i64, in_trip: bool| Shot { id, taken_at, folder: if id < 10 { "Kyoto".into() } else { "Oslo".into() }, camera: "X100".into(), located: id % 2 == 0, in_trip };
        let shots = vec![
            shot(2, 10 * DAY + 3600, false),
            shot(1, 10 * DAY, false),
            shot(3, 11 * DAY, true),
            shot(11, 20 * DAY, false),
            shot(12, 20 * DAY + 60, false),
            shot(13, 0, false),
            shot(14, 40 * DAY, false),
        ];
        let found = suggest_trips(&shots, 2 * DAY, 2, false);
        assert_eq!(found.iter().map(|group| group.photos.clone()).collect::<Vec<_>>(), vec![vec![11, 12], vec![1, 2, 3]], "alone and timeless are left out");
        assert_eq!((found[1].first, found[1].last, found[1].located, found[1].in_trip), (10 * DAY, 11 * DAY, 1, 1));
        assert_eq!((found[0].folders.clone(), found[0].cameras.clone()), (vec!["Oslo".to_string()], vec!["X100".to_string()]));
        let fresh = suggest_trips(&shots, 2 * DAY, 2, true);
        assert_eq!(fresh[1].photos, vec![1, 2], "photos already in a trip are skipped");
        assert_eq!(suggest_trips(&shots, DAY / 2, 2, false).len(), 2, "a shorter gap splits day 10 from day 11");
        // A camera a day behind joins the next group once its clock is moved.
        let mut late = shots.clone();
        late.push(Shot { id: 20, taken_at: 19 * DAY - 7200, camera: "GoPro".into(), ..Shot::default() });
        late.push(Shot { id: 21, taken_at: 0, camera: "GoPro".into(), ..Shot::default() });
        let moved = shift_cameras(&late, &std::collections::HashMap::from([("GoPro".to_string(), DAY)]));
        assert_eq!(suggest_trips(&moved, DAY / 2, 2, false)[0].photos, vec![20, 11, 12]);
        assert_eq!(moved.iter().find(|shot| shot.id == 21).map(|shot| shot.taken_at), Some(0), "no time stays none");
    }

    use super::*;

    #[test]
    fn a_route_keeps_few_points_and_draws_inside_its_box() {
        let point = |n: usize| crate::track::Point { lat: 35.0 + n as f64 * 1e-4, lon: 135.0, time: None };
        let track = crate::track::Track { name: "Walk".into(), segments: vec![(0..2001).map(point).collect(), vec![point(0)]] };
        let route = route_of(&[track]);
        assert_eq!(route.len(), 1, "a stretch of one point is no line");
        assert!(route[0].len() <= ROUTE_POINTS + 1);
        assert_eq!(route[0].last(), Some(&[3_520_000, 13_500_000]), "the last point stays");
        assert!((route_meters(&route) - 22_240.0).abs() < 100.0);
        let block = Block::Route { caption: "Kyoto".into(), source: "/w.gpx".into(), route: route.clone() };
        assert_eq!(Block::read(block.kind(), block.version(), &block.payload(), Vec::new()), block);
        let (view, shape) = route_shape(&route, 200.0, 100.0, 10.0).unwrap();
        assert!(shape.iter().flatten().all(|&[x, y]| (10.0..=190.0).contains(&x) && (10.0..=90.0).contains(&y)));
        let (closer, _) = route_shape(&route, 400.0, 200.0, 20.0).unwrap();
        assert_eq!(closer.zoom, view.zoom + 1, "twice the box is one zoom closer");
        assert!(route_shape(&[], 200.0, 100.0, 10.0).is_none());
    }

    #[test]
    fn a_trip_zone_is_a_named_zone_or_a_fixed_offset() {
        assert_eq!(TripZone::parse("Asia/Taipei"), Some(TripZone::Named("Asia/Taipei".into())));
        assert_eq!(TripZone::parse("America/Argentina/Buenos_Aires").map(|zone| zone.code()).as_deref(), Some("America/Argentina/Buenos_Aires"));
        assert_eq!(TripZone::parse("+08:00"), Some(TripZone::Fixed(8 * 3600)));
        assert_eq!(TripZone::parse("UTC-03:30"), Some(TripZone::Fixed(-(3 * 3600 + 1800))));
        assert_eq!(TripZone::parse("UTC"), Some(TripZone::Fixed(0)));
        assert_eq!(TripZone::Fixed(-(3 * 3600 + 1800)).code(), "-03:30");
        assert_eq!(TripZone::parse("asia/taipei"), None, "names are spelled as in the timezone data");
        for bad in ["", "  ", "Asia/", "/Taipei", "Asia Taipei", "Mars/Olympus", "+25:00", "../etc"] {
            assert_eq!(TripZone::parse(bad), None, "{bad:?}");
        }
    }

    fn clock(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_day_comes_from_the_clock_and_its_offset() {
        let taipei = TripZone::parse("Asia/Taipei").unwrap();
        // 00:30 in Tokyo is 23:30 the evening before in Taipei.
        let tokyo = Capture::Clock { clock: clock("2026-03-02 00:30:00"), offset: Some(9 * 3600) };
        assert_eq!(capture_day(tokyo, 0, &taipei, false), Ok(date("2026-03-01")));
        assert_eq!(capture_day(tokyo, 3600, &taipei, false), Ok(date("2026-03-02")), "an Adjust date shift counts once");
        let video = Capture::Utc(clock("2026-03-01 16:30:00").and_utc().timestamp());
        assert_eq!(capture_day(video, 0, &taipei, false), Ok(date("2026-03-02")));
        assert_eq!(capture_day(Capture::Missing, 0, &taipei, true), Err(Review::NoTime));
        assert_eq!(capture_day(Capture::Unread, 0, &taipei, true), Err(Review::Unread));
    }

    #[test]
    fn a_clock_without_offset_needs_the_user_and_daylight_saving_care() {
        let new_york = TripZone::parse("America/New_York").unwrap();
        let bare = |text| Capture::Clock { clock: clock(text), offset: None };
        assert_eq!(capture_day(bare("2026-07-04 23:59:00"), 0, &new_york, false), Err(Review::NoOffset));
        assert_eq!(capture_day(bare("2026-07-04 23:59:00"), 0, &new_york, true), Ok(date("2026-07-04")));
        // 02:30 never happens on 8 March 2026, and 01:30 happens twice on 1 November.
        assert_eq!(capture_day(bare("2026-03-08 02:30:00"), 0, &new_york, true), Err(Review::Ambiguous));
        assert_eq!(capture_day(bare("2026-11-01 01:30:00"), 0, &new_york, true), Err(Review::Ambiguous));
        // A fixed offset has no daylight saving.
        let fixed = TripZone::parse("-05:00").unwrap();
        assert_eq!(capture_day(bare("2026-03-08 02:30:00"), 0, &fixed, true), Ok(date("2026-03-08")));
        // Leap day, and across the date line.
        assert_eq!(capture_day(bare("2028-02-29 12:00:00"), 0, &fixed, true), Ok(date("2028-02-29")));
        let samoa = TripZone::parse("Pacific/Apia").unwrap();
        let fiji = Capture::Clock { clock: clock("2026-06-01 10:00:00"), offset: Some(12 * 3600) };
        assert_eq!(capture_day(fiji, 0, &samoa, false), Ok(date("2026-06-01")), "UTC+13 is ahead of UTC+12");
        let honolulu = Capture::Clock { clock: clock("2026-06-01 10:00:00"), offset: Some(-10 * 3600) };
        assert_eq!(capture_day(honolulu, 0, &samoa, false), Ok(date("2026-06-02")));
    }

    #[test]
    fn a_trip_needs_a_name_a_time_zone_and_days_in_order() {
        let mut draft = TripDraft::new("Kyoto", "Asia/Tokyo");
        assert_eq!(draft.validate(), Ok(TripZone::Named("Asia/Tokyo".into())));
        draft.first_day = NaiveDate::from_ymd_opt(2026, 4, 2);
        draft.last_day = NaiveDate::from_ymd_opt(2026, 4, 1);
        assert!(draft.validate().is_err(), "the last day is before the first");
        draft.last_day = draft.first_day;
        assert!(draft.validate().is_ok(), "a one-day trip");
        assert!(TripDraft::new(" ", "Asia/Tokyo").validate().is_err());
        assert!(TripDraft::new("Kyoto", "").validate().is_err(), "the time zone has no default");
    }

    #[test]
    fn only_web_addresses_are_followed() {
        assert_eq!(web_address(" https://example.com/a "), Some("https://example.com/a"));
        assert_eq!(web_address("HTTP://example.com"), Some("HTTP://example.com"));
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "example.com", "https://", "https://a b.com", ""] {
            assert_eq!(web_address(bad), None, "{bad}");
        }
    }

    #[test]
    fn blocks_read_back_what_they_store() {
        let photo = MediaUse { media_id: 3, caption: "Gion at dusk".into(), alt: "Lanterns on a street".into() };
        let blocks = [
            Block::Heading("Day 1 — 京都".into()),
            Block::Paragraph("<script>\"quoted\" & 'odd'</script>".into()),
            Block::Photo(photo.clone()),
            Block::Gallery(vec![photo.clone(), MediaUse::new(4)]),
            Block::Gallery(Vec::new()),
            Block::Grid { columns: 3, rows: 2, caption: "Market \"day\"".into(), media: vec![photo.clone(), MediaUse::new(5)] },
            Block::Separator,
            Block::Credits("Photos: me".into()),
            Block::Info { title: "Getting there".into(), items: vec![("Train".into(), "2 h from Lisbon".into()), (String::new(), "No \"booking\" needed".into())] },
            Block::Link { url: "https://example.com/ferry?a=1&b=2".into(), title: "Ferry times".into(), note: "Summer only".into() },
        ];
        for block in blocks {
            let read = Block::read(block.kind(), block.version(), &block.payload(), block.media().to_vec());
            assert_eq!(read, block);
        }
    }

    #[test]
    fn a_grid_gets_the_layout_with_fewest_empty_places() {
        let cases = [(1, (2, 1)), (2, (2, 1)), (3, (3, 1)), (4, (2, 2)), (5, (3, 2)), (6, (3, 2)), (9, (3, 3)), (12, (3, 3))];
        for (count, layout) in cases {
            assert_eq!(grid_layout_for(count), layout, "{count} photos");
        }
        let wide = Block::read("grid", 1, r#"{"columns":9,"rows":1,"caption":""}"#, Vec::new());
        assert!(matches!(wide, Block::Unknown { .. }), "a layout this build can't show is kept as it is");
    }

    #[test]
    fn blocks_this_build_cannot_read_are_kept_whole() {
        let media = vec![MediaUse::new(9)];
        let newer = Block::read("paragraph", BLOCK_VERSION + 1, r#"{"text":"hi","style":"lead"}"#, Vec::new());
        let map = Block::read("map", 1, r#"{"zoom":7}"#, media.clone());
        let broken = Block::read("heading", 1, "not json", Vec::new());
        for (block, kind) in [(&newer, "paragraph"), (&map, "map"), (&broken, "heading")] {
            assert!(matches!(block, Block::Unknown { .. }), "{block:?}");
            assert_eq!(block.kind(), kind);
        }
        assert_eq!(newer.version(), BLOCK_VERSION + 1);
        assert_eq!(newer.payload(), r#"{"text":"hi","style":"lead"}"#, "written back unchanged");
        assert_eq!(map.media(), &media[..], "its photos stay with it");
    }

    #[test]
    fn new_block_ids_differ() {
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| new_uid()).collect();
        assert_eq!(ids.len(), 1000);
        assert!(ids.iter().all(|id| id.len() == 16));
    }
}

#[cfg(test)]
mod cover_tests {
    use super::*;

    #[test]
    fn a_cover_frame_keeps_its_shape_on_the_photo_and_reads_back() {
        // 16:9 on a 3:2 photo: full width, part of the height.
        let wide = CoverCrop::framed(Some((16, 9)), 3000, 2000, 1.0, [0.5, 0.5]);
        let [left, top, right, bottom] = wide.fractions();
        assert_eq!((left, right), (0.0, 1.0));
        assert!(((right - left) * 3000.0 / ((bottom - top) * 2000.0) - 16.0 / 9.0).abs() < 0.01);
        // Pushed past the edge, it stays on the photo.
        let low = CoverCrop::framed(Some((1, 1)), 3000, 2000, 0.5, [0.9, 1.0]);
        assert_eq!((low.0[2], low.0[3]), (10_000, 10_000));
        assert_eq!(CoverCrop::framed(None, 3000, 2000, 1.0, [0.5, 0.5]), CoverCrop::FULL);
        assert_eq!(CoverCrop::from_text(&wide.to_text()), wide);
        assert_eq!(CoverCrop::FULL.to_text(), "");
        for bad in ["", "1,2,3", "5,5,4,9", "0,0,10001,10000", "a,b,c,d"] {
            assert_eq!(CoverCrop::from_text(bad), CoverCrop::FULL, "{bad}");
        }
    }

    #[test]
    fn only_a_cropped_cover_has_a_key_of_its_own() {
        let mut story = Story {
            id: 1,
            trip_id: 1,
            title: String::new(),
            subtitle: String::new(),
            author: String::new(),
            show_date: false,
            introduction: String::new(),
            conclusion: String::new(),
            cover: Some(7),
            cover_crop: CoverCrop::FULL,
            template: DEFAULT_TEMPLATE.into(),
            archived: false,
            finished: false,
            revision: 1,
            updated_at: 0,
        };
        assert_eq!(story.cover_key(), Some(7));
        story.cover_crop = CoverCrop([0, 0, 5_000, 5_000]);
        assert_eq!(story.cover_key(), Some(-7));
        story.cover = None;
        assert_eq!(story.cover_key(), None);
    }
}

#[cfg(test)]
mod resource_tests {
    use super::*;

    #[test]
    fn resources_say_what_they_lack() {
        let link = TripResource { kind: ResourceKind::Link, reference: "example.com".into(), ..Default::default() };
        assert!(link.problem().is_some());
        assert!(TripResource { reference: "https://example.com".into(), ..link }.problem().is_none());
        assert!(TripResource { kind: ResourceKind::Document, ..Default::default() }.problem().is_some());
        let note = TripResource { kind: ResourceKind::Note, ..Default::default() };
        assert!(note.problem().is_some());
        assert!(TripResource { notes: "Ferry at 9".into(), ..note }.problem().is_none());
    }

    #[test]
    fn a_resource_is_named_by_its_title_else_its_file() {
        let doc = TripResource { kind: ResourceKind::Document, reference: "/trips/spain/tickets.pdf".into(), ..Default::default() };
        assert_eq!(doc.shown_name(), "tickets.pdf");
        assert!(doc.missing());
        assert_eq!(TripResource { title: "Train".into(), ..doc }.shown_name(), "Train");
        assert!(!TripResource { kind: ResourceKind::Link, ..Default::default() }.missing());
    }
}
