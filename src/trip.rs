//! Trips and Stories: travel photos grouped into a Trip and its Days, and the
//! narratives written from them. `docs/TRAVEL.md` describes the model and
//! its rules; the tables and every change to them are in `src/db/trips.rs`.

// The Trips page that uses these is not built yet; the tests do.
#![cfg_attr(not(test), allow(dead_code))]

use chrono::NaiveDate;
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
    /// A fixed offset such as `+08:00` or `UTC`, or the name of a zone.
    /// Only the form of a name is checked here; Days are counted with the
    /// timezone data packaged with Picasso, which knows the real names.
    pub fn parse(text: &str) -> Option<TripZone> {
        let text = text.trim();
        if text.starts_with(['+', '-']) || ["UTC", "GMT", "Z"].iter().any(|prefix| text.starts_with(prefix)) {
            return parse_utc_offset(text).map(TripZone::Fixed);
        }
        let named = !text.is_empty()
            && text.split('/').all(|part| {
                part.starts_with(|c: char| c.is_ascii_alphabetic())
                    && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+'))
            });
        named.then(|| TripZone::Named(text.to_string()))
    }

    /// The text [`TripZone::parse`] reads back, as stored in the library.
    pub fn code(&self) -> String {
        match self {
            TripZone::Named(name) => name.clone(),
            TripZone::Fixed(seconds) => format_utc_offset(*seconds),
        }
    }
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DayFilter {
    #[default]
    Any,
    Unassigned,
    Day(i64),
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
            Block::Photo(_) | Block::Gallery(_) | Block::Separator => "{}".into(),
            Block::Unknown { payload, .. } => payload.clone(),
        }
    }

    pub fn media(&self) -> &[MediaUse] {
        match self {
            Block::Photo(one) => std::slice::from_ref(one),
            Block::Gallery(media) | Block::Unknown { media, .. } => media,
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
        for bad in ["", "  ", "Asia/", "/Taipei", "Asia Taipei", "+25:00", "../etc"] {
            assert_eq!(TripZone::parse(bad), None, "{bad:?}");
        }
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
            Block::Separator,
            Block::Credits("Photos: me".into()),
        ];
        for block in blocks {
            let read = Block::read(block.kind(), block.version(), &block.payload(), block.media().to_vec());
            assert_eq!(read, block);
        }
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
