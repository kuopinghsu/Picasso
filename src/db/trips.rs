//! Tables of Trips, their Days, members, and track links, and of Stories
//! (`docs/TRAVEL.md`). Every change that touches more than one row runs in
//! one transaction.
//!
//! Trips and Stories refer to photos through `media_refs`, never only by the
//! catalog row: a row goes when its file leaves the library, and comes back
//! under a new id. A reference keeps the file's path, size, modification
//! time, and date as hints, and is linked again after each scan.

// The Trips page that calls most of these is not built yet; the tests do.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashSet;

use chrono::{NaiveDate, NaiveDateTime};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{order_sql, path_text, read_photo, Database, PHOTO_COLS};
use crate::i18n::tr;
use crate::trip::{
    capture_day, cluster_places, valid_position, AddPreview, Capture, Located, Shot, TripPlace, DaySuggestion, Review, TripZone, Block, DayAssignment, DayFilter, DaySource, MediaUse, Member, MemberQuery, Story, StoryBlock, StoryFilter, Trip, TripDay,
    TripDraft, TripImpact, TripQuery, TripTrack, DEFAULT_TEMPLATE,
};
use crate::types::{is_video_path, unix_now, Photo, Sort};

pub(super) const TABLES: &str = "\
-- A photo as Trips and Stories know it. `photo_id` is the catalog row while
-- the photo is in the library; the other columns are the hints that find it
-- again. `unlinked_at` is when the row went.
CREATE TABLE IF NOT EXISTS media_refs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id INTEGER UNIQUE REFERENCES photos(id) ON DELETE SET NULL,
    path TEXT NOT NULL,
    filename TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    file_taken_at INTEGER NOT NULL,
    unlinked_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_media_refs_unlinked ON media_refs(unlinked_at) WHERE photo_id IS NULL;
CREATE TRIGGER IF NOT EXISTS photos_media_hints AFTER UPDATE OF path, filename, bytes, modified_at, file_taken_at ON photos BEGIN
    UPDATE media_refs SET path = NEW.path, filename = NEW.filename, bytes = NEW.bytes,
        modified_at = NEW.modified_at, file_taken_at = NEW.file_taken_at
    WHERE photo_id = NEW.id;
END;
CREATE TRIGGER IF NOT EXISTS photos_media_unlinked BEFORE DELETE ON photos BEGIN
    UPDATE media_refs SET unlinked_at = CAST(strftime('%s', 'now') AS INTEGER) WHERE photo_id = OLD.id;
END;
CREATE TABLE IF NOT EXISTS trips (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    destination TEXT NOT NULL DEFAULT '',
    first_day TEXT,
    last_day TEXT,
    zone TEXT NOT NULL,
    cover_ref INTEGER REFERENCES media_refs(id),
    archived INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS trip_days (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trip_id INTEGER NOT NULL REFERENCES trips(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    day TEXT,
    title TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_trip_days_trip ON trip_days(trip_id, position);
CREATE TRIGGER IF NOT EXISTS trip_days_stay BEFORE UPDATE OF trip_id ON trip_days BEGIN
    SELECT RAISE(ABORT, 'a day stays in its trip');
END;
-- `day_source` is 0 without a Day, 1 for a suggested Day, 2 for one the
-- user chose. `day_position` orders the members of a Day.
CREATE TABLE IF NOT EXISTS trip_media (
    trip_id INTEGER NOT NULL REFERENCES trips(id) ON DELETE CASCADE,
    media_id INTEGER NOT NULL REFERENCES media_refs(id),
    position INTEGER NOT NULL,
    day_id INTEGER REFERENCES trip_days(id) ON DELETE SET NULL,
    day_source INTEGER NOT NULL DEFAULT 0,
    day_position INTEGER NOT NULL DEFAULT 0,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (trip_id, media_id)
);
CREATE INDEX IF NOT EXISTS idx_trip_media_media ON trip_media(media_id);
CREATE INDEX IF NOT EXISTS idx_trip_media_day ON trip_media(day_id, day_position);
CREATE TRIGGER IF NOT EXISTS trip_media_day_insert BEFORE INSERT ON trip_media
WHEN NEW.day_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM trip_days WHERE id = NEW.day_id AND trip_id = NEW.trip_id) BEGIN
    SELECT RAISE(ABORT, 'a member can only be in a day of its own trip');
END;
CREATE TRIGGER IF NOT EXISTS trip_media_day_update BEFORE UPDATE OF trip_id, day_id ON trip_media
WHEN NEW.day_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM trip_days WHERE id = NEW.day_id AND trip_id = NEW.trip_id) BEGIN
    SELECT RAISE(ABORT, 'a member can only be in a day of its own trip');
END;
-- GPS tracks linked to a Trip. The path and name stay when the track goes.
CREATE TABLE IF NOT EXISTS trip_tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trip_id INTEGER NOT NULL REFERENCES trips(id) ON DELETE CASCADE,
    day_id INTEGER REFERENCES trip_days(id) ON DELETE SET NULL,
    track_id INTEGER REFERENCES tracks(id) ON DELETE SET NULL,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    UNIQUE (trip_id, path)
);
CREATE TRIGGER IF NOT EXISTS trip_tracks_day_insert BEFORE INSERT ON trip_tracks
WHEN NEW.day_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM trip_days WHERE id = NEW.day_id AND trip_id = NEW.trip_id) BEGIN
    SELECT RAISE(ABORT, 'a track can only be in a day of its own trip');
END;
CREATE TRIGGER IF NOT EXISTS trip_tracks_day BEFORE UPDATE OF trip_id, day_id ON trip_tracks
WHEN NEW.day_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM trip_days WHERE id = NEW.day_id AND trip_id = NEW.trip_id) BEGIN
    SELECT RAISE(ABORT, 'a track can only be in a day of its own trip');
