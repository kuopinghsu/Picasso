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
    let instant = match capture {
        Capture::Unread => return Err(Review::Unread),
        Capture::Missing => return Err(Review::NoTime),
        Capture::Utc(instant) => instant,
        Capture::Clock { clock, offset: Some(offset) } => clock.and_utc().timestamp() - i64::from(offset),
        Capture::Clock { offset: None, .. } if !trip_time => return Err(Review::NoOffset),
        Capture::Clock { clock, offset: None } => zone.instant_of(clock).ok_or(Review::Ambiguous)?,
    };
    zone.day_of(instant + shift).ok_or(Review::NoTime)
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
    pub story: StoryFilter,
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

/// What adding photos to a Trip would do, shown before it is done.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddPreview {
    pub photos: usize,
    pub videos: usize,
    /// Already in the Trip, so left as they are.
    pub already: usize,
    /// Earliest and latest date taken of the ones that would be added.
    pub taken: Option<(i64, i64)>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Story {
    pub id: i64,
    pub trip_id: i64,
    pub title: String,
    pub subtitle: String,
    pub author: String,
    pub show_date: bool,
    /// Media reference of the cover, picked for this Story alone.
    pub cover: Option<i64>,
    pub template: String,
    pub archived: bool,
    /// The author marked it finished; until then it is a draft.
    pub finished: bool,
    /// Goes up with every saved change, so a stale editor can't overwrite.
    pub revision: i64,
    pub updated_at: i64,
}

/// The layout a new Story starts with.
pub const DEFAULT_TEMPLATE: &str = "journal";

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
    Paragraph(String),
    Photo(MediaUse),
    Gallery(Vec<MediaUse>),
    /// Photos in a grid of `columns` across and `rows` down, with one
    /// description for all. More photos than places make more rows.
    Grid { columns: u8, rows: u8, caption: String, media: Vec<MediaUse> },
    Separator,
    Credits(String),
    /// A kind or version this build doesn't know, kept as it was read.
    Unknown { kind: String, version: i64, payload: String, media: Vec<MediaUse> },
}

impl Block {
    /// Name stored with the block.
    pub fn kind(&self) -> &str {
        match self {
            Block::Heading(_) => "heading",
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
            Block::Heading(text) | Block::Paragraph(text) | Block::Credits(text) => json!({ "text": text }).to_string(),
            Block::Grid { columns, rows, caption, .. } => json!({ "columns": columns, "rows": rows, "caption": caption }).to_string(),
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
        match (kind, text, media.len()) {
            ("heading", Some(text), 0) => Block::Heading(text),
            ("paragraph", Some(text), 0) => Block::Paragraph(text),
            ("credits", Some(text), 0) => Block::Credits(text),
            ("separator", _, 0) => Block::Separator,
            ("photo", _, 1) => Block::Photo(media.remove(0)),
            ("gallery", _, _) => Block::Gallery(media),
            _ => unknown(media),
        }
    }
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
    use super::*;

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