END;
-- Places of a Trip, in `position` order. `kept` is 1 once the user changed
-- one, so suggesting Places again leaves it and its members.
CREATE TABLE IF NOT EXISTS trip_places (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trip_id INTEGER NOT NULL REFERENCES trips(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    local_name TEXT NOT NULL DEFAULT '',
    lat REAL NOT NULL CHECK (lat BETWEEN -90 AND 90),
    lon REAL NOT NULL CHECK (lon BETWEEN -180 AND 180),
    notes TEXT NOT NULL DEFAULT '',
    kept INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_trip_places_trip ON trip_places(trip_id, position);
CREATE TABLE IF NOT EXISTS stories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trip_id INTEGER NOT NULL REFERENCES trips(id) ON DELETE CASCADE,
    title TEXT NOT NULL DEFAULT '',
    subtitle TEXT NOT NULL DEFAULT '',
    author TEXT NOT NULL DEFAULT '',
    show_date INTEGER NOT NULL DEFAULT 1,
    cover_ref INTEGER REFERENCES media_refs(id),
    template TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_stories_trip ON stories(trip_id);
-- `uid` is the block's id through edits and saves; `payload` is JSON of
-- everything but its photos, in the layout of `version`.
CREATE TABLE IF NOT EXISTS story_blocks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    story_id INTEGER NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    uid TEXT NOT NULL,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    version INTEGER NOT NULL,
    payload TEXT NOT NULL,
    UNIQUE (story_id, uid)
);
CREATE INDEX IF NOT EXISTS idx_story_blocks_story ON story_blocks(story_id, position);
CREATE TABLE IF NOT EXISTS story_media (
    block_id INTEGER NOT NULL REFERENCES story_blocks(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    media_id INTEGER NOT NULL REFERENCES media_refs(id),
    caption TEXT NOT NULL DEFAULT '',
    alt TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (block_id, position)
);
CREATE INDEX IF NOT EXISTS idx_story_media_media ON story_media(media_id);
";

/// Creates the tables, and adds the columns later builds added to them.
pub(super) fn upgrade(db: &Database) -> Result<(), String> {
    db.conn.execute_batch(TABLES).map_err(sql_err)?;
    // What the file says about when it was taken (`Capture`): `clock_kind`
    // is 0 unread, 1 a clock reading in `clock` with `clock_offset` when the
    // file records one, 2 an instant in `clock_utc`, 3 none. `clock_read_for`
    // is the modification time of the file that was read.
    for (column, decl) in [
        ("clock_kind", "INTEGER NOT NULL DEFAULT 0"),
        ("clock", "TEXT"),
        ("clock_offset", "INTEGER"),
        ("clock_utc", "INTEGER"),
        ("clock_read_for", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        db.ensure_column("media_refs", column, decl)?;
    }
    // Cameras without a recorded offset were set to Trip time.
    db.ensure_column("trips", "trip_time", "INTEGER NOT NULL DEFAULT 0")?;
    // Why Suggest days left a member without a Day (`Review::code`), until
    // the user places it; 0 when nothing is to review.
    db.ensure_column("trip_media", "review", "INTEGER NOT NULL DEFAULT 0")?;
    // The member's Place: `place_source` is 0 without one, 1 suggested, 2
    // chosen by the user, as `day_source` is for Days.
    db.ensure_column("trip_media", "place_id", "INTEGER REFERENCES trip_places(id) ON DELETE SET NULL")?;
    db.ensure_column("trip_media", "place_source", "INTEGER NOT NULL DEFAULT 0")?;
    // 1 once the author marks the Story finished; a draft until then.
    db.ensure_column("stories", "finished", "INTEGER NOT NULL DEFAULT 0")?;
    // Optional text before and after the Story's blocks.
    db.ensure_column("stories", "introduction", "TEXT NOT NULL DEFAULT ''")?;
    db.ensure_column("stories", "conclusion", "TEXT NOT NULL DEFAULT ''")
}

const CLOCK_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

fn read_capture_row(kind: i64, clock: Option<String>, offset: Option<i32>, utc: Option<i64>) -> Capture {
    match (kind, clock, utc) {
        (1, Some(clock), _) => NaiveDateTime::parse_from_str(&clock, CLOCK_FORMAT).map_or(Capture::Missing, |clock| Capture::Clock { clock, offset }),
        (2, _, Some(utc)) => Capture::Utc(utc),
        (3, ..) => Capture::Missing,
        _ => Capture::Unread,
    }
}

/// Media references nothing uses any more.
const UNUSED_REFS: &str = "\
DELETE FROM media_refs WHERE id NOT IN (SELECT media_id FROM trip_media)
    AND id NOT IN (SELECT media_id FROM story_media)
    AND id NOT IN (SELECT cover_ref FROM trips WHERE cover_ref IS NOT NULL)
    AND id NOT IN (SELECT cover_ref FROM stories WHERE cover_ref IS NOT NULL)";

const TRIP_COLS: &str = "\
t.id, t.title, t.description, t.destination, t.first_day, t.last_day, t.zone, t.cover_ref, t.archived,
t.created_at, t.updated_at, t.trip_time,
(SELECT COUNT(*) FROM trip_media tm WHERE tm.trip_id = t.id),
(SELECT COUNT(*) FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id WHERE tm.trip_id = t.id AND m.photo_id IS NOT NULL),
(SELECT COUNT(*) FROM trip_days d WHERE d.trip_id = t.id),
(SELECT COUNT(*) FROM stories s WHERE s.trip_id = t.id)";

const STORY_COLS: &str = "id, trip_id, title, subtitle, author, show_date, cover_ref, template, archived, revision, updated_at, finished, introduction, conclusion";

fn sql_err(err: rusqlite::Error) -> String {
    err.to_string()
}

fn day_text(day: Option<NaiveDate>) -> Option<String> {
    day.map(|day| day.format("%Y-%m-%d").to_string())
}

fn read_day(text: Option<String>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.as_deref()?, "%Y-%m-%d").ok()
}

fn read_trip(row: &rusqlite::Row) -> rusqlite::Result<Trip> {
    Ok(Trip {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        destination: row.get(3)?,
        first_day: read_day(row.get(4)?),
        last_day: read_day(row.get(5)?),
        zone: row.get(6)?,
        cover: row.get(7)?,
        archived: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        members: row.get(12)?,
        available: row.get(13)?,
        days: row.get(14)?,
        stories: row.get(15)?,
        trip_time: row.get(11)?,
    })
}

fn read_story(row: &rusqlite::Row) -> rusqlite::Result<Story> {
    Ok(Story {
        id: row.get(0)?,
        trip_id: row.get(1)?,
        title: row.get(2)?,
        subtitle: row.get(3)?,
        author: row.get(4)?,
        show_date: row.get(5)?,
        cover: row.get(6)?,
        template: row.get(7)?,
        archived: row.get(8)?,
        revision: row.get(9)?,
        updated_at: row.get(10)?,
        finished: row.get(11)?,
        introduction: row.get(12)?,
        conclusion: row.get(13)?,
    })
}

/// The media reference of catalog photo `photo_id`, made when it has none.
fn media_ref(tx: &Connection, photo_id: i64) -> Result<i64, String> {
    let found = tx
        .query_row("SELECT id FROM media_refs WHERE photo_id = ?1", [photo_id], |row| row.get(0))
        .optional()
        .map_err(sql_err)?;
    if let Some(id) = found {
        return Ok(id);
    }
    let made = tx
        .execute(
            "INSERT INTO media_refs(photo_id, path, filename, bytes, modified_at, file_taken_at, created_at)
             SELECT id, path, filename, bytes, modified_at, file_taken_at, ?2 FROM photos WHERE id = ?1",
            params![photo_id, unix_now()],
        )
        .map_err(sql_err)?;
    if made == 0 {
        return Err(format!("photo {photo_id} is not in the library"));
    }
    Ok(tx.last_insert_rowid())
}

fn touch_trip(tx: &Connection, trip_id: i64) -> Result<(), String> {
    tx.execute("UPDATE trips SET updated_at = ?2 WHERE id = ?1", params![trip_id, unix_now()]).map(|_| ()).map_err(sql_err)
}

/// Numbers the Trip's Places 0, 1, … in their order.
fn renumber_places(tx: &Connection, trip_id: i64) -> Result<(), String> {
    tx.execute(
        "UPDATE trip_places SET position = (SELECT COUNT(*) FROM trip_places pl WHERE pl.trip_id = trip_places.trip_id
            AND (pl.position < trip_places.position OR (pl.position = trip_places.position AND pl.id < trip_places.id)))
         WHERE trip_id = ?1",
        [trip_id],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn check_trip(tx: &Connection, trip_id: i64) -> Result<(), String> {
    let found: Option<i64> = tx.query_row("SELECT id FROM trips WHERE id = ?1", [trip_id], |row| row.get(0)).optional().map_err(sql_err)?;
    found.map(|_| ()).ok_or_else(|| format!("trip {trip_id} is gone"))
}

/// The `WHERE` condition of a [`MemberQuery`] on `trip_media tm` joined
/// with `media_refs m` and `photos p`, with the Trip as `?1` and the Day
/// as `?2`.
fn member_filter(query: &MemberQuery) -> String {
    let mut filter = String::from("tm.trip_id = ?1");
    match query.day {
        DayFilter::Any => filter.push_str(" AND ?2 = ?2"),
        DayFilter::Unassigned => filter.push_str(" AND tm.day_id IS NULL AND ?2 = ?2"),
        DayFilter::Day(_) => filter.push_str(" AND tm.day_id = ?2"),
        DayFilter::Review => filter.push_str(" AND tm.review != 0 AND ?2 = ?2"),
    }
    if query.starred {
        filter.push_str(" AND p.starred != 0");
    }
    if query.no_location {
        filter.push_str(" AND p.id IS NOT NULL AND p.gps_lat IS NULL AND p.pin_lat IS NULL");
    }
    if query.unavailable {
        filter.push_str(" AND p.id IS NULL");
    }
    let used = "EXISTS (SELECT 1 FROM story_media sm JOIN story_blocks sb ON sb.id = sm.block_id \
                JOIN stories s ON s.id = sb.story_id WHERE sm.media_id = tm.media_id AND s.trip_id = tm.trip_id";
    if let Some(place) = query.place {
        filter.push_str(&format!(" AND tm.place_id = {place}"));
    }
    match query.story {
        StoryFilter::Any => {}
        StoryFilter::AnyStory => filter.push_str(&format!(" AND {used})")),
        StoryFilter::Story(id) => filter.push_str(&format!(" AND {used} AND s.id = {id})")),
    }
    filter
}

fn day_param(query: &MemberQuery) -> i64 {
    match query.day {
        DayFilter::Day(id) => id,
        _ => 0,
    }
}

impl Database {
    pub fn create_trip(&self, draft: &TripDraft) -> Result<i64, String> {
        let zone = draft.validate()?;
        let now = unix_now();
        self.conn
            .execute(
                "INSERT INTO trips(title, description, destination, first_day, last_day, zone, trip_time, created_at, updated_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?8, ?7, ?7)",
                params![
                    draft.title.trim(),
                    draft.description,
                    draft.destination.trim(),
                    day_text(draft.first_day),
                    day_text(draft.last_day),
                    zone.code(),
                    now,
                    draft.trip_time
                ],
            )
            .map_err(sql_err)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_trip(&self, id: i64, draft: &TripDraft) -> Result<(), String> {
        let zone = draft.validate()?;
        let changed = self
            .conn
            .execute(
                "UPDATE trips SET title = ?2, description = ?3, destination = ?4, first_day = ?5, last_day = ?6,
                    zone = ?7, updated_at = ?8, trip_time = ?9 WHERE id = ?1",
                params![
                    id,
                    draft.title.trim(),
                    draft.description,
                    draft.destination.trim(),
                    day_text(draft.first_day),
                    day_text(draft.last_day),
                    zone.code(),
                    unix_now(),
                    draft.trip_time
                ],
            )
            .map_err(sql_err)?;
        if changed == 0 {
            return Err(format!("trip {id} is gone"));
        }
        Ok(())
    }

    /// Archives a Trip, or brings it back. Everything in it stays.
    pub fn set_trip_archived(&self, id: i64, archived: bool) {
        let _ = self.conn.execute(
            "UPDATE trips SET archived = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, archived, unix_now()],
        );
    }

    /// Makes member `media_id` the cover, or clears it.
    pub fn set_trip_cover(&self, trip_id: i64, media_id: Option<i64>) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        if let Some(media_id) = media_id {
            let member: Option<i64> = tx
                .query_row("SELECT 1 FROM trip_media WHERE trip_id = ?1 AND media_id = ?2", params![trip_id, media_id], |row| row.get(0))
                .optional()
                .map_err(sql_err)?;
            if member.is_none() {
                return Err(format!("media {media_id} is not in trip {trip_id}"));
            }
        }
        tx.execute("UPDATE trips SET cover_ref = ?2, updated_at = ?3 WHERE id = ?1", params![trip_id, media_id, unix_now()])
            .map_err(sql_err)?;
        tx.execute(UNUSED_REFS, []).map_err(sql_err)?;
        tx.commit().map_err(sql_err)
    }

    /// What [`Database::delete_trip`] would take with it.
    pub fn trip_impact(&self, id: i64) -> TripImpact {
        self.conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM trip_days WHERE trip_id = ?1), (SELECT COUNT(*) FROM trip_media WHERE trip_id = ?1),
                        (SELECT COUNT(*) FROM stories WHERE trip_id = ?1), (SELECT COUNT(*) FROM trip_tracks WHERE trip_id = ?1)",
                [id],
                |row| Ok(TripImpact { days: row.get(0)?, members: row.get(1)?, stories: row.get(2)?, tracks: row.get(3)? }),
            )
            .unwrap_or_default()
    }

    /// Deletes a Trip with its Days, members, track links, and Stories. The
    /// photos, their files, and their catalog data stay.
    pub fn delete_trip(&self, id: i64) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        tx.execute("DELETE FROM trips WHERE id = ?1", [id]).map_err(sql_err)?;
        tx.execute(UNUSED_REFS, []).map_err(sql_err)?;
        tx.commit().map_err(sql_err)
    }

    pub fn trip(&self, id: i64) -> Option<Trip> {
        self.conn.query_row(&format!("SELECT {TRIP_COLS} FROM trips t WHERE t.id = ?1"), [id], read_trip).optional().ok().flatten()
    }

    /// Trips matching `query`, the most recently changed first.
    pub fn trips(&self, query: &TripQuery) -> Vec<Trip> {
        let words: Vec<String> = query.search.split_whitespace().map(|word| format!("%{}%", like_escape(word))).collect();
        let mut sql = format!("SELECT {TRIP_COLS} FROM trips t WHERE t.archived = ?1");
        for at in 0..words.len() {
            let n = at + 2;
            sql.push_str(&format!(
                " AND (t.title LIKE ?{n} ESCAPE '\\' OR t.destination LIKE ?{n} ESCAPE '\\' OR t.description LIKE ?{n} ESCAPE '\\')"
            ));
        }
        sql.push_str(" ORDER BY t.updated_at DESC, t.id DESC");
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&query.archived];
        values.extend(words.iter().map(|word| word as &dyn rusqlite::ToSql));
        self.map_rows(&sql, values.as_slice(), read_trip)
    }

    /// Every library photo as Suggest trips groups them, each with its
    /// folder's name and whether a Trip has it already.
    pub fn trip_shots(&self) -> Vec<Shot> {
        self.map_rows(
            "SELECT p.id, p.taken_at, p.path, p.camera, p.gps_lat IS NOT NULL OR p.pin_lat IS NOT NULL,
                    EXISTS (SELECT 1 FROM media_refs m JOIN trip_media tm ON tm.media_id = m.id WHERE m.photo_id = p.id)
             FROM photos p",
            [],
            |row| {
                let path: String = row.get(2)?;
                let folder = std::path::Path::new(&path).parent().and_then(|dir| dir.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                Ok(Shot { id: row.get(0)?, taken_at: row.get(1)?, folder, camera: row.get(3)?, located: row.get(4)?, in_trip: row.get(5)? })
            },
        )
    }

    /// What [`Database::add_to_trip`] would do with these photos.
    pub fn preview_add_to_trip(&self, trip_id: i64, photo_ids: &[i64]) -> AddPreview {
        let mut preview = AddPreview::default();
        let member = self.conn.prepare(
            "SELECT 1 FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id WHERE tm.trip_id = ?1 AND m.photo_id = ?2",
        );
        let other = self.conn.prepare(
            "SELECT 1 FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id WHERE tm.trip_id != ?1 AND m.photo_id = ?2",
        );
        let photo = self.conn.prepare(
            "SELECT path, taken_at, gps_lat IS NOT NULL OR pin_lat IS NOT NULL, bytes, width, height, file_taken_at FROM photos WHERE id = ?1",
        );
        let (Ok(mut member), Ok(mut other), Ok(mut photo)) = (member, other, photo) else {
            return preview;
        };
        let mut seen = HashSet::new();
        let mut copies = HashSet::new();
        for &id in photo_ids.iter().filter(|id| seen.insert(**id)) {
            let Ok((path, taken_at, located, copy)) = photo.query_row([id], |row| {
                let copy: (i64, i64, i64, i64) = (row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?);
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, bool>(2)?, copy))
            }) else {
                continue;
            };
            if member.exists(params![trip_id, id]).unwrap_or(false) {
                preview.already += 1;
                continue;
            }
            if is_video_path(std::path::Path::new(&path)) {
                preview.videos += 1;
            } else {
                preview.photos += 1;
            }
            if other.exists(params![trip_id, id]).unwrap_or(false) {
                preview.in_other_trip += 1;
            }
            if located {
                preview.located += 1;
            }
            if copy.0 > 0 && !copies.insert(copy) {
                preview.duplicates += 1;
            }
            if taken_at == 0 {
                preview.no_time += 1;
                continue;
            }
            preview.taken = Some(preview.taken.map_or((taken_at, taken_at), |(first, last)| (first.min(taken_at), last.max(taken_at))));
        }
        preview
    }

    /// Adds photos to a Trip in the order given, after its members. Photos
    /// already in it stay where they are. Returns the media reference of each
    /// photo, in the order given, whether it was added now or before.
    pub fn add_to_trip(&self, trip_id: i64, photo_ids: &[i64]) -> Result<Vec<i64>, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        check_trip(&tx, trip_id)?;
        let mut next: i64 = tx
            .query_row("SELECT coalesce(max(position), -1) + 1 FROM trip_media WHERE trip_id = ?1", [trip_id], |row| row.get(0))
            .map_err(sql_err)?;
        let mut refs = Vec::with_capacity(photo_ids.len());
        {
            let mut insert = tx
                .prepare("INSERT OR IGNORE INTO trip_media(trip_id, media_id, position, added_at) VALUES(?1, ?2, ?3, ?4)")
                .map_err(sql_err)?;
            let now = unix_now();
            for &photo_id in photo_ids {
                let media_id = media_ref(&tx, photo_id)?;
                if insert.execute(params![trip_id, media_id, next, now]).map_err(sql_err)? > 0 {
                    next += 1;
                }
                refs.push(media_id);
            }
        }
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)?;
        Ok(refs)
    }

    /// Takes members out of a Trip. Story blocks that use them keep them.
    pub fn remove_from_trip(&self, trip_id: i64, media_ids: &[i64]) -> Result<usize, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let mut removed = 0;
        {
            let mut delete = tx.prepare("DELETE FROM trip_media WHERE trip_id = ?1 AND media_id = ?2").map_err(sql_err)?;
            for media_id in media_ids {
                removed += delete.execute(params![trip_id, media_id]).map_err(sql_err)?;
            }
            tx.execute("UPDATE trips SET cover_ref = NULL WHERE id = ?1 AND cover_ref NOT IN (SELECT media_id FROM trip_media WHERE trip_id = ?1)", [trip_id])
                .map_err(sql_err)?;
        }
        touch_trip(&tx, trip_id)?;
        tx.execute(UNUSED_REFS, []).map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        Ok(removed)
    }

    /// Takes catalog photos out of a Trip; see [`Database::remove_from_trip`].
    pub fn remove_photos_from_trip(&self, trip_id: i64, photo_ids: &[i64]) -> Result<usize, String> {
        let media: Vec<i64> = photo_ids
            .iter()
            .filter_map(|id| self.conn.query_row("SELECT id FROM media_refs WHERE photo_id = ?1", [id], |row| row.get(0)).ok())
            .collect();
        self.remove_from_trip(trip_id, &media)
    }

    /// Members matching `query`, `limit` of them from `offset`: in a Day,
    /// in the Day's order; otherwise in the order they joined the Trip.
    pub fn trip_members(&self, trip_id: i64, query: &MemberQuery, offset: usize, limit: usize) -> Vec<Member> {
        let order = match query.day {
            DayFilter::Day(_) => "tm.day_position, tm.position",
            _ => "tm.position",
        };
        let sql = format!(
            "SELECT tm.media_id, m.photo_id, m.path, m.filename, tm.day_id, tm.day_source
             FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id LEFT JOIN photos p ON p.id = m.photo_id
             WHERE {} ORDER BY {order} LIMIT ?3 OFFSET ?4",
            member_filter(query)
        );
        self.map_rows(&sql, params![trip_id, day_param(query), limit as i64, offset as i64], |row| {
            let path: String = row.get(2)?;
            Ok(Member {
                media_id: row.get(0)?,
                photo_id: row.get(1)?,
                is_video: is_video_path(std::path::Path::new(&path)),
                path,
                filename: row.get(3)?,
                day_id: row.get(4)?,
                day_source: DaySource::from_code(row.get(5)?),
            })
        })
    }

    /// The catalog photos of the members matching `query`, for the photo
    /// grid: in a Day, in the Day's order; otherwise in `sort` order.
    /// Unavailable members have none.
    pub fn trip_photos(&self, trip_id: i64, query: &MemberQuery, sort: Sort) -> Vec<Photo> {
        let order = match query.day {
            DayFilter::Day(_) => "tm.day_position, tm.position",
            _ => order_sql(sort),
        };
        let sql = format!(
            "SELECT {PHOTO_COLS} FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id JOIN photos p ON p.id = m.photo_id
             WHERE {} ORDER BY {order}",
            member_filter(query)
        );
        self.map_rows(&sql, params![trip_id, day_param(query)], read_photo)
    }

    /// The catalog photo of the file at `path`, to link an unavailable member to.
    pub fn photo_at_path(&self, path: &std::path::Path) -> Option<i64> {
        let path = path_text(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        self.conn.query_row("SELECT id FROM photos WHERE path = ?1", [path], |row| row.get(0)).optional().ok().flatten()
    }

    /// How many members match `query`; the same members [`Database::trip_members`] lists.
    pub fn count_trip_members(&self, trip_id: i64, query: &MemberQuery) -> i64 {
        let sql = format!(
            "SELECT COUNT(*) FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id LEFT JOIN photos p ON p.id = m.photo_id WHERE {}",
            member_filter(query)
        );
        self.conn.query_row(&sql, params![trip_id, day_param(query)], |row| row.get(0)).unwrap_or(0)
    }

    /// Adds an empty Day after the Trip's last one.
    pub fn add_trip_day(&self, trip_id: i64, day: Option<NaiveDate>, title: &str) -> Result<i64, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        check_trip(&tx, trip_id)?;
        tx.execute(
            "INSERT INTO trip_days(trip_id, position, day, title)
             VALUES(?1, (SELECT coalesce(max(position), -1) + 1 FROM trip_days WHERE trip_id = ?1), ?2, ?3)",
            params![trip_id, day_text(day), title.trim()],
        )
        .map_err(sql_err)?;
        let id = tx.last_insert_rowid();
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)?;
        Ok(id)
    }

    pub fn update_trip_day(&self, id: i64, day: Option<NaiveDate>, title: &str, notes: &str) -> Result<(), String> {
        let changed = self
            .conn
            .execute(
                "UPDATE trip_days SET day = ?2, title = ?3, notes = ?4 WHERE id = ?1",
                params![id, day_text(day), title.trim(), notes],
            )
            .map_err(sql_err)?;
        if changed == 0 {
            return Err(format!("day {id} is gone"));
        }
        Ok(())
    }

    /// Deletes a Day. Its members stay in the Trip, without a Day.
    /// The Trip's Places in order, each with how many members are at it.
    pub fn trip_places(&self, trip_id: i64) -> Vec<TripPlace> {
        self.map_rows(
            "SELECT pl.id, pl.trip_id, pl.position, pl.name, pl.local_name, pl.lat, pl.lon, pl.notes, pl.kept,
                    (SELECT COUNT(*) FROM trip_media tm WHERE tm.place_id = pl.id)
             FROM trip_places pl WHERE pl.trip_id = ?1 ORDER BY pl.position, pl.id",
            [trip_id],
            |row| {
                Ok(TripPlace {
                    id: row.get(0)?,
                    trip_id: row.get(1)?,
                    position: row.get(2)?,
                    name: row.get(3)?,
                    local_name: row.get(4)?,
                    lat: row.get(5)?,
                    lon: row.get(6)?,
                    notes: row.get(7)?,
                    kept: row.get(8)?,
                    members: row.get(9)?,
                })
            },
        )
    }

    /// Names a Place and keeps it, with its members, when Places are suggested again.
    pub fn update_trip_place(&self, id: i64, name: &str, local_name: &str, notes: &str) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let changed = tx
            .execute(
                "UPDATE trip_places SET name = ?2, local_name = ?3, notes = ?4, kept = 1 WHERE id = ?1",
                params![id, name.trim(), local_name.trim(), notes],
            )
            .map_err(sql_err)?;
        if changed == 0 {
            return Err(format!("place {id} is gone"));
        }
        tx.execute("UPDATE trip_media SET place_source = 2 WHERE place_id = ?1", [id]).map_err(sql_err)?;
        let trip_id: i64 = tx.query_row("SELECT trip_id FROM trip_places WHERE id = ?1", [id], |row| row.get(0)).map_err(sql_err)?;
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)
    }

    /// Puts the Trip's Places in the order of `place_ids`, which must name
    /// each of them once; the order is kept, as a change by the user.
    pub fn reorder_trip_places(&self, trip_id: i64, place_ids: &[i64]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let mut current: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM trip_places WHERE trip_id = ?1").map_err(sql_err)?;
            let rows = stmt.query_map([trip_id], |row| row.get(0)).map_err(sql_err)?;
            rows.collect::<Result<_, _>>().map_err(sql_err)?
        };
        let mut wanted = place_ids.to_vec();
        current.sort_unstable();
        wanted.sort_unstable();
        if current != wanted {
            return Err("the places changed meanwhile".into());
        }
        for (position, id) in place_ids.iter().enumerate() {
            tx.execute("UPDATE trip_places SET position = ?2, kept = 1 WHERE id = ?1", params![id, position as i64]).map_err(sql_err)?;
        }
        tx.execute("UPDATE trip_media SET place_source = 2 WHERE trip_id = ?1 AND place_id IS NOT NULL", [trip_id]).map_err(sql_err)?;
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)
    }

    /// Removes a Place; its members stay in the Trip, at no Place.
    pub fn delete_trip_place(&self, id: i64) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let trip_id: Option<i64> =
            tx.query_row("SELECT trip_id FROM trip_places WHERE id = ?1", [id], |row| row.get(0)).optional().map_err(sql_err)?;
        let Some(trip_id) = trip_id else {
            return Ok(());
        };
        tx.execute("UPDATE trip_media SET place_id = NULL, place_source = 0 WHERE place_id = ?1", [id]).map_err(sql_err)?;
        tx.execute("DELETE FROM trip_places WHERE id = ?1", [id]).map_err(sql_err)?;
        renumber_places(&tx, trip_id)?;
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)
    }

    /// Puts the Trip's members with a position at Places, grouping them as
    /// [`cluster_places`] does. Places the user changed stay, with their
    /// members, and take in new groups within `distance` of them; the other
    /// suggestions are made again, after the kept ones in the order first
    /// visited. Camera GPS and pins are read, never changed. Returns how many
    /// Places the Trip has.
    pub fn suggest_places(&self, trip_id: i64, distance: f64, gap: i64) -> Result<usize, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        check_trip(&tx, trip_id)?;
        tx.execute("UPDATE trip_media SET place_id = NULL, place_source = 0 WHERE trip_id = ?1 AND place_source = 1", [trip_id])
            .map_err(sql_err)?;
        tx.execute(
            "DELETE FROM trip_places WHERE trip_id = ?1 AND kept = 0
               AND NOT EXISTS (SELECT 1 FROM trip_media tm WHERE tm.place_id = trip_places.id)",
            [trip_id],
        )
        .map_err(sql_err)?;
        let points: Vec<Located> = {
            let mut stmt = tx
                .prepare(
                    "SELECT tm.media_id, p.taken_at, coalesce(p.pin_lat, p.gps_lat), coalesce(p.pin_lon, p.gps_lon)
                     FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id JOIN photos p ON p.id = m.photo_id
                     WHERE tm.trip_id = ?1 AND tm.place_id IS NULL AND coalesce(p.pin_lat, p.gps_lat) IS NOT NULL",
                )
                .map_err(sql_err)?;
            let rows = stmt
                .query_map([trip_id], |row| Ok(Located { media_id: row.get(0)?, at: row.get(1)?, lat: row.get(2)?, lon: row.get(3)? }))
                .map_err(sql_err)?;
            rows.collect::<Result<_, _>>().map_err(sql_err)?
        };
        let kept: Vec<(i64, f64, f64)> = {
            let mut stmt = tx.prepare("SELECT id, lat, lon FROM trip_places WHERE trip_id = ?1").map_err(sql_err)?;
            let rows = stmt.query_map([trip_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).map_err(sql_err)?;
            rows.collect::<Result<_, _>>().map_err(sql_err)?
        };
        let mut next: i64 = tx
            .query_row("SELECT coalesce(max(position) + 1, 0) FROM trip_places WHERE trip_id = ?1", [trip_id], |row| row.get(0))
            .map_err(sql_err)?;
        let mut clusters = cluster_places(&points, distance, gap);
        clusters.sort_by_key(|cluster| cluster.first);
        for cluster in clusters {
            let at = crate::track::Point { lat: cluster.lat, lon: cluster.lon, time: None };
            let near = kept.iter().find(|(_, lat, lon)| crate::track::distance_m(&at, &crate::track::Point { lat: *lat, lon: *lon, time: None }) <= distance);
            let place_id = match near {
                Some((id, ..)) => *id,
                None if valid_position(cluster.lat, cluster.lon) => {
                    tx.execute(
                        "INSERT INTO trip_places(trip_id, position, lat, lon) VALUES(?1, ?2, ?3, ?4)",
                        params![trip_id, next, cluster.lat, cluster.lon],
                    )
                    .map_err(sql_err)?;
                    next += 1;
                    tx.last_insert_rowid()
                }
                None => continue,
            };
            for media_id in &cluster.members {
                tx.execute(
                    "UPDATE trip_media SET place_id = ?3, place_source = 1 WHERE trip_id = ?1 AND media_id = ?2",
                    params![trip_id, media_id, place_id],
                )
                .map_err(sql_err)?;
            }
        }
        renumber_places(&tx, trip_id)?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM trip_places WHERE trip_id = ?1", [trip_id], |row| row.get(0)).map_err(sql_err)?;
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)?;
        Ok(count as usize)
    }

    pub fn delete_trip_day(&self, id: i64) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let trip_id: Option<i64> =
            tx.query_row("SELECT trip_id FROM trip_days WHERE id = ?1", [id], |row| row.get(0)).optional().map_err(sql_err)?;
        let Some(trip_id) = trip_id else {
            return Ok(());
        };
        tx.execute("UPDATE trip_media SET day_id = NULL, day_source = 0, day_position = 0 WHERE day_id = ?1", [id])
            .map_err(sql_err)?;
        tx.execute("DELETE FROM trip_days WHERE id = ?1", [id]).map_err(sql_err)?;
        tx.execute(
            "UPDATE trip_days SET position = (SELECT COUNT(*) FROM trip_days d WHERE d.trip_id = trip_days.trip_id
                AND (d.position < trip_days.position OR (d.position = trip_days.position AND d.id < trip_days.id)))
             WHERE trip_id = ?1",
            [trip_id],
        )
        .map_err(sql_err)?;
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)
    }

    /// The Trip's Days in order, each with its number of members.
    pub fn trip_days(&self, trip_id: i64) -> Vec<TripDay> {
        self.map_rows(
            "SELECT d.id, d.trip_id, d.position, d.day, d.title, d.notes,
                    (SELECT COUNT(*) FROM trip_media tm WHERE tm.day_id = d.id)
             FROM trip_days d WHERE d.trip_id = ?1 ORDER BY d.position, d.id",
            [trip_id],
            |row| {
                Ok(TripDay {
                    id: row.get(0)?,
                    trip_id: row.get(1)?,
                    position: row.get(2)?,
                    day: read_day(row.get(3)?),
                    title: row.get(4)?,
                    notes: row.get(5)?,
                    members: row.get(6)?,
                })
            },
        )
    }

    /// Puts the Trip's Days in the order of `day_ids`, which must name each of them once.
    pub fn reorder_trip_days(&self, trip_id: i64, day_ids: &[i64]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let mut current: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM trip_days WHERE trip_id = ?1").map_err(sql_err)?;
            let rows = stmt.query_map([trip_id], |row| row.get(0)).map_err(sql_err)?;
            rows.collect::<Result<_, _>>().map_err(sql_err)?
        };
        let mut given = day_ids.to_vec();
        current.sort_unstable();
        given.sort_unstable();
        if current != given {
            return Err(format!("the days given are not those of trip {trip_id}"));
        }
        for (position, id) in day_ids.iter().enumerate() {
            tx.execute("UPDATE trip_days SET position = ?2 WHERE id = ?1", params![id, position as i64]).map_err(sql_err)?;
        }
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)
    }

    /// Where these members are now, to put them back with [`Database::restore_day_assignments`].
    pub fn day_assignments(&self, trip_id: i64, media_ids: &[i64]) -> Vec<DayAssignment> {
        let Ok(mut stmt) =
            self.conn.prepare("SELECT day_id, day_source, day_position, review FROM trip_media WHERE trip_id = ?1 AND media_id = ?2")
        else {
            return Vec::new();
        };
        media_ids
            .iter()
            .filter_map(|&media_id| {
                stmt.query_row(params![trip_id, media_id], |row| {
                    Ok(DayAssignment {
                        media_id,
                        day_id: row.get(0)?,
                        source: DaySource::from_code(row.get(1)?),
                        position: row.get(2)?,
                        review: Review::from_code(row.get(3)?),
                    })
                })
                .ok()
            })
            .collect()
    }

    /// Moves members to Day `day_id` (or out of any Day), after the Day's
    /// members and in the order given. A suggestion leaves members the user
    /// put in a Day where they are; the user placing members ends their
    /// review. Returns the members moved and where each
    /// was before, for Undo.
    pub fn move_to_day(&self, trip_id: i64, day_id: Option<i64>, media_ids: &[i64], source: DaySource) -> Result<Vec<DayAssignment>, String> {
        let before = self.day_assignments(trip_id, media_ids);
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        if let Some(day_id) = day_id {
            let owner: Option<i64> =
                tx.query_row("SELECT trip_id FROM trip_days WHERE id = ?1", [day_id], |row| row.get(0)).optional().map_err(sql_err)?;
            if owner != Some(trip_id) {
                return Err(format!("day {day_id} is not in trip {trip_id}"));
            }
        }
        let mut next: i64 = tx
            .query_row("SELECT coalesce(max(day_position), -1) + 1 FROM trip_media WHERE day_id IS ?1", [day_id], |row| row.get(0))
            .map_err(sql_err)?;
        let mut moved = Vec::new();
        {
            let mut update = tx
                .prepare(
                    "UPDATE trip_media SET day_id = ?3, day_source = ?4, day_position = ?5, review = CASE WHEN ?6 = 0 THEN 0 ELSE review END
                     WHERE trip_id = ?1 AND media_id = ?2 AND (?6 = 0 OR day_source != 2)",
                )
                .map_err(sql_err)?;
            let code = if day_id.is_some() { source.code() } else { 0 };
            let suggestion = source == DaySource::Suggested;
            for assignment in before {
                if update.execute(params![trip_id, assignment.media_id, day_id, code, next, suggestion]).map_err(sql_err)? > 0 {
                    next += 1;
                    moved.push(assignment);
                }
            }
        }
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)?;
        Ok(moved)
    }

    /// Puts members back where [`Database::day_assignments`] or [`Database::move_to_day`] found them.
    pub fn restore_day_assignments(&self, trip_id: i64, assignments: &[DayAssignment]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        {
            let mut update = tx
                .prepare("UPDATE trip_media SET day_id = ?3, day_source = ?4, day_position = ?5, review = ?6 WHERE trip_id = ?1 AND media_id = ?2")
                .map_err(sql_err)?;
            for assignment in assignments {
                let code = assignment.source.map_or(0, DaySource::code);
                update
                    .execute(params![trip_id, assignment.media_id, assignment.day_id, code, assignment.position, assignment.review.map_or(0, Review::code)])
                    .map_err(sql_err)?;
            }
        }
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)
    }

    /// Orders a Day's members as `media_ids`, which must all be in it;
    /// members not named follow in their current order.
    pub fn order_day(&self, day_id: i64, media_ids: &[i64]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let current: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT media_id FROM trip_media WHERE day_id = ?1 ORDER BY day_position, position").map_err(sql_err)?;
            let rows = stmt.query_map([day_id], |row| row.get(0)).map_err(sql_err)?;
            rows.collect::<Result<_, _>>().map_err(sql_err)?
        };
        let named: HashSet<i64> = media_ids.iter().copied().collect();
        if let Some(stray) = media_ids.iter().find(|id| !current.contains(id)) {
            return Err(format!("media {stray} is not in day {day_id}"));
        }
        let order = media_ids.iter().copied().chain(current.into_iter().filter(|id| !named.contains(id)));
        for (position, media_id) in order.enumerate() {
            tx.execute("UPDATE trip_media SET day_position = ?3 WHERE day_id = ?1 AND media_id = ?2", params![day_id, media_id, position as i64])
                .map_err(sql_err)?;
        }
        tx.commit().map_err(sql_err)
    }

    /// Links a library track to a Trip, and optionally to one of its Days.
    pub fn link_trip_track(&self, trip_id: i64, track_id: i64, day_id: Option<i64>) -> Result<i64, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        check_trip(&tx, trip_id)?;
        if let Some(day_id) = day_id {
            let owner: Option<i64> =
                tx.query_row("SELECT trip_id FROM trip_days WHERE id = ?1", [day_id], |row| row.get(0)).optional().map_err(sql_err)?;
            if owner != Some(trip_id) {
                return Err(format!("day {day_id} is not in trip {trip_id}"));
            }
        }
        let changed = tx
            .execute(
                "INSERT INTO trip_tracks(trip_id, day_id, track_id, path, name) SELECT ?1, ?2, id, path, name FROM tracks WHERE id = ?3
                 ON CONFLICT(trip_id, path) DO UPDATE SET day_id = excluded.day_id, track_id = excluded.track_id, name = excluded.name",
                params![trip_id, day_id, track_id],
            )
            .map_err(sql_err)?;
        if changed == 0 {
            return Err(format!("track {track_id} is not in the library"));
        }
        let id = tx
            .query_row(
                "SELECT t.id FROM trip_tracks t JOIN tracks k ON k.path = t.path WHERE t.trip_id = ?1 AND k.id = ?2",
                params![trip_id, track_id],
                |row| row.get(0),
            )
            .map_err(sql_err)?;
        touch_trip(&tx, trip_id)?;
        tx.commit().map_err(sql_err)?;
        Ok(id)
    }

    /// The library's tracks to pick from, without their points: (id,
    /// name, start time), the most recent first.
    pub fn track_choices(&self) -> Vec<(i64, String, Option<i64>)> {
        self.map_rows("SELECT id, name, started_at FROM tracks ORDER BY started_at DESC, name", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
    }

    /// The photo to show for a Trip: its cover, or else its first member
    /// in the library.
    pub fn trip_cover_photo(&self, trip_id: i64) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT m.photo_id FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id JOIN trips t ON t.id = tm.trip_id
                 WHERE tm.trip_id = ?1 AND m.photo_id IS NOT NULL
                 ORDER BY (tm.media_id = t.cover_ref) DESC, tm.position LIMIT 1",
                [trip_id],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// The catalog photo a media reference points to now.
    pub fn media_photo(&self, media_id: i64) -> Option<i64> {
        self.conn.query_row("SELECT photo_id FROM media_refs WHERE id = ?1", [media_id], |row| row.get(0)).optional().ok().flatten()
    }

    pub fn unlink_trip_track(&self, link_id: i64) {
        let _ = self.conn.execute("DELETE FROM trip_tracks WHERE id = ?1", [link_id]);
    }

    pub fn trip_tracks(&self, trip_id: i64) -> Vec<TripTrack> {
        self.map_rows(
            "SELECT id, day_id, track_id, path, name FROM trip_tracks WHERE trip_id = ?1 ORDER BY name COLLATE NOCASE, id",
            [trip_id],
            |row| Ok(TripTrack { id: row.get(0)?, day_id: row.get(1)?, track_id: row.get(2)?, path: row.get(3)?, name: row.get(4)? }),
        )
    }

    /// Links unavailable Trip and Story photos, and Trip tracks, to the
    /// library again: a photo at the same path, or else the one photo that
    /// arrived since with the same size, modification time, and date. When
    /// several photos match, the reference waits for the user to choose.
    /// Returns how many were linked.
    pub fn relink_trip_media(&self) -> usize {
        let unlinked: Vec<(i64, String, i64, i64, i64, i64)> = self.map_rows(
            "SELECT id, path, bytes, modified_at, file_taken_at, coalesce(unlinked_at, 0) FROM media_refs WHERE photo_id IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        );
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return 0;
        };
        let mut linked = 0;
        if !unlinked.is_empty() {
            let free = "NOT EXISTS (SELECT 1 FROM media_refs r WHERE r.photo_id = p.id)";
            let by_path = tx.prepare(&format!("SELECT p.id FROM photos p WHERE p.path = ?1 AND {free}"));
            let moved = tx.prepare(&format!(
                "SELECT p.id FROM photos p WHERE p.bytes = ?1 AND p.modified_at = ?2 AND p.file_taken_at = ?3
                 AND p.added_at >= ?4 AND {free} LIMIT 2"
            ));
            let link = tx.prepare(
                "UPDATE media_refs SET photo_id = p.id, path = p.path, filename = p.filename, bytes = p.bytes,
                    modified_at = p.modified_at, file_taken_at = p.file_taken_at, unlinked_at = NULL
                 FROM (SELECT * FROM photos WHERE id = ?2) p WHERE media_refs.id = ?1",
            );
            let (Ok(mut by_path), Ok(mut moved), Ok(mut link)) = (by_path, moved, link) else {
                return 0;
            };
            for (id, path, bytes, modified_at, taken_at, unlinked_at) in unlinked {
                let mut photo: Option<i64> = by_path.query_row([&path], |row| row.get(0)).optional().ok().flatten();
                if photo.is_none() {
                    let found: Vec<i64> = moved
                        .query_map(params![bytes, modified_at, taken_at, unlinked_at - 86_400], |row| row.get(0))
                        .map(|rows| rows.filter_map(Result::ok).collect())
                        .unwrap_or_default();
                    if let [one] = found[..] {
                        photo = Some(one);
                    }
                }
                if let Some(photo) = photo {
                    linked += link.execute(params![id, photo]).unwrap_or(0);
                }
            }
        }
        linked += tx
            .execute(
                "UPDATE trip_tracks SET track_id = (SELECT k.id FROM tracks k WHERE k.path = trip_tracks.path)
                 WHERE track_id IS NULL AND EXISTS (SELECT 1 FROM tracks k WHERE k.path = trip_tracks.path)",
                [],
            )
            .unwrap_or(0);
        if tx.commit().is_err() {
            return 0;
        }
        linked
    }

    /// Links an unavailable reference to a photo the user chose. The photo
    /// can't already stand for another reference.
    pub fn relink_media_to(&self, media_id: i64, photo_id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE media_refs SET photo_id = p.id, path = p.path, filename = p.filename, bytes = p.bytes,
                    modified_at = p.modified_at, file_taken_at = p.file_taken_at, unlinked_at = NULL
                 FROM (SELECT * FROM photos WHERE id = ?2) p WHERE media_refs.id = ?1 AND media_refs.photo_id IS NULL",
                params![media_id, photo_id],
            )
            .map_err(sql_err)
            .and_then(|changed| if changed == 0 { Err(format!("media {media_id} can't be linked to photo {photo_id}")) } else { Ok(()) })
    }

    /// How many members are to review, by reason.
    pub fn review_counts(&self, trip_id: i64) -> Vec<(Review, usize)> {
        self.map_rows(
            "SELECT review, COUNT(*) FROM trip_media WHERE trip_id = ?1 AND review != 0 GROUP BY review ORDER BY review",
            [trip_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .into_iter()
        .filter_map(|(code, count)| Some((Review::from_code(code)?, count as usize)))
        .collect()
    }

    /// Members whose file hasn't been read for its capture time, or has
    /// changed since: (media reference, path, modification time).
    pub fn captures_to_read(&self, trip_id: i64) -> Vec<(i64, String, i64)> {
        self.map_rows(
            "SELECT m.id, p.path, p.modified_at FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id
             JOIN photos p ON p.id = m.photo_id
             WHERE tm.trip_id = ?1 AND (m.clock_kind = 0 OR m.clock_read_for != p.modified_at)
             ORDER BY tm.position",
            [trip_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
    }

    /// Keeps what a file of modification time `read_for` says about when it was taken.
    pub fn store_capture(&self, media_id: i64, capture: Capture, read_for: i64) {
        let (kind, clock, offset, utc) = match capture {
            Capture::Unread => (0, None, None, None),
            Capture::Clock { clock, offset } => (1, Some(clock.format(CLOCK_FORMAT).to_string()), offset, None),
            Capture::Utc(utc) => (2, None, None, Some(utc)),
            Capture::Missing => (3, None, None, None),
        };
        let _ = self.conn.execute(
            "UPDATE media_refs SET clock_kind = ?2, clock = ?3, clock_offset = ?4, clock_utc = ?5, clock_read_for = ?6 WHERE id = ?1",
            params![media_id, kind, clock, offset, utc, read_for],
        );
    }

    /// Suggests a Day for each member from its capture time in the Trip's
    /// zone, making a Day for each date that has none, after the others and
    /// in date order. Members the user put in a Day stay there; members
    /// whose day is uncertain lose any earlier suggestion and are counted
    /// for review. The catalog's dates are not changed.
    pub fn suggest_days(&self, trip_id: i64) -> Result<DaySuggestion, String> {
        let trip = self.trip(trip_id).ok_or_else(|| format!("trip {trip_id} is gone"))?;
        let zone = TripZone::parse(&trip.zone).ok_or_else(|| tr("Pick the trip's time zone").to_string())?;
        let rows: Vec<(i64, i64, Capture, i64)> = self.map_rows(
            "SELECT tm.media_id, tm.day_source, m.clock_kind, m.clock, m.clock_offset, m.clock_utc,
                    coalesce(p.taken_at - p.file_taken_at, 0)
             FROM trip_media tm JOIN media_refs m ON m.id = tm.media_id LEFT JOIN photos p ON p.id = m.photo_id
             WHERE tm.trip_id = ?1 ORDER BY tm.position",
            [trip_id],
            |row| Ok((row.get(0)?, row.get(1)?, read_capture_row(row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?), row.get(6)?)),
        );
        let mut outcome = DaySuggestion::default();
        let mut by_date: std::collections::BTreeMap<NaiveDate, Vec<i64>> = Default::default();
        let mut review: std::collections::HashMap<Review, usize> = Default::default();
        let mut unsure = Vec::new();
        let mut reasons = Vec::new();
        for (media_id, source, capture, shift) in rows {
            if source == DaySource::User.code() {
                outcome.kept += 1;
                continue;
            }
            match capture_day(capture, shift, &zone, trip.trip_time) {
                Ok(date) => {
                    by_date.entry(date).or_default().push(media_id);
                    reasons.push((media_id, 0));
                }
                Err(reason) => {
                    *review.entry(reason).or_default() += 1;
                    unsure.push(media_id);
                    reasons.push((media_id, reason.code()));
                }
            }
        }
        let days = self.trip_days(trip_id);
        for (date, members) in by_date {
            let day_id = match days.iter().find(|day| day.day == Some(date)) {
                Some(day) => day.id,
                None => {
                    outcome.new_days += 1;
                    self.add_trip_day(trip_id, Some(date), "")?
                }
            };
            outcome.assigned += self.move_to_day(trip_id, Some(day_id), &members, DaySource::Suggested)?.len();
        }
        self.move_to_day(trip_id, None, &unsure, DaySource::Suggested)?;
        for (media_id, code) in reasons {
            self.conn
                .execute("UPDATE trip_media SET review = ?3 WHERE trip_id = ?1 AND media_id = ?2", params![trip_id, media_id, code])
                .map_err(sql_err)?;
        }
        outcome.review = review.into_iter().collect();
        outcome.review.sort_by_key(|(reason, _)| *reason as u8);
        Ok(outcome)
    }

    pub fn create_story(&self, trip_id: i64, title: &str) -> Result<i64, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        check_trip(&tx, trip_id)?;
        let id = insert_story(&tx, trip_id, title)?;
        tx.commit().map_err(sql_err)?;
        Ok(id)
    }

    /// A Story with a heading for each of `day_ids`, in that order, followed
    /// by a photo block for each of the Day's photos in the Day's order.
    /// Videos are left out. Headings are the Day titles, or `date_heading`
    /// of the Day's date; Days with neither get none. Later changes to the
    /// Days don't change the Story. Returns the Story and the videos left out.
    pub fn create_story_from_days(
        &self,
        trip_id: i64,
        title: &str,
        day_ids: &[i64],
        date_heading: impl Fn(NaiveDate) -> String,
    ) -> Result<(i64, usize), String> {
        let days = self.trip_days(trip_id);
        let mut blocks = Vec::new();
        let mut videos = 0;
        for day_id in day_ids {
            let day = days.iter().find(|day| day.id == *day_id).ok_or_else(|| format!("day {day_id} is not in trip {trip_id}"))?;
            let heading = if day.title.is_empty() { day.day.map(&date_heading).unwrap_or_default() } else { day.title.clone() };
            if !heading.is_empty() {
                blocks.push(StoryBlock::new(Block::Heading(heading)));
            }
            let query = MemberQuery { day: DayFilter::Day(day.id), ..MemberQuery::default() };
            for member in self.trip_members(trip_id, &query, 0, usize::MAX >> 1) {
                if member.is_video {
                    videos += 1;
                } else {
                    blocks.push(StoryBlock::new(Block::Photo(MediaUse::new(member.media_id))));
                }
            }
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let id = insert_story(&tx, trip_id, title)?;
        write_blocks(&tx, id, &blocks)?;
        tx.commit().map_err(sql_err)?;
        Ok((id, videos))
    }

    /// Adds a Trip package as a new Trip, with the package's Days, its
    /// photos as members in their Days, and its Stories, in one transaction.
    /// `files` says where each carried photo was written; until a scan finds
    /// them there, the photos wait unavailable, as references with those
    /// paths as hints, and are linked by the scan. Photos the package
    /// carried as unavailable stay so. `existing` names, by key, photos
    /// already in the library that stand for carried ones. Returns the new
    /// Trip.
    pub fn import_trip(
        &self,
        package: &crate::trip_package::Package,
        files: &std::collections::HashMap<i64, std::path::PathBuf>,
        existing: &std::collections::HashMap<i64, i64>,
    ) -> Result<i64, String> {
        let zone = TripZone::parse(&package.trip.zone)
            .ok_or_else(|| format!("unknown time zone {}", package.trip.zone))?;
        let trip = &package.trip;
        let now = unix_now();
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        tx.execute(
            "INSERT INTO trips(title, description, destination, first_day, last_day, zone, trip_time, created_at, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![trip.title.trim(), trip.description, trip.destination.trim(), day_text(trip.first_day), day_text(trip.last_day), zone.code(), trip.trip_time, now],
        )
        .map_err(sql_err)?;
        let trip_id = tx.last_insert_rowid();
        let mut days = Vec::new();
        for (position, day) in package.days.iter().enumerate() {
            tx.execute(
                "INSERT INTO trip_days(trip_id, position, day, title, notes) VALUES(?1, ?2, ?3, ?4, ?5)",
                params![trip_id, position as i64, day_text(day.day), day.title, day.notes],
            )
            .map_err(sql_err)?;
            days.push(tx.last_insert_rowid());
        }
        let mut refs = std::collections::HashMap::new();
        let mut in_day = vec![0i64; days.len()];
        for (position, photo) in package.photos.iter().enumerate() {
            let media_id = if let Some(&photo_id) = existing.get(&photo.key) {
                media_ref(&tx, photo_id)?
            } else {
                let (path, bytes, modified) = match files.get(&photo.key) {
                    Some(path) => {
                        let meta = std::fs::metadata(path).map_err(|err| err.to_string())?;
                        let modified = meta
                            .modified()
                            .ok()
                            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                            .map_or(0, |d| d.as_secs() as i64);
                        (path_text(path), meta.len() as i64, modified)
                    }
                    None => (String::new(), 0, 0),
                };
                let name = files
                    .get(&photo.key)
                    .and_then(|path| path.file_name())
                    .map_or(photo.name.clone(), |name| {
                        name.to_string_lossy().into_owned()
                    });
                tx.execute(
                    "INSERT INTO media_refs(photo_id, path, filename, bytes, modified_at, file_taken_at, unlinked_at, created_at)
                     VALUES(NULL, ?1, ?2, ?3, ?4, 0, ?5, ?5)",
                    params![path, name, bytes, modified, now],
                )
                .map_err(sql_err)?;
                tx.last_insert_rowid()
            };
            refs.insert(photo.key, media_id);
            if !photo.member {
                continue;
            }
            let day = photo.day.map(|index| (days[index], index));
            let day_position = day.map_or(0, |(_, index)| {
                in_day[index] += 1;
                in_day[index] - 1
            });
            tx.execute(
                "INSERT OR IGNORE INTO trip_media(trip_id, media_id, position, day_id, day_source, day_position, added_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![trip_id, media_id, position as i64, day.map(|(id, _)| id), if day.is_some() { DaySource::User.code() } else { 0 }, day_position, now],
            )
            .map_err(sql_err)?;
        }
        for crate::trip_package::PackStory { story, blocks } in &package.stories {
            let story_id = insert_story(&tx, trip_id, &story.title)?;
            tx.execute(
                "UPDATE stories SET subtitle = ?2, author = ?3, show_date = ?4, template = ?5, finished = ?6, cover_ref = ?7, archived = ?8,
                    introduction = ?9, conclusion = ?10 WHERE id = ?1",
                params![story_id, story.subtitle.trim(), story.author.trim(), story.show_date, story.template, story.finished, story.cover.and_then(|key| refs.get(&key)), story.archived, story.introduction.trim(), story.conclusion.trim()],
            )
            .map_err(sql_err)?;
            let mut blocks = blocks.clone();
            for block in &mut blocks {
                let media = match &mut block.block {
                    Block::Photo(one) => std::slice::from_mut(one),
                    Block::Gallery(items)
                    | Block::Grid { media: items, .. }
                    | Block::Unknown { media: items, .. } => items.as_mut_slice(),
                    _ => &mut [],
                };
                for item in media {
                    item.media_id = *refs
                        .get(&item.media_id)
                        .ok_or("a block uses a photo the package doesn't have")?;
                }
            }
            write_blocks(&tx, story_id, &blocks)?;
        }
        // Tracks link by path; those just written wait for the scan.
        for track in &package.tracks {
            let Some(path) = &track.path else { continue };
            tx.execute(
                "INSERT INTO trip_tracks(trip_id, day_id, track_id, path, name) VALUES(?1, ?2, (SELECT id FROM tracks WHERE path = ?3), ?3, ?4)
                 ON CONFLICT(trip_id, path) DO NOTHING",
                params![trip_id, track.day.map(|index| days[index]), path_text(path), track.name],
            )
            .map_err(sql_err)?;
        }
        tx.commit().map_err(sql_err)?;
        Ok(trip_id)
    }

    pub fn story(&self, id: i64) -> Option<Story> {
        self.conn.query_row(&format!("SELECT {STORY_COLS} FROM stories WHERE id = ?1"), [id], read_story).optional().ok().flatten()
    }

    /// The Trip's Stories, the most recently changed first.
    pub fn stories(&self, trip_id: i64) -> Vec<Story> {
        self.map_rows(&format!("SELECT {STORY_COLS} FROM stories WHERE trip_id = ?1 ORDER BY updated_at DESC, id DESC"), [trip_id], read_story)
    }

    /// The Story's blocks in order.
    pub fn story_blocks(&self, story_id: i64) -> Vec<StoryBlock> {
        let rows: Vec<(i64, String, String, i64, String)> = self.map_rows(
            "SELECT id, uid, kind, version, payload FROM story_blocks WHERE story_id = ?1 ORDER BY position",
            [story_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        );
        let Ok(mut media) = self.conn.prepare("SELECT media_id, caption, alt FROM story_media WHERE block_id = ?1 ORDER BY position") else {
            return Vec::new();
        };
        rows.into_iter()
            .map(|(id, uid, kind, version, payload)| {
                let uses: Vec<MediaUse> = media
                    .query_map([id], |row| Ok(MediaUse { media_id: row.get(0)?, caption: row.get(1)?, alt: row.get(2)? }))
                    .map(|rows| rows.filter_map(Result::ok).collect())
                    .unwrap_or_default();
                StoryBlock { uid, block: Block::read(&kind, version, &payload, uses) }
            })
            .collect()
    }

    /// Saves the Story's title, subtitle, author, date display, template,
    /// introduction and conclusion, and whether it is finished.
    /// Fails without a change if the Story was saved since `revision`.
    /// Returns its new revision.
    pub fn save_story_details(&self, story: &Story, revision: i64) -> Result<i64, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let next = next_revision(&tx, story.id, revision)?;
        tx.execute(
            "UPDATE stories SET title = ?2, subtitle = ?3, author = ?4, show_date = ?5, template = ?6, finished = ?7, introduction = ?8, conclusion = ?9 WHERE id = ?1",
            params![
                story.id,
                story.title.trim(),
                story.subtitle.trim(),
                story.author.trim(),
                story.show_date,
                story.template,
                story.finished,
                story.introduction.trim(),
                story.conclusion.trim()
            ],
        )
        .map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        Ok(next)
    }

    /// Makes `media_id` the Story's cover, or clears it. The cover is the
    /// Story's own: stars and other Stories are left as they are.
    pub fn set_story_cover(&self, story_id: i64, media_id: Option<i64>, revision: i64) -> Result<i64, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let next = next_revision(&tx, story_id, revision)?;
        tx.execute("UPDATE stories SET cover_ref = ?2 WHERE id = ?1", params![story_id, media_id]).map_err(sql_err)?;
        tx.execute(UNUSED_REFS, []).map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        Ok(next)
    }

    /// Replaces the Story's blocks with `blocks`. Fails without a change if
    /// the Story was saved since `revision`. Returns its new revision.
    pub fn save_story_blocks(&self, story_id: i64, revision: i64, blocks: &[StoryBlock]) -> Result<i64, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        let next = next_revision(&tx, story_id, revision)?;
        tx.execute("DELETE FROM story_blocks WHERE story_id = ?1", [story_id]).map_err(sql_err)?;
        write_blocks(&tx, story_id, blocks)?;
        tx.execute(UNUSED_REFS, []).map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        Ok(next)
    }

    /// Archives a Story, or brings it back.
    pub fn set_story_archived(&self, id: i64, archived: bool) {
        let _ = self.conn.execute("UPDATE stories SET archived = ?2, updated_at = ?3 WHERE id = ?1", params![id, archived, unix_now()]);
    }

    /// Deletes a Story and its blocks. Its photos stay in the Trip.
    pub fn delete_story(&self, id: i64) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_err)?;
        tx.execute("DELETE FROM stories WHERE id = ?1", [id]).map_err(sql_err)?;
        tx.execute(UNUSED_REFS, []).map_err(sql_err)?;
        tx.commit().map_err(sql_err)
    }
}

fn insert_story(tx: &Transaction, trip_id: i64, title: &str) -> Result<i64, String> {
    let now = unix_now();
    tx.execute(
        "INSERT INTO stories(trip_id, title, template, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?4)",
        params![trip_id, title.trim(), DEFAULT_TEMPLATE, now],
    )
    .map_err(sql_err)?;
    let id = tx.last_insert_rowid();
    touch_trip(tx, trip_id)?;
    Ok(id)
}

/// Checks that the Story is still at `revision`, and moves it to the next.
fn next_revision(tx: &Transaction, story_id: i64, revision: i64) -> Result<i64, String> {
    let current: Option<i64> =
        tx.query_row("SELECT revision FROM stories WHERE id = ?1", [story_id], |row| row.get(0)).optional().map_err(sql_err)?;
    match current {
        None => Err(format!("story {story_id} is gone")),
        Some(current) if current != revision => Err(tr("This story was changed elsewhere. Open it again to see the latest version.").into()),
        Some(current) => {
            tx.execute("UPDATE stories SET revision = ?2, updated_at = ?3 WHERE id = ?1", params![story_id, current + 1, unix_now()])
                .map_err(sql_err)?;
            Ok(current + 1)
        }
    }
}

fn write_blocks(tx: &Transaction, story_id: i64, blocks: &[StoryBlock]) -> Result<(), String> {
    let mut uids = HashSet::new();
    let mut block = tx
        .prepare("INSERT INTO story_blocks(story_id, uid, position, kind, version, payload) VALUES(?1, ?2, ?3, ?4, ?5, ?6)")
        .map_err(sql_err)?;
    let mut media = tx
        .prepare("INSERT INTO story_media(block_id, position, media_id, caption, alt) VALUES(?1, ?2, ?3, ?4, ?5)")
        .map_err(sql_err)?;
    for (position, item) in blocks.iter().enumerate() {
        if !uids.insert(item.uid.as_str()) {
            return Err(format!("two blocks share the id {}", item.uid));
        }
        block
            .execute(params![story_id, item.uid, position as i64, item.block.kind(), item.block.version(), item.block.payload()])
            .map_err(sql_err)?;
        let block_id = tx.last_insert_rowid();
        for (at, used) in item.block.media().iter().enumerate() {
            media.execute(params![block_id, at as i64, used.media_id, used.caption, used.alt]).map_err(sql_err)?;
        }
    }
    Ok(())
}

/// `text` with the wildcards of `LIKE` escaped by a backslash.
fn like_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::tests::{folder_path, library, record, temp_dir};
    use super::super::SCHEMA_VERSION;
    use super::*;
    use crate::track::{Point, Track};

    #[test]
    fn suggested_places_keep_the_ones_the_user_named() {
        let (db, _, _, ids) = photos("places", &["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let trip_id = trip(&db, "Coast");
        db.add_to_trip(trip_id, &ids).unwrap();
        db.set_pins(&ids[2..], 38.0, 140.0);
        assert_eq!(db.suggest_places(trip_id, 300.0, 3 * 3600).unwrap(), 2);
        let places = db.trip_places(trip_id);
        assert_eq!(places.iter().map(|place| place.members).collect::<Vec<_>>(), vec![2, 2]);
        let near = places.iter().find(|place| place.lat == 38.0).unwrap().id;
        let filtered = MemberQuery { place: Some(near), ..MemberQuery::default() };
        assert_eq!(db.count_trip_members(trip_id, &filtered), 2);

        db.update_trip_place(near, "Matsushima", "松島", "Boat at noon").unwrap();
        let order: Vec<i64> = db.trip_places(trip_id).iter().rev().map(|place| place.id).collect();
        db.reorder_trip_places(trip_id, &order).unwrap();
        // Suggesting again, even with a narrower distance, leaves the named one as it was.
        db.suggest_places(trip_id, 50.0, 3600).unwrap();
        let again = db.trip_places(trip_id);
        let named = again.iter().find(|place| place.id == near).unwrap();
        assert_eq!((named.name.as_str(), named.local_name.as_str(), named.notes.as_str(), named.members, named.kept), ("Matsushima", "松島", "Boat at noon", 2, true));
        assert_eq!(again.len(), 2);
        let photo = db.photo(ids[2]).unwrap();
        assert_eq!((photo.pin_lat, photo.pin_lon), (Some(38.0), Some(140.0)), "positions are never changed");

        db.delete_trip_place(near).unwrap();
        assert_eq!(db.trip_places(trip_id).len(), 1);
        assert_eq!(db.count_trip_members(trip_id, &MemberQuery::default()), 4, "its photos stay in the trip");
        assert!(db.update_trip_place(near, "Gone", "", "").is_err());
        assert!(db.reorder_trip_places(trip_id, &[near]).is_err());
    }

    #[test]
    fn library_photos_say_whether_a_trip_has_them() {
        let (db, _, folder, ids) = photos("shots", &["a.jpg", "b.jpg"]);
        let trip = db.create_trip(&TripDraft::new("Kyoto", "Asia/Tokyo")).unwrap();
        db.add_to_trip(trip, &ids[..1]).unwrap();
        let mut shots = db.trip_shots();
        shots.sort_by_key(|shot| shot.id);
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(shots.iter().map(|shot| (shot.id, shot.in_trip, shot.folder.clone())).collect::<Vec<_>>(), vec![(ids[0], true, name.clone()), (ids[1], false, name)]);
    }

    /// A library with photos `a.jpg`, `b.jpg`, … in its folder, by name.
    fn photos(tag: &str, names: &[&str]) -> (Database, i64, std::path::PathBuf, Vec<i64>) {
        let (db, folder_id, folder) = library(tag);
        let records: Vec<_> = names.iter().enumerate().map(|(at, name)| record(folder_id, &folder, name, at as i64)).collect();
        let ids = db.write_batch(&records).unwrap().ids;
        (db, folder_id, folder, ids)
    }

    fn trip(db: &Database, title: &str) -> i64 {
        db.create_trip(&TripDraft::new(title, "Asia/Taipei")).unwrap()
    }

    fn all(db: &Database, trip_id: i64) -> Vec<Member> {
        db.trip_members(trip_id, &MemberQuery::default(), 0, 1000)
    }

    fn in_day(db: &Database, trip_id: i64, day_id: i64) -> Vec<i64> {
        let query = MemberQuery { day: DayFilter::Day(day_id), ..MemberQuery::default() };
        db.trip_members(trip_id, &query, 0, 1000).into_iter().map(|member| member.media_id).collect()
    }

    fn day(text: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
    }

    #[test]
    fn a_trip_from_two_folders_keeps_everything_through_a_reopen() {
        let (db, folder_id, folder, ids) = photos("trip-reopen", &["a.jpg", "b.jpg"]);
        let other = folder.parent().unwrap().join("more");
        std::fs::create_dir_all(&other).unwrap();
        let other_id = db.add_folder(&other).unwrap();
        let more = db.write_batch(&[record(other_id, &other.canonicalize().unwrap(), "c.jpg", 5)]).unwrap().ids[0];
        db.set_star(ids[0], true);
        let album = db.create_album("Favorites").unwrap();
        db.add_to_album(album, &[ids[1]]);
        let before: Vec<String> = [ids[0], ids[1], more].iter().map(|id| format!("{:?}", db.photo(*id).unwrap())).collect();

        let mut draft = TripDraft::new("  Taiwan  ", "Asia/Taipei");
        draft.first_day = day("2026-03-01");
        draft.last_day = day("2026-03-07");
        draft.destination = "Taipei".into();
        let trip_id = db.create_trip(&draft).unwrap();
        let refs = db.add_to_trip(trip_id, &[more, ids[1], ids[0]]).unwrap();
        let first = db.add_trip_day(trip_id, day("2026-03-01"), "Arrival").unwrap();
        db.move_to_day(trip_id, Some(first), &[refs[2], refs[0]], DaySource::User).unwrap();
        db.set_trip_cover(trip_id, Some(refs[1])).unwrap();
        let story = db.create_story(trip_id, "Seven days").unwrap();
        db.save_story_blocks(story, 1, &[StoryBlock::new(Block::Photo(MediaUse::new(refs[0])))]).unwrap();

        let path = db.conn.path().unwrap().to_string();
        drop(db);
        let db = Database::open(Path::new(&path)).unwrap();
        let trip = db.trip(trip_id).unwrap();
        assert_eq!((trip.title.as_str(), trip.zone.as_str(), trip.first_day, trip.last_day), ("Taiwan", "Asia/Taipei", day("2026-03-01"), day("2026-03-07")));
        assert_eq!((trip.members, trip.available, trip.days, trip.stories, trip.cover), (3, 3, 1, 1, Some(refs[1])));
        let order: Vec<Option<i64>> = all(&db, trip_id).iter().map(|member| member.photo_id).collect();
        assert_eq!(order, [Some(more), Some(ids[1]), Some(ids[0])], "in the order they were added");
        assert_eq!(in_day(&db, trip_id, first), [refs[2], refs[0]], "in the order they were put in the day");
        assert_eq!(db.story_blocks(story).len(), 1);
        let after: Vec<String> = [ids[0], ids[1], more].iter().map(|id| format!("{:?}", db.photo(*id).unwrap())).collect();
        assert_eq!(before, after, "stars, captions, dates, and paths are untouched");
        assert_eq!(db.albums()[0].count, 1, "albums are untouched");
        let _ = folder_id;
    }

    #[test]
    fn adding_twice_keeps_one_member_and_reports_what_it_would_do() {
        let (db, _, _, ids) = photos("trip-add", &["a.jpg", "b.jpg", "clip.mp4"]);
        let trip_id = trip(&db, "Coast");
        db.add_to_trip(trip_id, &[ids[0]]).unwrap();
        let preview = db.preview_add_to_trip(trip_id, &[ids[0], ids[1], ids[2], ids[1], 9999]);
        assert_eq!((preview.photos, preview.videos, preview.already), (1, 1, 1));
        assert_eq!(preview.taken, Some((1_700_000_000, 1_700_000_000)));
        // The fixtures share size, pixel size and date, so the second looks like a copy.
        assert_eq!((preview.no_time, preview.located, preview.in_other_trip, preview.duplicates), (0, 2, 0, 1));
        let other = trip(&db, "Hills");
        assert_eq!(db.preview_add_to_trip(other, &[ids[0]]).in_other_trip, 1);
        let refs = db.add_to_trip(trip_id, &[ids[1], ids[0], ids[2]]).unwrap();
        assert_eq!(refs.len(), 3);
        assert_eq!(all(&db, trip_id).len(), 3);
        assert!(all(&db, trip_id)[2].is_video);
        assert!(db.add_to_trip(trip_id, &[9999]).is_err(), "a photo that isn't in the library");
        assert_eq!(db.remove_photos_from_trip(trip_id, &[ids[2], 9999]).unwrap(), 1, "by catalog photo");
        db.add_to_trip(trip_id, &[ids[2]]).unwrap();
        assert!(db.add_to_trip(other + 1, &[ids[0]]).is_err(), "a trip that doesn't exist");
        assert_eq!(all(&db, trip_id).len(), 3, "a failed add changes nothing");
    }

    #[test]
    fn a_photo_can_be_in_several_trips_but_only_in_days_of_each() {
        let (db, _, _, ids) = photos("trip-days", &["a.jpg"]);
        let (north, south) = (trip(&db, "North"), trip(&db, "South"));
        let north_ref = db.add_to_trip(north, &ids).unwrap()[0];
        let south_ref = db.add_to_trip(south, &ids).unwrap()[0];
        assert_eq!(north_ref, south_ref, "one reference per photo");
        let (north_day, south_day) = (db.add_trip_day(north, None, "").unwrap(), db.add_trip_day(south, None, "").unwrap());
        db.move_to_day(north, Some(north_day), &[north_ref], DaySource::User).unwrap();
        db.move_to_day(south, Some(south_day), &[south_ref], DaySource::User).unwrap();
        assert!(db.move_to_day(north, Some(south_day), &[north_ref], DaySource::User).is_err());
        assert_eq!(in_day(&db, north, north_day), [north_ref], "a rejected move changes nothing");

        // The library refuses it too, whoever writes.
        let raw = db.conn.execute("UPDATE trip_media SET day_id = ?1 WHERE trip_id = ?2", params![south_day, north]);
        assert!(raw.is_err());
        assert!(db.conn.execute("UPDATE trip_days SET trip_id = ?1 WHERE id = ?2", params![south, north_day]).is_err());
        let track = db.conn.execute(
            "INSERT INTO trip_tracks(trip_id, day_id, path, name) VALUES(?1, ?2, '/t.gpx', 't')",
            params![north, south_day],
        );
        assert!(track.is_err());
    }

    #[test]
    fn suggestions_never_replace_the_users_days_and_moves_can_be_undone() {
        let (db, _, _, ids) = photos("trip-suggest", &["a.jpg", "b.jpg", "c.jpg"]);
        let trip_id = trip(&db, "Alps");
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        let (one, two) = (db.add_trip_day(trip_id, day("2026-07-01"), "").unwrap(), db.add_trip_day(trip_id, day("2026-07-02"), "").unwrap());
        db.move_to_day(trip_id, Some(one), &[refs[0]], DaySource::User).unwrap();
        let moved = db.move_to_day(trip_id, Some(two), &refs, DaySource::Suggested).unwrap();
        assert_eq!(moved.len(), 2, "the user's choice stays");
        assert_eq!(in_day(&db, trip_id, one), [refs[0]]);
        assert_eq!(in_day(&db, trip_id, two), [refs[1], refs[2]]);
        let sources: Vec<_> = all(&db, trip_id).iter().map(|member| member.day_source).collect();
        assert_eq!(sources, [Some(DaySource::User), Some(DaySource::Suggested), Some(DaySource::Suggested)]);

        let undo = db.move_to_day(trip_id, Some(one), &[refs[2], refs[1]], DaySource::User).unwrap();
        assert_eq!(in_day(&db, trip_id, one), [refs[0], refs[2], refs[1]], "after the day's members, in selection order");
        db.restore_day_assignments(trip_id, &undo).unwrap();
        assert_eq!(in_day(&db, trip_id, two), [refs[1], refs[2]]);
        assert_eq!(all(&db, trip_id)[1].day_source, Some(DaySource::Suggested), "undo puts back who chose the day");

        db.order_day(two, &[refs[2]]).unwrap();
        assert_eq!(in_day(&db, trip_id, two), [refs[2], refs[1]]);
        let in_two = MemberQuery { day: DayFilter::Day(two), ..MemberQuery::default() };
        let shown: Vec<i64> = db.trip_photos(trip_id, &in_two, Sort::Name).iter().map(|photo| photo.id).collect();
        assert_eq!(shown, [ids[2], ids[1]], "a Day shows its own order, whatever the sort");
        assert!(db.order_day(two, &[refs[0]]).is_err(), "only members of the day");
        db.move_to_day(trip_id, None, &[refs[1]], DaySource::User).unwrap();
        let unassigned = MemberQuery { day: DayFilter::Unassigned, ..MemberQuery::default() };
        assert_eq!(db.count_trip_members(trip_id, &unassigned), 1);
        assert_eq!(all(&db, trip_id)[1].day_source, None);
    }

    #[test]
    fn days_can_be_empty_share_a_date_be_reordered_and_deleted() {
        let (db, _, _, ids) = photos("trip-day-order", &["a.jpg"]);
        let trip_id = trip(&db, "Loop");
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        let a = db.add_trip_day(trip_id, day("2026-05-01"), "Morning").unwrap();
        let b = db.add_trip_day(trip_id, day("2026-05-01"), "Evening").unwrap();
        let c = db.add_trip_day(trip_id, None, "Later").unwrap();
        db.update_trip_day(c, day("2026-05-03"), " Last ", "notes").unwrap();
        db.reorder_trip_days(trip_id, &[c, a, b]).unwrap();
        let days = db.trip_days(trip_id);
        let order: Vec<_> = days.iter().map(|day| (day.id, day.title.as_str())).collect();
        assert_eq!(order, [(c, "Last"), (a, "Morning"), (b, "Evening")]);
        assert_eq!(days[0].notes, "notes");
        assert!(db.reorder_trip_days(trip_id, &[c, a]).is_err(), "every day once");
        assert!(db.reorder_trip_days(trip_id, &[c, a, a]).is_err());

        db.move_to_day(trip_id, Some(a), &refs, DaySource::User).unwrap();
        db.delete_trip_day(a).unwrap();
        let days = db.trip_days(trip_id);
        assert_eq!(days.iter().map(|day| (day.id, day.position)).collect::<Vec<_>>(), [(c, 0), (b, 1)]);
        let member = &all(&db, trip_id)[0];
        assert_eq!((member.day_id, member.day_source), (None, None), "the member stays, without a day");
    }

    #[test]
    fn member_lists_and_counts_agree_for_every_filter() {
        let (db, _, folder, ids) = photos("trip-query", &["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let trip_id = trip(&db, "Filters");
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        let day_id = db.add_trip_day(trip_id, None, "").unwrap();
        db.move_to_day(trip_id, Some(day_id), &refs[..2], DaySource::User).unwrap();
        db.set_star(ids[1], true);
        db.write_batch(&[{
            let mut bare = record(1, &folder, "c.jpg", 2);
            bare.photo.gps_lat = None;
            bare.photo.gps_lon = None;
            bare.existing = Some(ids[2]);
            bare
        }])
        .unwrap();
        db.delete_photos(&[ids[3]]);
        let first = db.create_story(trip_id, "First").unwrap();
        let photo = |media: i64| StoryBlock::new(Block::Photo(MediaUse::new(media)));
        db.save_story_blocks(first, 1, &[photo(refs[0]), StoryBlock::new(Block::Gallery(vec![MediaUse::new(refs[3])]))]).unwrap();
        let second = db.create_story(trip_id, "Second").unwrap();
        db.save_story_blocks(second, 1, &[photo(refs[2])]).unwrap();
        let other_trip = trip(&db, "Other");
        let elsewhere = db.create_story(other_trip, "").unwrap();
        db.add_to_trip(other_trip, &ids[1..2]).unwrap();
        db.save_story_blocks(elsewhere, 1, &[photo(refs[1])]).unwrap();
        let cases = [
            (MemberQuery::default(), vec![refs[0], refs[1], refs[2], refs[3]]),
            (MemberQuery { day: DayFilter::Day(day_id), ..MemberQuery::default() }, vec![refs[0], refs[1]]),
            (MemberQuery { day: DayFilter::Unassigned, ..MemberQuery::default() }, vec![refs[2], refs[3]]),
            (MemberQuery { starred: true, ..MemberQuery::default() }, vec![refs[1]]),
            (MemberQuery { no_location: true, ..MemberQuery::default() }, vec![refs[2]]),
            (MemberQuery { unavailable: true, ..MemberQuery::default() }, vec![refs[3]]),
            (MemberQuery { day: DayFilter::Day(day_id), starred: true, ..MemberQuery::default() }, vec![refs[1]]),
            (MemberQuery { story: StoryFilter::AnyStory, ..MemberQuery::default() }, vec![refs[0], refs[2], refs[3]]),
            (MemberQuery { story: StoryFilter::Story(first), ..MemberQuery::default() }, vec![refs[0], refs[3]]),
            (MemberQuery { story: StoryFilter::Story(elsewhere), ..MemberQuery::default() }, vec![]),
        ];
        for (query, expected) in cases {
            let listed: Vec<i64> = db.trip_members(trip_id, &query, 0, 100).into_iter().map(|member| member.media_id).collect();
            assert_eq!(listed, expected, "{query:?}");
            assert_eq!(db.count_trip_members(trip_id, &query), expected.len() as i64, "{query:?}");
        }
        let starred = MemberQuery { starred: true, ..MemberQuery::default() };
        let grid: Vec<i64> = db.trip_photos(trip_id, &starred, Sort::Name).iter().map(|photo| photo.id).collect();
        assert_eq!(grid, [ids[1]], "the grid uses the same query");
        assert_eq!(db.trip_photos(trip_id, &MemberQuery::default(), Sort::Name).len(), 3, "unavailable members have no photo");
        assert_eq!(db.photo_at_path(Path::new(&folder_path(&folder, "a.jpg"))), Some(ids[0]));
        assert_eq!(db.photo_at_path(Path::new(&folder_path(&folder, "d.jpg"))), None);
        let page: Vec<i64> = db.trip_members(trip_id, &MemberQuery::default(), 1, 2).into_iter().map(|member| member.media_id).collect();
        assert_eq!(page, [refs[1], refs[2]]);
        let trip = db.trip(trip_id).unwrap();
        assert_eq!((trip.members, trip.available), (4, 3));
    }

    #[test]
    fn a_removed_photo_stays_in_its_trip_and_story_and_comes_back() {
        let (db, folder_id, folder, ids) = photos("trip-relink", &["a.jpg", "b.jpg"]);
        let trip_id = trip(&db, "Return");
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        let story = db.create_story(trip_id, "").unwrap();
        db.save_story_blocks(story, 1, &[StoryBlock::new(Block::Gallery(vec![MediaUse::new(refs[0]), MediaUse::new(refs[1])]))]).unwrap();

        // The file goes, and the archive of removed photos is emptied.
        db.remove_missing(folder_id, &[folder_path(&folder, "b.jpg")]);
        db.purge_archive(i64::MAX);
        let gone = &all(&db, trip_id)[0];
        assert_eq!((gone.photo_id, gone.path.as_str(), gone.filename.as_str()), (None, folder_path(&folder, "a.jpg").as_str(), "a.jpg"));
        assert_eq!(db.story_blocks(story)[0].block.media().len(), 2, "the story keeps it");

        // Another photo arrives; it gets a new id and nothing of the old one.
        let other = db.write_batch(&[record(folder_id, &folder, "z.jpg", 99)]).unwrap().ids[0];
        assert!(other > ids[1]);
        assert_eq!(db.relink_trip_media(), 0);
        assert_eq!(all(&db, trip_id)[0].photo_id, None);
        assert_eq!(db.count_trip_members(trip_id, &MemberQuery::default()), 2);

        // The file comes back at its path.
        let back = db.write_batch(&[record(folder_id, &folder, "a.jpg", 0)]).unwrap().ids[0];
        assert_eq!(db.relink_trip_media(), 1);
        assert_eq!(all(&db, trip_id)[0].photo_id, Some(back));
        assert_eq!(db.trip(trip_id).unwrap().available, 2);
    }

    #[test]
    fn a_moved_photo_is_found_only_when_one_photo_matches() {
        let (db, folder_id, folder, ids) = photos("trip-moved", &["a.jpg", "b.jpg"]);
        let trip_id = trip(&db, "Moved");
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        db.rename_photos(&[(ids[0], folder_path(&folder, "renamed.jpg"), "renamed.jpg".into())]).unwrap();
        assert_eq!(all(&db, trip_id)[0].filename, "renamed.jpg", "the hints follow a rename");

        db.delete_photos(&ids);
        // a.jpg turns up under one new name; b.jpg under two.
        let one = db.write_batch(&[record(folder_id, &folder.join("x"), "a.jpg", 0)]).unwrap().ids[0];
        let twins = db.write_batch(&[record(folder_id, &folder.join("x"), "b.jpg", 1), record(folder_id, &folder.join("y"), "b.jpg", 1)]).unwrap().ids;
        assert_eq!(db.relink_trip_media(), 1);
        let members = all(&db, trip_id);
        assert_eq!(members[0].photo_id, Some(one));
        assert_eq!(members[1].photo_id, None, "two matches: the user chooses");

        assert!(db.relink_media_to(refs[1], one).is_err(), "a photo stands for one reference");
        db.relink_media_to(refs[1], twins[1]).unwrap();
        assert_eq!(all(&db, trip_id)[1].photo_id, Some(twins[1]));
        assert!(db.relink_media_to(refs[1], twins[0]).is_err(), "only an unavailable reference");
    }

    #[test]
    fn removing_a_folder_leaves_its_photos_unavailable() {
        let (db, folder_id, _, ids) = photos("trip-folder", &["a.jpg"]);
        let trip_id = trip(&db, "Drive");
        db.add_to_trip(trip_id, &ids).unwrap();
        db.remove_folder(folder_id).unwrap();
        let member = &all(&db, trip_id)[0];
        assert_eq!(member.photo_id, None);
        let unlinked: Option<i64> = db.conn.query_row("SELECT unlinked_at FROM media_refs", [], |row| row.get(0)).unwrap();
        assert!(unlinked.is_some());
    }

    #[test]
    fn stories_keep_their_blocks_and_refuse_stale_saves() {
        let (db, _, _, ids) = photos("trip-story", &["a.jpg", "b.jpg"]);
        let trip_id = trip(&db, "Story");
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        let story_id = db.create_story(trip_id, " Notes ").unwrap();
        let story = db.story(story_id).unwrap();
        assert_eq!((story.title.as_str(), story.revision, story.template.as_str(), story.cover), ("Notes", 1, DEFAULT_TEMPLATE, None));
        assert!(db.story_blocks(story_id).is_empty(), "a blank story has no blocks");

        let photo = MediaUse { media_id: refs[0], caption: "Harbor".into(), alt: "Boats at a pier".into() };
        let unknown = Block::Unknown { kind: "map".into(), version: 7, payload: r#"{"zoom":3}"#.into(), media: vec![MediaUse::new(refs[1])] };
        let blocks: Vec<StoryBlock> = [Block::Heading("Day 1".into()), Block::Photo(photo), Block::Paragraph("Rain.".into()), unknown]
            .into_iter()
            .map(StoryBlock::new)
            .collect();
        let revision = db.save_story_blocks(story_id, 1, &blocks).unwrap();
        assert_eq!(revision, 2);
        assert_eq!(db.story_blocks(story_id), blocks, "ids, order, captions, and unknown blocks come back");

        let stale = db.save_story_blocks(story_id, 1, &blocks[..1]);
        assert!(stale.is_err());
        assert_eq!(db.story_blocks(story_id).len(), 4, "a stale save changes nothing");
        let twice = [blocks[0].clone(), blocks[0].clone()];
        assert!(db.save_story_blocks(story_id, revision, &twice).is_err(), "block ids are unique");
        assert_eq!(db.story(story_id).unwrap().revision, revision);

        let mut details = db.story(story_id).unwrap();
        assert!(!details.finished, "a new story is a draft");
        details.subtitle = "A week by the sea".into();
        details.introduction = "Why we went.".into();
        details.conclusion = "We'll go back.".into();
        details.show_date = false;
        details.finished = true;
        let revision = db.save_story_details(&details, revision).unwrap();
        let revision = db.set_story_cover(story_id, Some(refs[1]), revision).unwrap();
        let saved = db.story(story_id).unwrap();
        assert_eq!((saved.subtitle.as_str(), saved.show_date, saved.cover, saved.revision), ("A week by the sea", false, Some(refs[1]), revision));
        assert_eq!((saved.introduction.as_str(), saved.conclusion.as_str()), ("Why we went.", "We'll go back."));
        assert!(saved.finished);
        assert!(!db.photo(ids[1]).unwrap().starred, "the cover is the story's own");

        db.set_story_archived(story_id, true);
        assert!(db.stories(trip_id)[0].archived);

        // Taking the photos out of the Trip leaves the story whole.
        db.remove_from_trip(trip_id, &refs).unwrap();
        assert_eq!(db.story_blocks(story_id), blocks);
        db.delete_story(story_id).unwrap();
        let left: i64 = db.conn.query_row("SELECT COUNT(*) FROM media_refs", [], |row| row.get(0)).unwrap();
        assert_eq!(left, 0, "references nothing uses are removed");
    }

    #[test]
    fn a_story_from_days_follows_their_order_without_videos() {
        let (db, _, _, ids) = photos("trip-story-days", &["a.jpg", "b.jpg", "clip.mov", "c.jpg"]);
        let trip_id = trip(&db, "Days");
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        let first = db.add_trip_day(trip_id, day("2026-08-01"), "").unwrap();
        let second = db.add_trip_day(trip_id, day("2026-08-02"), "Lake").unwrap();
        let empty = db.add_trip_day(trip_id, None, "").unwrap();
        db.move_to_day(trip_id, Some(first), &[refs[1], refs[0], refs[2]], DaySource::User).unwrap();
        db.move_to_day(trip_id, Some(second), &[refs[3]], DaySource::User).unwrap();

        let (story, videos) = db.create_story_from_days(trip_id, "Summer", &[second, first, empty], |date| date.format("%b %-d").to_string()).unwrap();
        assert_eq!(videos, 1);
        let blocks: Vec<Block> = db.story_blocks(story).into_iter().map(|item| item.block).collect();
        assert_eq!(
            blocks,
            [
                Block::Heading("Lake".into()),
                Block::Photo(MediaUse::new(refs[3])),
                Block::Heading("Aug 1".into()),
                Block::Photo(MediaUse::new(refs[1])),
                Block::Photo(MediaUse::new(refs[0])),
            ]
        );
        db.reorder_trip_days(trip_id, &[first, second, empty]).unwrap();
        assert_eq!(db.story_blocks(story).len(), 5, "changing the days later leaves the story");
        let other = trip(&db, "Other");
        let foreign = db.add_trip_day(other, None, "").unwrap();
        assert!(db.create_story_from_days(trip_id, "", &[foreign], |_| String::new()).is_err());
    }

    #[test]
    fn a_trip_shows_its_cover_or_else_its_first_photo() {
        let (db, _, _, ids) = photos("trip-cover", &["a.jpg", "b.jpg"]);
        let trip_id = trip(&db, "Alps");
        assert_eq!(db.trip_cover_photo(trip_id), None);
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        assert_eq!(db.trip_cover_photo(trip_id), Some(ids[0]));
        db.set_trip_cover(trip_id, Some(refs[1])).unwrap();
        assert_eq!(db.trip_cover_photo(trip_id), Some(ids[1]));
    }

    #[test]
    fn deleting_a_trip_takes_only_what_is_in_it() {
        let (db, folder_id, folder, ids) = photos("trip-delete", &["a.jpg", "b.jpg"]);
        let (kept, gone) = (trip(&db, "Kept"), trip(&db, "Gone"));
        db.add_to_trip(kept, &ids[..1]).unwrap();
        let refs = db.add_to_trip(gone, &ids).unwrap();
        db.add_trip_day(gone, None, "").unwrap();
        let story = db.create_story(gone, "").unwrap();
        db.save_story_blocks(story, 1, &[StoryBlock::new(Block::Photo(MediaUse::new(refs[1])))]).unwrap();
        let ride = folder_path(&folder, "ride.gpx");
        db.store_track(folder_id, &ride, 1, 1, &Track { name: "Ride".into(), segments: Vec::new() }).unwrap();
        let track_id = db.tracks()[0].0;
        db.link_trip_track(gone, track_id, None).unwrap();
        assert_eq!(db.track_choices(), [(track_id, "Ride".to_string(), None)]);

        db.set_trip_archived(gone, true);
        assert_eq!(db.trips(&TripQuery::default()).iter().map(|trip| trip.id).collect::<Vec<_>>(), [kept]);
        let archived = TripQuery { archived: true, ..TripQuery::default() };
        assert_eq!(db.trips(&archived)[0].stories, 1, "archiving keeps the stories");
        assert_eq!(db.trip_impact(gone), TripImpact { days: 1, members: 2, stories: 1, tracks: 1 });

        db.delete_trip(gone).unwrap();
        assert!(db.trip(gone).is_none());
        assert!(db.story(story).is_none());
        assert_eq!(db.trip(kept).unwrap().members, 1);
        assert!(db.photo(ids[1]).is_some(), "photos stay in the library");
        assert_eq!(db.tracks().len(), 1, "tracks stay in the library");
        let left: i64 = db.conn.query_row("SELECT COUNT(*) FROM media_refs", [], |row| row.get(0)).unwrap();
        assert_eq!(left, 1, "only the reference the kept trip uses");
    }

    #[test]
    fn trips_are_found_by_their_words() {
        let (db, _, _, _) = photos("trip-search", &[]);
        let mut draft = TripDraft::new("Spring in Kyoto", "+09:00");
        draft.destination = "Japan".into();
        let kyoto = db.create_trip(&draft).unwrap();
        let mut draft = TripDraft::new("100% Iceland_trip", "Atlantic/Reykjavik");
        draft.description = "Ring road".into();
        let iceland = db.create_trip(&draft).unwrap();
        let find = |search: &str| -> Vec<i64> {
            db.trips(&TripQuery { search: search.into(), ..TripQuery::default() }).iter().map(|trip| trip.id).collect()
        };
        assert_eq!(find("kyoto japan"), [kyoto]);
        assert_eq!(find("road"), [iceland], "the description counts");
        assert_eq!(find("100%"), [iceland]);
        assert!(find("0_I").is_empty(), "wildcards are plain text");
        assert_eq!(find("").len(), 2);
        assert_eq!(db.trip(kyoto).unwrap().zone, "+09:00");
        assert!(db.create_trip(&TripDraft::new("No zone", "")).is_err());
        assert!(db.update_trip(kyoto, &TripDraft::new("", "+09:00")).is_err());
        assert!(db.update_trip(9999, &TripDraft::new("Gone", "+09:00")).is_err());
    }

    #[test]
    fn trip_tracks_survive_their_file_leaving_and_coming_back() {
        let (db, folder_id, folder, _) = photos("trip-tracks", &[]);
        let trip_id = trip(&db, "Hike");
        let day_id = db.add_trip_day(trip_id, None, "").unwrap();
        let path = folder_path(&folder, "hike.gpx");
        let track = Track { name: "Hike".into(), segments: vec![vec![Point { lat: 1.0, lon: 2.0, time: Some(1) }, Point { lat: 1.1, lon: 2.0, time: Some(2) }]] };
        db.store_track(folder_id, &path, 1, 1, &track).unwrap();
        let track_id = db.tracks()[0].0;
        let link = db.link_trip_track(trip_id, track_id, Some(day_id)).unwrap();
        assert_eq!(db.link_trip_track(trip_id, track_id, None).unwrap(), link, "linking again updates the link");

        db.remove_missing_tracks(folder_id, &[]);
        let gone = &db.trip_tracks(trip_id)[0];
        assert_eq!((gone.track_id, gone.path.as_str(), gone.name.as_str()), (None, path.as_str(), "Hike"));
        db.store_track(folder_id, &path, 1, 1, &track).unwrap();
        assert_eq!(db.relink_trip_media(), 1);
        assert_eq!(db.trip_tracks(trip_id)[0].track_id, Some(db.tracks()[0].0));
        db.unlink_trip_track(link);
        assert!(db.trip_tracks(trip_id).is_empty());
    }

    #[test]
    fn an_imported_trip_is_a_new_trip_whose_photos_link_after_a_scan() {
        use crate::trip_package::{PackDay, PackPhoto, PackStory, PackTrip, Package};
        let (db, _, folder, ids) = photos("trip-import", &["a.jpg"]);
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("a.jpg");
        std::fs::write(&file, b"jpeg").unwrap();
        let mut story = db
            .story(db.create_story(trip(&db, "Elsewhere"), "Sea").unwrap())
            .unwrap();
        story.cover = Some(11);
        story.finished = true;
        let grid = Block::Grid {
            columns: 2,
            rows: 1,
            caption: "Two".into(),
            media: vec![MediaUse::new(11), MediaUse::new(12)],
        };
        let package = Package {
            trip: PackTrip {
                title: "Coast".into(),
                zone: "Europe/Lisbon".into(),
                ..PackTrip::default()
            },
            days: vec![
                PackDay {
                    title: "Arrival".into(),
                    ..PackDay::default()
                },
                PackDay::default(),
            ],
            stories: vec![PackStory {
                story,
                blocks: vec![
                    StoryBlock::new(Block::Heading("Day 1".into())),
                    StoryBlock::new(grid),
                ],
            }],
            photos: vec![
                PackPhoto {
                    key: 11,
                    name: "a.jpg".into(),
                    day: Some(0),
                    size: Some(4),
                    member: true,
                    hash: None,
                },
                PackPhoto {
                    key: 12,
                    name: String::new(),
                    day: Some(0),
                    size: None,
                    member: true,
                    hash: None,
                },
            ],
            tracks: vec![crate::trip_package::PackTrack {
                name: "Walk".into(),
                day: Some(1),
                path: Some(folder.join("walk.gpx")),
                ..Default::default()
            }],
        };
        let files = std::collections::HashMap::from([(11, file)]);
        let existing = std::collections::HashMap::<i64, i64>::new();
        let trip_id = db.import_trip(&package, &files, &existing).unwrap();
        let story_id = db.stories(trip_id)[0].id;
        let track = &db.trip_tracks(trip_id)[0];
        assert_eq!(
            (track.name.as_str(), track.day_id, track.track_id),
            ("Walk", Some(db.trip_days(trip_id)[1].id), None),
            "the track waits for the scan, in its day"
        );

        let imported = db.trip(trip_id).unwrap();
        assert_eq!(
            (
                imported.title.as_str(),
                imported.zone.as_str(),
                imported.members,
                imported.available
            ),
            ("Coast", "Europe/Lisbon", 2, 0)
        );
        assert_eq!(
            db.trip_days(trip_id)
                .iter()
                .map(|day| (day.title.as_str(), day.members))
                .collect::<Vec<_>>(),
            [("Arrival", 2), ("", 0)]
        );
        assert_eq!(
            db.relink_trip_media(),
            1,
            "the scan links the photo written at its path"
        );
        let members = db.trip_members(trip_id, &MemberQuery::default(), 0, 10);
        assert_eq!(
            members
                .iter()
                .map(|member| member.photo_id)
                .collect::<Vec<_>>(),
            [Some(ids[0]), None]
        );
        let saved = db.story(story_id).unwrap();
        assert_eq!(
            (saved.title.as_str(), saved.finished, saved.cover),
            ("Sea", true, Some(members[0].media_id))
        );
        let blocks = db.story_blocks(story_id);
        let refs: Vec<i64> = members.iter().map(|member| member.media_id).collect();
        assert!(
            matches!(&blocks[1].block, Block::Grid { caption, media, .. } if caption == "Two" && media.iter().map(|m| m.media_id).collect::<Vec<_>>() == refs)
        );

        let reused = db.import_trip(&package, &files, &std::collections::HashMap::from([(11, ids[0])])).unwrap();
        let members = db.trip_members(reused, &MemberQuery::default(), 0, 10);
        assert_eq!(members[0].photo_id, Some(ids[0]), "a photo already in the library is used at once");
        assert_eq!(db.relink_trip_media(), 0);

        let mut bad = package.clone();
        bad.trip.zone = "Mars/Olympus".into();
        let before = db.trips(&TripQuery::default()).len();
        assert!(db.import_trip(&bad, &files, &existing).is_err());
        assert_eq!(
            db.trips(&TripQuery::default()).len(),
            before,
            "a failed import adds nothing"
        );
    }

    #[test]
    fn overlapping_tracks_each_keep_their_own_day() {
        let (db, folder_id, folder, _) = photos("trip-overlap", &[]);
        let trip_id = trip(&db, "Two devices");
        let first = db.add_trip_day(trip_id, None, "").unwrap();
        let second = db.add_trip_day(trip_id, None, "").unwrap();
        // A watch and a phone recorded the same walk.
        let walk = |lat: f64| Track { name: String::new(), segments: vec![vec![Point { lat, lon: 2.0, time: Some(10) }, Point { lat, lon: 2.1, time: Some(20) }]] };
        for (name, lat) in [("watch.gpx", 1.0), ("phone.gpx", 1.0001)] {
            db.store_track(folder_id, &folder_path(&folder, name), 1, 1, &Track { name: name.into(), ..walk(lat) }).unwrap();
        }
        let ids: Vec<i64> = db.tracks().iter().map(|track| track.0).collect();
        let watch = db.link_trip_track(trip_id, ids[0], Some(first)).unwrap();
        let phone = db.link_trip_track(trip_id, ids[1], Some(first)).unwrap();
        assert_ne!(watch, phone);
        db.link_trip_track(trip_id, ids[1], Some(second)).unwrap();
        let days: Vec<(i64, Option<i64>)> = db.trip_tracks(trip_id).iter().map(|link| (link.id, link.day_id)).collect();
        assert!(days.contains(&(watch, Some(first))) && days.contains(&(phone, Some(second))), "{days:?}");
    }

    #[test]
    fn a_library_from_before_trips_is_upgraded_and_keeps_its_photos() {
        let dir = temp_dir("trip-upgrade");
        let path = dir.join("v1.db");
        {
            let db = Database::open(&path).unwrap();
            let pics = dir.join("pics");
            std::fs::create_dir_all(&pics).unwrap();
            let folder_id = db.add_folder(&pics).unwrap();
            let id = db.write_batch(&[record(folder_id, &pics.canonicalize().unwrap(), "a.jpg", 1)]).unwrap().ids[0];
            db.set_star(id, true);
            // Take the library back to the layout of version 1.
            let tables = ["story_media", "story_blocks", "stories", "trip_tracks", "trip_media", "trip_days", "trips", "media_refs"];
            for table in tables {
                db.conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
            }
            db.conn.execute_batch("DROP TRIGGER photos_media_hints; DROP TRIGGER photos_media_unlinked; PRAGMA user_version = 1").unwrap();
        }
        let db = Database::open(&path).unwrap();
        let version: i64 = db.conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(db.photos(&crate::types::Nav::Starred, "", crate::types::Sort::DateNewest).len() == 1, "the photo and its star stay");
        let trip_id = trip(&db, "After");
        let ids: Vec<i64> = db.photo_paths().into_iter().map(|(id, _)| id).collect();
        db.add_to_trip(trip_id, &ids).unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        assert_eq!(db.trip(trip_id).unwrap().members, 1, "opening again changes nothing");
    }

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, CLOCK_FORMAT).unwrap()
    }

    #[test]
    fn days_are_suggested_from_capture_times_and_keep_the_users_choices() {
        let (db, _, _, ids) = photos("trip-suggest-days", &["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg", "f.jpg"]);
        let trip_id = trip(&db, "Taipei");
        let refs = db.add_to_trip(trip_id, &ids).unwrap();
        let unread = db.captures_to_read(trip_id);
        assert_eq!(unread.len(), 6, "nothing read yet");
        let tokyo = |text| Capture::Clock { clock: at(text), offset: Some(9 * 3600) };
        let captures = [
            tokyo("2026-03-02 00:30:00"), // 1 March in Taipei
            tokyo("2026-03-02 12:00:00"),
            Capture::Clock { clock: at("2026-03-02 09:00:00"), offset: None },
            Capture::Missing,
            tokyo("2026-03-03 12:00:00"),
            tokyo("2026-03-01 12:00:00"),
        ];
        for ((media_id, _, modified_at), capture) in unread.into_iter().zip(captures) {
            db.store_capture(media_id, capture, modified_at);
        }
        assert!(db.captures_to_read(trip_id).is_empty());
        db.store_capture(refs[0], captures[0], -1);
        assert_eq!(db.captures_to_read(trip_id).len(), 1, "a file that changed is read again");
        db.store_capture(refs[0], captures[0], 0);
        let mine = db.add_trip_day(trip_id, None, "Arrival").unwrap();
        db.move_to_day(trip_id, Some(mine), &[refs[5]], DaySource::User).unwrap();

        let first = db.suggest_days(trip_id).unwrap();
        assert_eq!((first.assigned, first.new_days, first.kept), (3, 3, 1));
        assert_eq!(first.review, [(Review::NoOffset, 1), (Review::NoTime, 1)]);
        let days = db.trip_days(trip_id);
        let dates: Vec<_> = days.iter().map(|day| day.day.map(|d| d.to_string())).collect();
        assert_eq!(dates, [None, Some("2026-03-01".into()), Some("2026-03-02".into()), Some("2026-03-03".into())]);
        assert_eq!(in_day(&db, trip_id, mine), [refs[5]], "the user's day stays");
        assert_eq!(in_day(&db, trip_id, days[1].id), [refs[0]]);
        assert_eq!(db.review_counts(trip_id), first.review, "kept to review later");
        let to_review = MemberQuery { day: DayFilter::Review, ..MemberQuery::default() };
        assert_eq!(db.count_trip_members(trip_id, &to_review), 2);
        let undo = db.move_to_day(trip_id, Some(mine), &[refs[3]], DaySource::User).unwrap();
        assert_eq!(db.review_counts(trip_id), [(Review::NoOffset, 1)], "placing a photo settles it");
        db.restore_day_assignments(trip_id, &undo).unwrap();
        assert_eq!(db.review_counts(trip_id), first.review, "and Undo brings its review back");

        // Accepting that the camera was on Trip time settles the clock without an offset.
        let mut draft = TripDraft::new("Taipei", "Asia/Taipei");
        draft.trip_time = true;
        db.update_trip(trip_id, &draft).unwrap();
        let again = db.suggest_days(trip_id).unwrap();
        assert_eq!((again.assigned, again.new_days), (4, 0), "no day twice");
        assert_eq!(again.review, [(Review::NoTime, 1)]);
        assert_eq!(db.review_counts(trip_id), again.review);
        assert_eq!(in_day(&db, trip_id, days[2].id), [refs[1], refs[2]]);

        // A shift made with Adjust date counts once.
        db.shift_taken(&[ids[4]], -86_400);
        db.suggest_days(trip_id).unwrap();
        assert_eq!(in_day(&db, trip_id, days[2].id), [refs[1], refs[2], refs[4]]);
        assert_eq!(db.photo(ids[4]).unwrap().taken_at, 1_700_000_000 - 86_400, "catalog dates stay as the user set them");
    }

    #[test]
    fn days_do_not_depend_on_the_computers_time_zone() {
        // Capture times are stored as clock readings and offsets, and Days
        // are counted in the Trip's zone, so nothing here reads the local zone.
        let zone = TripZone::parse("Europe/Lisbon").unwrap();
        let capture = Capture::Clock { clock: at("2026-10-25 01:30:00"), offset: Some(3600) };
        let day = capture_day(capture, 0, &zone, false);
        assert_eq!(day.map(|day| day.to_string()), Ok("2026-10-25".into()));
        assert_eq!(read_capture_row(1, Some("2026-10-25 01:30:00".into()), Some(3600), None), capture);
        assert_eq!(read_capture_row(2, None, None, Some(5)), Capture::Utc(5));
        assert_eq!(read_capture_row(0, None, None, None), Capture::Unread);
    }

    #[test]
    fn a_backup_holds_the_trips() {
        let dir = temp_dir("trip-backup");
        let db = Database::open(&dir.join("picasso.db")).unwrap();
        let trip_id = trip(&db, "Saved");
        db.create_story(trip_id, "Kept").unwrap();
        let backup = crate::backup::create(&dir).unwrap();
        let copy = Database::open(&backup).unwrap();
        assert_eq!(copy.trip(trip_id).unwrap().stories, 1);
        assert_eq!(copy.stories(trip_id)[0].title, "Kept");
    }
}
