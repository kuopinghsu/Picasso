//! SQLite catalog. The UI and the background indexer each open their own connection.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

use crate::track::Track;
use crate::types::{unix_now, Album, Folder, Nav, NewPhoto, Photo, Sort, Unreadable, EARLIEST_PHOTO};

/// Bumped whenever the scanner starts reading new metadata from files, so
/// rows written by an older build are probed once more on the next scan.
pub const INDEX_VERSION: i64 = 3;

const PHOTO_COLS: &str = "\
p.id, p.folder_id, p.path, p.filename, p.width, p.height, p.bytes, \
p.taken_at, p.modified_at, p.starred, p.caption, p.camera, \
p.gps_lat, p.gps_lon, p.pin_lat, p.pin_lon, p.rotation, p.duration, \
p.lens, p.focal_mm";

/// Days a removed photo's stars, captions, albums and edits are kept, so
/// they come back if the file returns or turns up under another name.
const ARCHIVE_DAYS: i64 = 90;

/// Days before a file that couldn't be read is tried again although it
/// hasn't changed, in case it was only locked or the drive was failing.
const UNREADABLE_RETRY_DAYS: i64 = 7;

/// True for a photo row with anything the user set by hand.
const HAS_USER_DATA: &str = "\
(p.starred != 0 OR p.caption != '' OR p.rotation != 0 OR p.pin_lat IS NOT NULL OR p.date_set != 0 \
OR EXISTS (SELECT 1 FROM album_photos ap WHERE ap.photo_id = p.id) \
OR EXISTS (SELECT 1 FROM edits e WHERE e.photo_id = p.id))";

/// Copies the user data of the matching photos into `photo_archive`. The
/// filter appended to `WHERE` takes `?1`; `?2` is the removal time.
const ARCHIVE_SQL: &str = "\
INSERT OR REPLACE INTO photo_archive(path, bytes, modified_at, taken_at, data, removed_at)
SELECT p.path, p.bytes, p.modified_at, p.file_taken_at,
    json_object(
        'starred', p.starred, 'caption', p.caption, 'rotation', p.rotation,
        'pin_lat', p.pin_lat, 'pin_lon', p.pin_lon,
        'taken_set', CASE WHEN p.date_set != 0 THEN p.taken_at END,
        'albums', json((SELECT json_group_array(ap.album_id) FROM album_photos ap WHERE ap.photo_id = p.id)),
        'steps', (SELECT e.steps FROM edits e WHERE e.photo_id = p.id),
        'applied', (SELECT e.applied FROM edits e WHERE e.photo_id = p.id)
    ),
    ?2
FROM photos p WHERE ";

pub struct Database {
    conn: Connection,
}

/// What the catalog already knows about a file, used to skip unchanged files
/// without opening them.
#[derive(Clone, Copy, Debug)]
pub struct Known {
    pub id: i64,
    pub modified_at: i64,
    pub bytes: u64,
    pub version: i64,
}

impl Known {
    pub fn current(&self, modified_at: i64, bytes: u64) -> bool {
        self.modified_at == modified_at && self.bytes == bytes && self.version >= INDEX_VERSION
    }
}

/// One probed file ready to be written by [`Database::write_batch`].
#[derive(Clone, Debug)]
pub struct ScanRecord {
    pub photo: NewPhoto,
    /// Row id when the file was already in the catalog.
    pub existing: Option<i64>,
    /// Star and caption imported from `.picasa.ini` for a file seen the first time.
    pub star: Option<bool>,
    pub caption: Option<String>,
    /// Clockwise rotation imported from `.picasa.ini` for a new file.
    pub rotation: Option<i32>,
    /// Place imported from the folder's geotag file for a new file.
    pub pin: Option<(f64, f64)>,
    /// False when the file could only be read in part (a video while FFmpeg
    /// is missing), so the next scan probes it again.
    pub complete: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchResult {
    pub inserted: usize,
    pub updated: usize,
    /// Row id of each record, in the order the records were given.
    pub ids: Vec<i64>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let conn = Connection::open(path).map_err(|err| err.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(8))
            .map_err(|err| err.to_string())?;
        conn.execute_batch(
            "\
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA foreign_keys=ON;
            ",
        )
        .map_err(|err| err.to_string())?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// A read-only connection for queries on another thread. The schema is
    /// left to the connection [`Database::open`] made.
    pub fn open_reader(path: &Path) -> Result<Self, String> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags).map_err(|err| err.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(8))
            .map_err(|err| err.to_string())?;
        Ok(Self { conn })
    }

    /// Rows this connection has changed since it opened.
    pub fn changes(&self) -> u64 {
        self.conn.total_changes()
    }

    fn migrate(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "\
                CREATE TABLE IF NOT EXISTS folders (
                    id INTEGER PRIMARY KEY,
                    path TEXT NOT NULL UNIQUE,
                    added_at INTEGER NOT NULL,
                    scan_pending INTEGER NOT NULL DEFAULT 1
                );
                CREATE TABLE IF NOT EXISTS photos (
                    id INTEGER PRIMARY KEY,
                    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                    path TEXT NOT NULL UNIQUE,
                    filename TEXT NOT NULL,
                    width INTEGER NOT NULL DEFAULT 0,
                    height INTEGER NOT NULL DEFAULT 0,
                    bytes INTEGER NOT NULL DEFAULT 0,
                    taken_at INTEGER NOT NULL DEFAULT 0,
                    modified_at INTEGER NOT NULL DEFAULT 0,
                    starred INTEGER NOT NULL DEFAULT 0,
                    caption TEXT NOT NULL DEFAULT '',
                    camera TEXT NOT NULL DEFAULT '',
                    gps_lat REAL,
                    gps_lon REAL,
                    pin_lat REAL,
                    pin_lon REAL,
                    rotation INTEGER NOT NULL DEFAULT 0,
                    indexed_version INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS albums (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS album_photos (
                    album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
                    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
                    PRIMARY KEY (album_id, photo_id)
                );
                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS edits (
                    photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
                    steps TEXT NOT NULL,
                    applied INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS photo_archive (
                    path TEXT PRIMARY KEY,
                    bytes INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL,
                    taken_at INTEGER NOT NULL,
                    data TEXT NOT NULL,
                    removed_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS unreadable (
                    path TEXT PRIMARY KEY,
                    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                    reason TEXT NOT NULL,
                    is_folder INTEGER NOT NULL DEFAULT 0,
                    bytes INTEGER NOT NULL DEFAULT 0,
                    modified_at INTEGER NOT NULL DEFAULT 0,
                    checked_at INTEGER NOT NULL,
                    indexed_version INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_unreadable_folder ON unreadable(folder_id);
                CREATE TABLE IF NOT EXISTS excluded (
                    path TEXT PRIMARY KEY,
                    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_photos_folder ON photos(folder_id);
                -- GPS tracks from GPX and KML files; `points` is Track::encode.
                CREATE TABLE IF NOT EXISTS tracks (
                    id INTEGER PRIMARY KEY,
                    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                    path TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    bytes INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL,
                    started_at INTEGER,
                    ended_at INTEGER,
                    points BLOB NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_tracks_folder ON tracks(folder_id);
                DROP TABLE IF EXISTS faces;
                DROP TABLE IF EXISTS people;
                ",
            )
            .map_err(|err| err.to_string())?;
        for (column, decl) in [
            ("gps_lat", "REAL"),
            ("gps_lon", "REAL"),
            ("pin_lat", "REAL"),
            ("pin_lon", "REAL"),
            ("indexed_version", "INTEGER NOT NULL DEFAULT 0"),
            ("rotation", "INTEGER NOT NULL DEFAULT 0"),
            ("duration", "REAL NOT NULL DEFAULT 0"),
            ("added_at", "INTEGER NOT NULL DEFAULT 0"),
            // Set when the user changed the date taken; scans keep it then.
            ("date_set", "INTEGER NOT NULL DEFAULT 0"),
            ("lens", "TEXT NOT NULL DEFAULT ''"),
            ("focal_mm", "REAL NOT NULL DEFAULT 0"),
        ] {
            self.ensure_column("photos", column, decl)?;
        }
        // The date taken read from the file, kept when the user changes it.
        if !self.columns("photos").iter().any(|name| name == "file_taken_at") {
            self.conn
                .execute_batch(
                    "ALTER TABLE photos ADD COLUMN file_taken_at INTEGER NOT NULL DEFAULT 0;
                     UPDATE photos SET file_taken_at = taken_at;",
                )
                .map_err(|err| err.to_string())?;
        }
        self.ensure_column("folders", "scan_pending", "INTEGER NOT NULL DEFAULT 1")?;
        self.conn
            .execute_batch(
                "\
                CREATE INDEX IF NOT EXISTS idx_photos_bytes ON photos(bytes);
                CREATE INDEX IF NOT EXISTS idx_photos_starred ON photos(starred);
                CREATE INDEX IF NOT EXISTS idx_photos_name ON photos(filename COLLATE NOCASE);
                CREATE INDEX IF NOT EXISTS idx_album_photos_photo ON album_photos(photo_id);
                -- Matches the default newest-first order, so the library
                -- needs no extra sort; it also serves lookups by date.
                CREATE INDEX IF NOT EXISTS idx_photos_taken_name ON photos(taken_at DESC, filename COLLATE NOCASE);
                DROP INDEX IF EXISTS idx_photos_taken;
                -- The highest photo id ever used. New photos are numbered
                -- above it, so an id never comes back after its photo
                -- leaves, and nothing cached under it is shown for another.
                CREATE TABLE IF NOT EXISTS id_floor (name TEXT PRIMARY KEY, value INTEGER NOT NULL);
                INSERT OR IGNORE INTO id_floor(name, value) SELECT 'photos', coalesce(max(id), 0) FROM photos;
                CREATE TRIGGER IF NOT EXISTS photos_id_floor AFTER DELETE ON photos BEGIN
                    UPDATE id_floor SET value = max(value, OLD.id) WHERE name = 'photos';
                END;
                ",
            )
            .map_err(|err| err.to_string())?;
        self.purge_archive(unix_now() - ARCHIVE_DAYS * 86_400);
        // Older builds stored placeholder dates such as year 1. Those files
        // are probed again on the next scan, which reads the real date.
        self.conn
            .execute("UPDATE photos SET indexed_version=0 WHERE file_taken_at < ?1", [EARLIEST_PHOTO])
            .map_err(|err| err.to_string())?;
        // Version 2 missed the EXIF of HEIF files that attach it to an item
        // other than the primary picture; only those are read again.
        self.conn
            .execute(
                "UPDATE photos SET indexed_version = CASE WHEN camera = ''
                     AND (lower(filename) LIKE '%.heic' OR lower(filename) LIKE '%.heif' OR lower(filename) LIKE '%.hif')
                     THEN 0 ELSE ?1 END
                 WHERE indexed_version = 2",
                [INDEX_VERSION],
            )
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    fn columns(&self, table: &str) -> Vec<String> {
        let pragma = format!("PRAGMA table_info({table})");
        self.map_rows(&pragma, [], |row| row.get::<_, String>(1))
    }

    fn ensure_column(&self, table: &str, column: &str, decl: &str) -> Result<(), String> {
        if self.columns(table).iter().any(|name| name == column) {
            return Ok(());
        }
        self.conn
            .execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))
            .map_err(|err| err.to_string())
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) {
        let _ = self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        );
    }

    /// Edit steps of a photo as JSON, and how many of them are applied.
    pub fn edit_history(&self, photo_id: i64) -> Option<(String, usize)> {
        self.conn
            .query_row("SELECT steps, applied FROM edits WHERE photo_id=?1", [photo_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?.max(0) as usize))
            })
            .optional()
            .ok()
            .flatten()
    }

    /// Every photo that has edit steps, with the steps and applied count.
    pub fn edit_histories(&self) -> Vec<(i64, String, usize)> {
        self.map_rows("SELECT photo_id, steps, applied FROM edits", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)?.max(0) as usize))
        })
    }

    /// Stores the edit steps of a photo; no steps removes its row.
    pub fn save_edit_history(&self, photo_id: i64, steps: &str, count: usize, applied: usize) {
        if count == 0 {
            let _ = self.conn.execute("DELETE FROM edits WHERE photo_id=?1", [photo_id]);
            return;
        }
        let _ = self.conn.execute(
            "INSERT INTO edits(photo_id, steps, applied, updated_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(photo_id) DO UPDATE SET steps=excluded.steps, applied=excluded.applied,
             updated_at=excluded.updated_at",
            params![photo_id, steps, applied as i64, unix_now()],
        );
    }

    pub fn folders(&self) -> Vec<Folder> {
        let sql = "\
            SELECT f.id, f.path, COUNT(p.id)
            FROM folders f
            LEFT JOIN photos p ON p.folder_id = f.id
            GROUP BY f.id
            ORDER BY f.path COLLATE NOCASE";
        self.map_rows(sql, [], |row| {
            Ok(Folder {
                id: row.get(0)?,
                path: row.get(1)?,
                count: row.get(2)?,
            })
        })
    }

    /// Number of photos directly inside each directory that holds any, by
    /// directory. Counted here rather than with GROUP BY, which sorts every
    /// path and takes three times as long.
    pub fn photo_dirs(&self) -> Vec<(String, i64)> {
        let mut counts: HashMap<String, i64> = HashMap::new();
        if let Ok(mut stmt) = self.conn.prepare("SELECT path FROM photos") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    let Some(path) = row.get_ref(0).ok().and_then(|value| value.as_str().ok()) else {
                        continue;
                    };
                    let dir = path.rfind(['\\', '/']).map_or("", |at| &path[..at]);
                    match counts.get_mut(dir) {
                        Some(count) => *count += 1,
                        None => {
                            counts.insert(dir.to_string(), 1);
                        }
                    }
                }
            }
        }
        let mut dirs: Vec<(String, i64)> = counts.into_iter().collect();
        dirs.sort_unstable();
        dirs
    }

    pub fn add_folder(&self, path: &Path) -> Result<i64, String> {
        let path = path
            .canonicalize()
            .map_err(|err| crate::i18n::trf("Can't open folder: {}", &[&err]))?;
        if !path.is_dir() {
            return Err(crate::i18n::tr("That path is not a folder").into());
        }
        let text = path_text(&path);
        if let Some(id) = self.folder_id(&text) {
            return Ok(id);
        }
        self.conn
            .execute(
                "INSERT INTO folders(path, added_at, scan_pending) VALUES(?1, ?2, 1)",
                params![text, unix_now()],
            )
            .map_err(|err| err.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn remove_folder(&self, id: i64) {
        let _ = self.conn.execute("DELETE FROM folders WHERE id=?1", [id]);
    }

    /// Stops scanning `dir`, a folder below one of the added folders, and
    /// drops its photos from the library. Their stars, captions, and edits
    /// are archived, so they come back if the folder is included again.
    /// Returns the added folder it belongs to and the path as stored.
    pub fn exclude_dir(&self, dir: &Path) -> Result<(i64, String), String> {
        let dir = path_text(&dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()));
        let dir = dir.trim_end_matches(std::path::MAIN_SEPARATOR).to_string();
        let folder_id = self
            .folders()
            .into_iter()
            .filter(|folder| Path::new(&dir).starts_with(&folder.path) && Path::new(&dir) != Path::new(&folder.path))
            .max_by_key(|folder| folder.path.len())
            .map(|folder| folder.id)
            .ok_or_else(|| crate::i18n::tr("Pick a folder inside one of your photo folders").to_string())?;
        let (start, end) = under_dir(&dir);
        let tx = self.conn.unchecked_transaction().map_err(|err| err.to_string())?;
        tx.execute(
            "INSERT OR REPLACE INTO excluded(path, folder_id) VALUES(?1, ?2)",
            params![dir, folder_id],
        )
        .map_err(|err| err.to_string())?;
        let archive = format!("{ARCHIVE_SQL}p.path >= ?1 AND p.path < ?3 AND p.folder_id = ?4 AND {HAS_USER_DATA}");
        tx.execute(&archive, params![start, unix_now(), end, folder_id]).map_err(|err| err.to_string())?;
        tx.execute(
            "DELETE FROM photos WHERE path >= ?1 AND path < ?2 AND folder_id = ?3",
            params![start, end, folder_id],
        )
        .map_err(|err| err.to_string())?;
        tx.execute(
            "DELETE FROM unreadable WHERE folder_id = ?4 AND (path = ?1 OR (path >= ?2 AND path < ?3))",
            params![dir, start, end, folder_id],
        )
        .map_err(|err| err.to_string())?;
        tx.execute(
            "DELETE FROM tracks WHERE path >= ?1 AND path < ?2 AND folder_id = ?3",
            params![start, end, folder_id],
        )
        .map_err(|err| err.to_string())?;
        tx.commit().map_err(|err| err.to_string())?;
        Ok((folder_id, dir))
    }

    /// Scans `dir` again. Returns the added folder it belongs to, which
    /// needs a scan to bring its photos back.
    pub fn include_dir(&self, dir: &str) -> Option<i64> {
        let folder_id = self
            .conn
            .query_row("SELECT folder_id FROM excluded WHERE path=?1", [dir], |row| row.get(0))
            .optional()
            .ok()
            .flatten()?;
        let _ = self.conn.execute("DELETE FROM excluded WHERE path=?1", [dir]);
        Some(folder_id)
    }

    /// Excluded folders as (path, folder id), by path.
    pub fn excluded_dirs(&self) -> Vec<(String, i64)> {
        self.map_rows("SELECT path, folder_id FROM excluded ORDER BY path COLLATE NOCASE", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
    }

    fn folder_id(&self, path: &str) -> Option<i64> {
        self.conn
            .query_row("SELECT id FROM folders WHERE path=?1", [path], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn folder_path(&self, id: i64) -> Option<String> {
        self.conn
            .query_row("SELECT path FROM folders WHERE id=?1", [id], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    /// Marks a folder as needing a scan. The flag clears only when a scan
    /// reaches the end of the folder, so a scan cut short by quitting is
    /// picked up again on the next launch.
    pub fn set_scan_pending(&self, folder_id: i64, pending: bool) {
        let _ = self.conn.execute(
            "UPDATE folders SET scan_pending=?1 WHERE id=?2",
            params![pending as i64, folder_id],
        );
    }

    pub fn pending_folders(&self) -> Vec<i64> {
        self.map_rows("SELECT id FROM folders WHERE scan_pending != 0 ORDER BY id", [], |row| row.get(0))
    }

    /// Every cataloged file under `folder_path`, keyed by path. Files owned by
    /// an overlapping folder are included so they are recognized as known.
    pub fn folder_snapshot(&self, folder_id: i64, folder_path: &str) -> HashMap<String, Known> {
        let (start, end) = under_dir(folder_path);
        let rows = self.map_rows(
            "SELECT path, id, modified_at, bytes, indexed_version FROM photos
             WHERE folder_id=?1 OR (path >= ?2 AND path < ?3)",
            params![folder_id, start, end],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    Known {
                        id: row.get(1)?,
                        modified_at: row.get(2)?,
                        bytes: row.get::<_, i64>(3)? as u64,
                        version: row.get(4)?,
                    },
                ))
            },
        );
        rows.into_iter().collect()
    }

    /// What the scans could not read, with the id of the folder each was in.
    pub fn unreadable(&self) -> Vec<(i64, Unreadable)> {
        self.map_rows(
            "SELECT folder_id, path, reason, is_folder, bytes, modified_at FROM unreadable ORDER BY path",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    Unreadable {
                        path: row.get(1)?,
                        reason: row.get(2)?,
                        folder: row.get::<_, i64>(3)? != 0,
                        bytes: row.get::<_, i64>(4)? as u64,
                        modified_at: row.get(5)?,
                    },
                ))
            },
        )
    }

    /// Files of a folder that failed lately with this build's decoders, by
    /// path. A scan skips them while their size and date stay the same.
    pub fn unreadable_snapshot(&self, folder_id: i64) -> HashMap<String, Unreadable> {
        let since = unix_now() - UNREADABLE_RETRY_DAYS * 86_400;
        let rows = self.map_rows(
            "SELECT path, reason, bytes, modified_at FROM unreadable
             WHERE folder_id=?1 AND is_folder=0 AND checked_at >= ?2 AND indexed_version=?3",
            params![folder_id, since, INDEX_VERSION],
            |row| {
                Ok(Unreadable {
                    path: row.get(0)?,
                    reason: row.get(1)?,
                    folder: false,
                    bytes: row.get::<_, i64>(2)? as u64,
                    modified_at: row.get(3)?,
                })
            },
        );
        rows.into_iter().map(|item| (item.path.clone(), item)).collect()
    }

    /// Stores what a scan of `folder_id` could not read. A complete scan
    /// replaces the folder's list; a partial one only adds to it. Entries
    /// that came from [`Self::unreadable_snapshot`] keep their check time.
    pub fn store_unreadable(&self, folder_id: i64, items: &[Unreadable], complete: bool, remembered: &HashSet<String>) {
        let checked = self.checked_times(folder_id);
        let write = || -> rusqlite::Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            if complete {
                tx.execute("DELETE FROM unreadable WHERE folder_id=?1", [folder_id])?;
            }
            {
                let mut insert = tx.prepare(
                    "INSERT INTO unreadable(path, folder_id, reason, is_folder, bytes, modified_at, checked_at, indexed_version)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(path) DO UPDATE SET folder_id=excluded.folder_id, reason=excluded.reason,
                        is_folder=excluded.is_folder, bytes=excluded.bytes, modified_at=excluded.modified_at,
                        checked_at=excluded.checked_at, indexed_version=excluded.indexed_version",
                )?;
                let now = unix_now();
                for item in items {
                    let at = if remembered.contains(&item.path) { checked.get(&item.path).copied().unwrap_or(now) } else { now };
                    insert.execute(params![
                        item.path,
                        folder_id,
                        item.reason,
                        item.folder,
                        item.bytes as i64,
                        item.modified_at,
                        at,
                        INDEX_VERSION
                    ])?;
                }
            }
            tx.commit()
        };
        if let Err(err) = write() {
            eprintln!("picasso unreadable: {err}");
        }
    }

    fn checked_times(&self, folder_id: i64) -> HashMap<String, i64> {
        self.map_rows("SELECT path, checked_at FROM unreadable WHERE folder_id=?1", [folder_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .into_iter()
        .collect()
    }

    /// Drops files from the unreadable list, after they went to the trash.
    pub fn forget_unreadable(&self, paths: &[String]) {
        for path in paths {
            let _ = self.conn.execute("DELETE FROM unreadable WHERE path=?1", [path]);
        }
    }

    /// Photos sharing their byte count, pixel size, and the date in the file
    /// with another, in groups that may hold copies of the same file. Some
    /// cameras pad every file to a block size, so the byte count alone
    /// matches many shots.
    pub fn duplicate_candidates(&self) -> Vec<Vec<Photo>> {
        let sql = format!(
            "SELECT {PHOTO_COLS}, p.file_taken_at FROM photos p \
             JOIN (SELECT bytes, width, height, file_taken_at FROM photos WHERE bytes > 0 \
                   GROUP BY bytes, width, height, file_taken_at HAVING COUNT(*) > 1) d \
               ON p.bytes = d.bytes AND p.width = d.width AND p.height = d.height AND p.file_taken_at = d.file_taken_at \
             ORDER BY p.bytes, p.width, p.height, p.file_taken_at, p.path"
        );
        let photos: Vec<(Photo, i64)> = self
            .conn
            .prepare(&sql)
            .and_then(|mut stmt| stmt.query_map([], |row| Ok((read_photo(row)?, row.get(18)?)))?.collect())
            .unwrap_or_default();
        let mut groups: Vec<(i64, Vec<Photo>)> = Vec::new();
        for (photo, file_taken_at) in photos {
            match groups.last_mut() {
                Some((taken, group)) if *taken == file_taken_at && (group[0].bytes, group[0].width, group[0].height) == (photo.bytes, photo.width, photo.height) => {
                    group.push(photo)
                }
                _ => groups.push((file_taken_at, vec![photo])),
            }
        }
        groups.into_iter().map(|(_, group)| group).collect()
    }

    /// Moves the date taken of `ids` by `seconds`. Scans keep the new date.
    pub fn shift_taken(&self, ids: &[i64], seconds: i64) {
        let Ok(tx) = self.conn.unchecked_transaction() else { return };
        for id in ids {
            let _ = tx.execute("UPDATE photos SET taken_at = taken_at + ?2, date_set = 1 WHERE id = ?1", params![id, seconds]);
        }
        let _ = tx.commit();
    }

    /// Gives `ids` the date taken in their files back.
    pub fn reset_taken(&self, ids: &[i64]) {
        let Ok(tx) = self.conn.unchecked_transaction() else { return };
        for id in ids {
            let _ = tx.execute("UPDATE photos SET taken_at = file_taken_at, date_set = 0 WHERE id = ?1", [id]);
        }
        let _ = tx.commit();
    }

    pub fn photo(&self, id: i64) -> Option<Photo> {
        let sql = format!("SELECT {PHOTO_COLS} FROM photos p WHERE p.id=?1");
        self.conn
            .query_row(&sql, [id], read_photo)
            .optional()
            .ok()
            .flatten()
    }

    /// Writes probed files in one transaction. Existing rows get fresh file
    /// metadata while stars, captions, albums, and pins stay untouched.
    pub fn write_batch(&self, records: &[ScanRecord]) -> Result<BatchResult, String> {
        let tx = self.conn.unchecked_transaction().map_err(|err| err.to_string())?;
        let mut result = BatchResult::default();
        {
            let mut update = tx
                .prepare_cached(
                    "UPDATE photos SET folder_id=?1, filename=?2, width=?3, height=?4, bytes=?5,
                     taken_at=CASE WHEN date_set != 0 THEN taken_at ELSE ?6 END, file_taken_at=?6,
                     modified_at=?7, camera=?8, gps_lat=?9, gps_lon=?10,
                     indexed_version=?11, duration=?13, lens=?14, focal_mm=?15
                     WHERE id=?12",
                )
                .map_err(|err| err.to_string())?;
            let mut insert = tx
                .prepare_cached(
                    "INSERT INTO photos(
                        id, folder_id, path, filename, width, height, bytes, taken_at, modified_at,
                        camera, gps_lat, gps_lon, indexed_version, starred, caption, rotation, duration,
                        added_at, file_taken_at, lens, focal_mm, pin_lat, pin_lon
                     ) VALUES (
                        max(coalesce((SELECT max(id) FROM photos), 0),
                            coalesce((SELECT value FROM id_floor WHERE name = 'photos'), 0)) + 1,
                        ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?7,?18,?19,?20,?21)
                     ON CONFLICT(path) DO UPDATE SET
                        width=excluded.width, height=excluded.height, bytes=excluded.bytes,
                        taken_at=CASE WHEN date_set != 0 THEN taken_at ELSE excluded.taken_at END,
                        file_taken_at=excluded.file_taken_at, modified_at=excluded.modified_at,
                        camera=excluded.camera, gps_lat=excluded.gps_lat, gps_lon=excluded.gps_lon,
                        indexed_version=excluded.indexed_version, duration=excluded.duration,
                        lens=excluded.lens, focal_mm=excluded.focal_mm
                     RETURNING id",
                )
                .map_err(|err| err.to_string())?;
            let now = unix_now();
            for record in records {
                let photo = &record.photo;
                let version = if record.complete { INDEX_VERSION } else { 0 };
                match record.existing {
                    Some(id) => {
                        update
                            .execute(params![
                                photo.folder_id,
                                photo.filename,
                                photo.width,
                                photo.height,
                                photo.bytes as i64,
                                photo.taken_at,
                                photo.modified_at,
                                photo.camera,
                                photo.gps_lat,
                                photo.gps_lon,
                                version,
                                id,
                                photo.duration,
                                photo.lens,
                                f64::from(photo.focal_mm),
                            ])
                            .map_err(|err| err.to_string())?;
                        result.updated += 1;
                        result.ids.push(id);
                    }
                    None => {
                        let id: i64 = insert
                            .query_row(params![
                                photo.folder_id,
                                photo.path,
                                photo.filename,
                                photo.width,
                                photo.height,
                                photo.bytes as i64,
                                photo.taken_at,
                                photo.modified_at,
                                photo.camera,
                                photo.gps_lat,
                                photo.gps_lon,
                                version,
                                record.star.unwrap_or(false) as i64,
                                record.caption.clone().unwrap_or_default(),
                                crate::types::turn(record.rotation.unwrap_or(0), 0),
                                photo.duration,
                                now,
                                photo.lens,
                                f64::from(photo.focal_mm),
                                record.pin.map(|(lat, _)| lat),
                                record.pin.map(|(_, lon)| lon),
                            ], |row| row.get(0))
                            .map_err(|err| err.to_string())?;
                        result.inserted += 1;
                        result.ids.push(id);
                    }
                }
            }
        }
        tx.commit().map_err(|err| err.to_string())?;
        Ok(result)
    }

    /// Runs `sql` once per id in one transaction; `?1` is the id.
    fn for_each_id(&self, sql: &str, ids: &[i64], extra: &[&dyn rusqlite::ToSql]) {
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return;
        };
        if let Ok(mut stmt) = tx.prepare(sql) {
            for id in ids {
                let mut values: Vec<&dyn rusqlite::ToSql> = vec![id];
                values.extend_from_slice(extra);
                let _ = stmt.execute(values.as_slice());
            }
        }
        let _ = tx.commit();
    }

    pub fn set_pins(&self, ids: &[i64], lat: f64, lon: f64) {
        self.for_each_id("UPDATE photos SET pin_lat=?2, pin_lon=?3 WHERE id=?1", ids, &[&lat, &lon]);
    }

    /// Pins each photo in `pins` to its own place and unpins `cleared`, all
    /// in one transaction.
    pub fn set_pin_each(&self, pins: &[(i64, f64, f64)], cleared: &[i64]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|err| err.to_string())?;
        {
            let mut pin = tx.prepare("UPDATE photos SET pin_lat=?2, pin_lon=?3 WHERE id=?1").map_err(|err| err.to_string())?;
            for (id, lat, lon) in pins {
                pin.execute(params![id, lat, lon]).map_err(|err| err.to_string())?;
            }
            let mut unpin = tx.prepare("UPDATE photos SET pin_lat=NULL, pin_lon=NULL WHERE id=?1").map_err(|err| err.to_string())?;
            for id in cleared {
                unpin.execute([id]).map_err(|err| err.to_string())?;
            }
        }
        tx.commit().map_err(|err| err.to_string())
    }

    pub fn clear_pins(&self, ids: &[i64]) {
        self.for_each_id("UPDATE photos SET pin_lat=NULL, pin_lon=NULL WHERE id=?1", ids, &[]);
    }

    pub fn set_stars(&self, ids: &[i64], starred: bool) {
        self.for_each_id("UPDATE photos SET starred=?2 WHERE id=?1", ids, &[&(starred as i64)]);
    }

    /// Deletes rows of `folder_id` whose file was not seen by a complete scan.
    /// Their stars, captions, albums and edits go to the archive first.
    pub fn remove_missing(&self, folder_id: i64, seen: &[String]) -> usize {
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return 0;
        };
        let _ = tx.execute("CREATE TEMP TABLE IF NOT EXISTS seen_paths(path TEXT PRIMARY KEY)", []);
        let _ = tx.execute("DELETE FROM seen_paths", []);
        if let Ok(mut stmt) = tx.prepare("INSERT OR IGNORE INTO seen_paths(path) VALUES(?1)") {
            for path in seen {
                let _ = stmt.execute([path]);
            }
        }
        let archive = format!(
            "{ARCHIVE_SQL}p.folder_id=?1 AND p.path NOT IN (SELECT path FROM seen_paths) AND {HAS_USER_DATA}"
        );
        if let Err(err) = tx.execute(&archive, params![folder_id, unix_now()]) {
            eprintln!("picasso archive: {err}");
            return 0;
        }
        let removed = tx
            .execute(
                "DELETE FROM photos WHERE folder_id=?1 AND path NOT IN (SELECT path FROM seen_paths)",
                [folder_id],
            )
            .unwrap_or(0);
        let _ = tx.execute("DELETE FROM seen_paths", []);
        let _ = tx.commit();
        removed
    }

    /// Size and modification time of every stored track under
    /// `folder_path`, by path, to skip unchanged files.
    pub fn track_snapshot(&self, folder_id: i64, folder_path: &str) -> HashMap<String, (u64, i64)> {
        let (start, end) = under_dir(folder_path);
        let rows = self.map_rows(
            "SELECT path, bytes, modified_at FROM tracks WHERE folder_id=?1 OR (path >= ?2 AND path < ?3)",
            params![folder_id, start, end],
            |row| Ok((row.get::<_, String>(0)?, (row.get::<_, i64>(1)? as u64, row.get::<_, i64>(2)?))),
        );
        rows.into_iter().collect()
    }

    pub fn store_track(&self, folder_id: i64, path: &str, bytes: u64, modified_at: i64, track: &Track) -> Result<(), String> {
        let span = track.time_span();
        self.conn
            .execute(
                "INSERT INTO tracks(folder_id, path, name, bytes, modified_at, started_at, ended_at, points)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(path) DO UPDATE SET folder_id=excluded.folder_id, name=excluded.name,
                    bytes=excluded.bytes, modified_at=excluded.modified_at, started_at=excluded.started_at,
                    ended_at=excluded.ended_at, points=excluded.points",
                params![
                    folder_id,
                    path,
                    track.name,
                    bytes as i64,
                    modified_at,
                    span.map(|(start, _)| start),
                    span.map(|(_, end)| end),
                    track.encode()
                ],
            )
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// Drops a stored track whose file can no longer be read. True if there was one.
    pub fn forget_track(&self, path: &str) -> bool {
        self.conn.execute("DELETE FROM tracks WHERE path=?1", [path]).unwrap_or(0) > 0
    }

    /// Deletes tracks of `folder_id` whose file a complete scan did not see.
    pub fn remove_missing_tracks(&self, folder_id: i64, seen: &[String]) -> usize {
        let seen: HashSet<&str> = seen.iter().map(String::as_str).collect();
        let stored: Vec<(i64, String)> =
            self.map_rows("SELECT id, path FROM tracks WHERE folder_id=?1", [folder_id], |row| Ok((row.get(0)?, row.get(1)?)));
        stored
            .into_iter()
            .filter(|(_, path)| !seen.contains(path.as_str()))
            .filter(|(id, _)| self.conn.execute("DELETE FROM tracks WHERE id=?1", [id]).unwrap_or(0) > 0)
            .count()
    }

    /// Every stored track as (id, folder id, path, track), oldest first.
    pub fn tracks(&self) -> Vec<(i64, i64, String, Track)> {
        self.map_rows(
            "SELECT id, folder_id, path, name, points FROM tracks ORDER BY started_at, path",
            [],
            |row| {
                let points: Vec<u8> = row.get(4)?;
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, Track::decode(row.get(3)?, &points)))
            },
        )
    }

    pub fn photos(&self, nav: &Nav, search: &str, sort: Sort) -> Vec<Photo> {
        let search = search.trim();
        if !search.is_empty() {
            return self.search(search, sort);
        }
        let order = order_sql(sort);
        match nav {
            Nav::Library | Nav::Statistics => {
                let sql = format!("SELECT {PHOTO_COLS} FROM photos p ORDER BY {order}");
                self.map_rows(&sql, [], read_photo)
            }
            Nav::Starred => {
                let sql = format!("SELECT {PHOTO_COLS} FROM photos p WHERE p.starred=1 ORDER BY {order}");
                self.map_rows(&sql, [], read_photo)
            }
            Nav::Folder(id) => {
                let sql = format!("SELECT {PHOTO_COLS} FROM photos p WHERE p.folder_id=?1 ORDER BY {order}");
                self.map_rows(&sql, [*id], read_photo)
            }
            Nav::Dir(dir) => {
                let (start, end) = under_dir(dir);
                let sql = format!("SELECT {PHOTO_COLS} FROM photos p WHERE p.path >= ?1 AND p.path < ?2 ORDER BY {order}");
                self.map_rows(&sql, params![start, end], read_photo)
            }
            Nav::Album(id) => {
                let sql = format!(
                    "SELECT {PHOTO_COLS} FROM photos p
                     JOIN album_photos ap ON ap.photo_id = p.id
                     WHERE ap.album_id=?1 ORDER BY {order}"
                );
                self.map_rows(&sql, [*id], read_photo)
            }
            Nav::Places | Nav::TravelMap => {
                let sql = format!(
                    "SELECT {PHOTO_COLS} FROM photos p
                     WHERE (p.pin_lat IS NOT NULL AND p.pin_lon IS NOT NULL)
                        OR (p.gps_lat IS NOT NULL AND p.gps_lon IS NOT NULL)
                     ORDER BY {order}"
                );
                self.map_rows(&sql, [], read_photo)
            }
        }
    }

    /// Id and path of every photo, newest first.
    pub fn photo_paths(&self) -> Vec<(i64, String)> {
        self.map_rows("SELECT id, path FROM photos ORDER BY taken_at DESC", [], |row| Ok((row.get(0)?, row.get(1)?)))
    }

    /// Photos matching every word of `query`. See [`search_terms`].
    fn search(&self, query: &str, sort: Sort) -> Vec<Photo> {
        let mut clauses = Vec::new();
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        for term in search_terms(query) {
            let at = values.len() + 1;
            match term {
                Term::Dates(start, end) => {
                    clauses.push(format!("(p.taken_at >= ?{at} AND p.taken_at < ?{})", at + 1));
                    values.extend([start.into(), end.into()]);
                }
                Term::Text(word) => {
                    clauses.push(format!(
                        "(substr(p.path, length(f.path) + 1) LIKE ?{at} ESCAPE '\\'
                          OR p.caption LIKE ?{at} ESCAPE '\\'
                          OR p.camera LIKE ?{at} ESCAPE '\\'
                          OR p.lens LIKE ?{at} ESCAPE '\\'
                          OR EXISTS (SELECT 1 FROM album_photos ap JOIN albums a ON a.id = ap.album_id
                                     WHERE ap.photo_id = p.id AND a.name LIKE ?{at} ESCAPE '\\'))"
                    ));
                    values.push(like_pattern(&word).into());
                }
            }
        }
        if clauses.is_empty() {
            return Vec::new();
        }
        let sql = format!(
            "SELECT {PHOTO_COLS}
             FROM photos p JOIN folders f ON f.id = p.folder_id
             WHERE {}
             ORDER BY {}",
            clauses.join(" AND "),
            order_sql(sort)
        );
        self.map_rows(&sql, rusqlite::params_from_iter(values), read_photo)
    }

    pub fn set_star(&self, id: i64, starred: bool) {
        let _ = self.conn.execute(
            "UPDATE photos SET starred=?1 WHERE id=?2",
            params![if starred { 1 } else { 0 }, id],
        );
    }

    pub fn set_caption(&self, id: i64, caption: &str) {
        let _ = self
            .conn
            .execute("UPDATE photos SET caption=?1 WHERE id=?2", params![caption, id]);
    }

    /// Turns photos by `delta` degrees (a multiple of 90, positive is
    /// clockwise). Only the catalog changes; the image files stay untouched.
    pub fn rotate(&self, ids: &[i64], delta: i32) {
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return;
        };
        if let Ok(mut stmt) = tx.prepare(
            "UPDATE photos SET rotation = (((rotation + ?1) % 360 + 360) % 360 / 90) * 90 WHERE id=?2",
        ) {
            for id in ids {
                let _ = stmt.execute(params![delta, id]);
            }
        }
        let _ = tx.commit();
    }

    /// Deletes photo rows, keeping their user data in the archive.
    pub fn delete_photos(&self, ids: &[i64]) {
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return;
        };
        let archive = format!("{ARCHIVE_SQL}p.id=?1 AND {HAS_USER_DATA}");
        let now = unix_now();
        if let (Ok(mut archive), Ok(mut delete)) = (tx.prepare(&archive), tx.prepare("DELETE FROM photos WHERE id=?1")) {
            for id in ids {
                if archive.execute(params![id, now]).is_ok() {
                    let _ = delete.execute([id]);
                }
            }
        }
        let _ = tx.commit();
    }

    /// Gives the stars, captions, albums and edits of removed photos back to
    /// the photos that took their place: a photo at the same path, or else
    /// the one new photo with the same size, modification time and date.
    /// Returns the ids of the photos that got data back.
    pub fn adopt_archived(&self) -> Vec<i64> {
        let entries: Vec<(String, i64, i64, i64, String, i64)> = self.map_rows(
            "SELECT path, bytes, modified_at, taken_at, data, removed_at FROM photo_archive",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        );
        if entries.is_empty() {
            return Vec::new();
        }
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return Vec::new();
        };
        let mut adopted = Vec::new();
        {
            let by_path = tx.prepare("SELECT id FROM photos WHERE path=?1");
            let moved = tx.prepare(&format!(
                "SELECT p.id FROM photos p WHERE p.bytes=?1 AND p.modified_at=?2 AND p.file_taken_at=?3
                 AND p.added_at >= ?4 AND NOT {HAS_USER_DATA} LIMIT 2"
            ));
            let forget = tx.prepare("DELETE FROM photo_archive WHERE path=?1");
            let (Ok(mut by_path), Ok(mut moved), Ok(mut forget)) = (by_path, moved, forget) else {
                return Vec::new();
            };
            for (path, bytes, modified_at, taken_at, data, removed_at) in entries {
                let mut id: Option<i64> = by_path.query_row([&path], |row| row.get(0)).optional().ok().flatten();
                if id.is_none() {
                    let found: Vec<i64> = moved
                        .query_map(params![bytes, modified_at, taken_at, removed_at - 86_400], |row| row.get(0))
                        .map(|rows| rows.filter_map(Result::ok).collect())
                        .unwrap_or_default();
                    if let [one] = found[..] {
                        id = Some(one);
                    }
                }
                let Some(id) = id else {
                    continue;
                };
                restore_user_data(&tx, id, &data);
                let _ = forget.execute([&path]);
                adopted.push(id);
            }
        }
        if tx.commit().is_err() {
            return Vec::new();
        }
        adopted
    }

    /// Drops archive entries of photos removed before `cutoff`.
    pub fn purge_archive(&self, cutoff: i64) -> usize {
        self.conn.execute("DELETE FROM photo_archive WHERE removed_at < ?1", [cutoff]).unwrap_or(0)
    }

    pub fn albums(&self) -> Vec<Album> {
        self.map_rows(
            "SELECT a.id, a.name, COUNT(ap.photo_id)
             FROM albums a
             LEFT JOIN album_photos ap ON ap.album_id = a.id
             GROUP BY a.id
             ORDER BY a.name COLLATE NOCASE",
            [],
            |row| {
                Ok(Album {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    count: row.get(2)?,
                })
            },
        )
    }

    pub fn create_album(&self, name: &str) -> Result<i64, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(crate::i18n::tr("Album name is empty").into());
        }
        self.conn
            .execute(
                "INSERT INTO albums(name, created_at) VALUES(?1, ?2)",
                params![name, unix_now()],
            )
            .map_err(|err| err.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_album(&self, id: i64) {
        let _ = self.conn.execute("DELETE FROM albums WHERE id=?1", [id]);
    }

    pub fn add_to_album(&self, album_id: i64, photo_ids: &[i64]) {
        self.for_each_id(
            "INSERT OR IGNORE INTO album_photos(album_id, photo_id) VALUES(?2, ?1)",
            photo_ids,
            &[&album_id],
        );
    }

    pub fn remove_from_album(&self, album_id: i64, photo_ids: &[i64]) {
        self.for_each_id("DELETE FROM album_photos WHERE album_id=?2 AND photo_id=?1", photo_ids, &[&album_id]);
    }

    fn map_rows<T, P, F>(&self, sql: &str, params: P, mut map: F) -> Vec<T>
    where
        P: rusqlite::Params,
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        let mut stmt = match self.conn.prepare(sql) {
            Ok(stmt) => stmt,
            Err(err) => {
                eprintln!("picasso sql: {err}\n{sql}");
                return Vec::new();
            }
        };
        let collected = match stmt.query_map(params, |row| map(row)) {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(err) => {
                eprintln!("picasso sql: {err}\n{sql}");
                Vec::new()
            }
        };
        collected
    }
}

/// Puts archived user data on photo `id`. Anything the photo already has,
/// such as a star read from `.picasa.ini`, is kept.
fn restore_user_data(conn: &Connection, id: i64, data: &str) {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(data) else {
        return;
    };
    let _ = conn.execute(
        "UPDATE photos SET
            starred = MAX(starred, ?2),
            caption = CASE WHEN caption = '' THEN ?3 ELSE caption END,
            rotation = CASE WHEN rotation = 0 THEN ?4 ELSE rotation END,
            pin_lat = CASE WHEN pin_lat IS NULL THEN ?5 ELSE pin_lat END,
            pin_lon = CASE WHEN pin_lat IS NULL THEN ?6 ELSE pin_lon END
         WHERE id=?1",
        params![
            id,
            data["starred"].as_i64().unwrap_or(0),
            data["caption"].as_str().unwrap_or(""),
            data["rotation"].as_i64().unwrap_or(0),
            data["pin_lat"].as_f64(),
            data["pin_lon"].as_f64(),
        ],
    );
    if let Some(taken_at) = data["taken_set"].as_i64() {
        let _ = conn.execute(
            "UPDATE photos SET taken_at=?2, date_set=1 WHERE id=?1 AND date_set=0",
            params![id, taken_at],
        );
    }
    for album in data["albums"].as_array().into_iter().flatten().filter_map(|album| album.as_i64()) {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO album_photos(album_id, photo_id)
             SELECT id, ?2 FROM albums WHERE id=?1",
            params![album, id],
        );
    }
    if let Some(steps) = data["steps"].as_str() {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO edits(photo_id, steps, applied, updated_at) VALUES(?1, ?2, ?3, ?4)",
            params![id, steps, data["applied"].as_i64().unwrap_or(0), unix_now()],
        );
    }
}

fn read_photo(row: &rusqlite::Row<'_>) -> rusqlite::Result<Photo> {
    Ok(Photo {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        path: row.get(2)?,
        filename: row.get(3)?,
        width: row.get::<_, i64>(4)? as u32,
        height: row.get::<_, i64>(5)? as u32,
        bytes: row.get::<_, i64>(6)? as u64,
        taken_at: row.get(7)?,
        modified_at: row.get(8)?,
        starred: row.get::<_, i64>(9)? != 0,
        caption: row.get(10)?,
        camera: row.get(11)?,
        gps_lat: row.get(12)?,
        gps_lon: row.get(13)?,
        pin_lat: row.get(14)?,
        pin_lon: row.get(15)?,
        rotation: row.get::<_, i64>(16)? as i32,
        duration: row.get(17)?,
        lens: row.get(18)?,
        focal_mm: row.get::<_, f64>(19)? as f32,
    })
}

fn order_sql(sort: Sort) -> &'static str {
    match sort {
        Sort::DateNewest => "p.taken_at DESC, p.filename COLLATE NOCASE ASC",
        Sort::DateOldest => "p.taken_at ASC, p.filename COLLATE NOCASE ASC",
        Sort::Name | Sort::NameEdited => "p.filename COLLATE NOCASE ASC",
    }
}

#[derive(Debug, PartialEq)]
enum Term {
    /// Photos taken from the first time up to, not including, the second.
    Dates(i64, i64),
    Text(String),
}

/// The words of a search. A year, month, or day such as `2019`, `2019-05`,
/// or `2019/5/17` stands for the photos taken then, in local time; any
/// other word is looked for in the file and subfolder names, the caption,
/// the camera, the lens, and the names of the photo's albums.
fn search_terms(query: &str) -> Vec<Term> {
    query
        .split_whitespace()
        .map(|word| match date_range(word) {
            Some((start, end)) => Term::Dates(start, end),
            None => Term::Text(word.to_string()),
        })
        .collect()
}

fn date_range(word: &str) -> Option<(i64, i64)> {
    use chrono::{Months, NaiveDate, TimeZone};

    let parts: Vec<&str> = word.split(['-', '/', '.']).collect();
    if parts.len() > 3 || parts[0].len() != 4 || parts.iter().any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let numbers: Vec<u32> = parts.iter().map(|part| part.parse().ok()).collect::<Option<_>>()?;
    let year = i32::try_from(numbers[0]).ok().filter(|year| (1800..=2200).contains(year))?;
    let month = numbers.get(1).copied().unwrap_or(1);
    let day = numbers.get(2).copied().unwrap_or(1);
    let start = NaiveDate::from_ymd_opt(year, month, day)?;
    let end = match parts.len() {
        1 => start.checked_add_months(Months::new(12))?,
        2 => start.checked_add_months(Months::new(1))?,
        _ => start.succ_opt()?,
    };
    let local = |date: NaiveDate| chrono::Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).earliest().map(|at| at.timestamp());
    Some((local(start)?, local(end)?))
}

fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// Bounds of the paths under `dir`: every path that starts with `dir` and a
/// separator sorts at or after the first and before the second, so the
/// query walks the path index instead of reading every row.
fn under_dir(dir: &str) -> (String, String) {
    const SEP: char = std::path::MAIN_SEPARATOR;
    let mut start = dir.trim_end_matches(SEP).to_string();
    start.push(SEP);
    let mut end = start.clone();
    end.pop();
    end.push(char::from(SEP as u8 + 1));
    (start, end)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn app_dirs() -> (PathBuf, PathBuf) {
    if let Some(dirs) = directories::ProjectDirs::from("app", "Picasso", "picasso") {
        (dirs.data_dir().to_path_buf(), dirs.cache_dir().to_path_buf())
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        (
            PathBuf::from(&home).join(".local/share/picasso"),
            PathBuf::from(&home).join(".cache/picasso"),
        )
    }
}

/// Folder for `settings.json`, the system's place for Picasso's settings.
pub fn config_dir() -> PathBuf {
    match directories::ProjectDirs::from("app", "Picasso", "picasso") {
        Some(dirs) => dirs.config_dir().to_path_buf(),
        None => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config/picasso"),
    }
}

pub fn pictures_dir() -> Option<PathBuf> {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.picture_dir().map(|path| path.to_path_buf()))
        .filter(|path| path.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(folder_id: i64, folder: &Path, name: &str, modified_at: i64) -> ScanRecord {
        ScanRecord {
            photo: NewPhoto {
                folder_id,
                path: folder.join(name).to_string_lossy().into_owned(),
                filename: name.into(),
                width: 100,
                height: 80,
                bytes: 10,
                taken_at: 1_700_000_000,
                modified_at,
                camera: "TestCam".into(),
                lens: String::new(),
                focal_mm: 0.0,
                gps_lat: Some(37.5),
                gps_lon: Some(-122.4),
                duration: 0.0,
            },
            existing: None,
            star: None,
            caption: None,
            rotation: None,
            pin: None,
            complete: true,
        }
    }

    #[test]
    fn subfolders_count_and_list_recursively() {
        let dir = temp_dir("dirs");
        let db = Database::open(&dir.join("t.db")).unwrap();
        let folder = dir.join("pics");
        std::fs::create_dir_all(&folder).unwrap();
        let id = db.add_folder(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        let trip = folder.join("trip");
        let day = trip.join("day1");
        db.write_batch(&[
            record(id, &folder, "a.jpg", 1),
            record(id, &trip, "b.jpg", 1),
            record(id, &day, "c.jpg", 1),
            record(id, &day, "d.jpg", 1),
        ])
        .unwrap();
        let text = |path: &Path| path.to_string_lossy().into_owned();
        let mut dirs = db.photo_dirs();
        dirs.sort();
        let mut expected = vec![(text(&folder), 1), (text(&trip), 1), (text(&day), 2)];
        expected.sort();
        assert_eq!(dirs, expected);
        assert_eq!(db.photos(&Nav::Dir(text(&trip)), "", Sort::Name).len(), 3);
        assert_eq!(db.photos(&Nav::Dir(text(&day)), "", Sort::Name).len(), 2);
        assert_eq!(db.photos(&Nav::Dir(format!("{}x", text(&trip))), "", Sort::Name).len(), 0);
    }

    #[test]
    fn folder_ranges_match_a_prefix_search() {
        let (db, id, folder) = library("ranges");
        let names = ["trip", "trip x", "trip]", "trip^", "tripa", "旅行", "trip\\deep", "trip/deep", "z"];
        let records: Vec<ScanRecord> = names
            .iter()
            .flat_map(|name| [record(id, &folder.join(name), "a.jpg", 1), record(id, &folder.join(name), "照片.heic", 1)])
            .collect();
        db.write_batch(&records).unwrap();
        db.write_batch(&[record(id, &folder, "trip.jpg", 1)]).unwrap();
        for name in names.iter().chain(&["", "tri", "nothing"]) {
            let dir = folder.join(name).to_string_lossy().into_owned();
            let mut prefix = dir.trim_end_matches(std::path::MAIN_SEPARATOR).to_string();
            prefix.push(std::path::MAIN_SEPARATOR);
            let mut expected: Vec<String> = db.map_rows(
                "SELECT path FROM photos WHERE substr(path, 1, ?2) = ?1",
                params![prefix, prefix.chars().count() as i64],
                |row| row.get(0),
            );
            expected.sort();
            let mut found: Vec<String> = db.photos(&Nav::Dir(dir.clone()), "", Sort::Name).into_iter().map(|photo| photo.path).collect();
            found.sort();
            assert_eq!(found, expected, "{dir}");
            let mut snapshot: Vec<String> = db.folder_snapshot(-1, &dir).into_keys().collect();
            snapshot.sort();
            assert_eq!(snapshot, expected, "{dir}");
        }
    }

    #[test]
    fn catalog_roundtrip() {
        let dir = temp_dir("db");
        let db = Database::open(&dir.join("t.db")).unwrap();
        let folder = dir.join("pics");
        std::fs::create_dir_all(&folder).unwrap();
        let id = db.add_folder(&folder).unwrap();
        assert_eq!(db.add_folder(&folder).unwrap(), id);
        let folder = folder.canonicalize().unwrap();
        let mut first = record(id, &folder, "a.jpg", 10);
        first.star = Some(true);
        first.rotation = Some(270);
        first.caption = Some("from ini".into());
        let result = db.write_batch(&[first, record(id, &folder, "b.jpg", 10)]).unwrap();
        assert_eq!((result.inserted, result.updated, result.ids.len()), (2, 0, 2));
        let snapshot = db.folder_snapshot(id, &folder.to_string_lossy());
        let known = snapshot[&folder.join("a.jpg").to_string_lossy().into_owned()];
        assert!(known.current(10, 10));
        assert!(!known.current(11, 10));
        let photo_id = known.id;
        assert_eq!(result.ids[0], photo_id);
        assert_eq!(db.photo(photo_id).unwrap().caption, "from ini");
        assert_eq!(db.photo(photo_id).unwrap().rotation, 270);
        db.save_edit_history(photo_id, r#"[{"control":"exposure","value":0.5}]"#, 1, 1);
        assert_eq!(db.edit_history(photo_id).map(|(_, applied)| applied), Some(1));
        assert_eq!(db.edit_histories().len(), 1);
        db.save_edit_history(photo_id, "[]", 0, 0);
        assert!(db.edit_history(photo_id).is_none());
        db.rotate(&[photo_id], 90);
        assert_eq!(db.photo(photo_id).unwrap().rotation, 0);
        db.rotate(&[photo_id], -90);
        db.rotate(&[photo_id], -90);
        assert_eq!(db.photo(photo_id).unwrap().rotation, 180);

        db.set_caption(photo_id, "hello");
        let album = db.create_album("Trip").unwrap();
        db.add_to_album(album, &[photo_id]);
        db.set_pins(&[photo_id], 1.0, 2.0);

        let mut changed = record(id, &folder, "a.jpg", 20);
        changed.existing = Some(photo_id);
        changed.photo.width = 300;
        let result = db.write_batch(&[changed]).unwrap();
        assert_eq!(result, BatchResult { inserted: 0, updated: 1, ids: vec![photo_id] });
        let photo = db.photo(photo_id).unwrap();
        assert_eq!(photo.width, 300);
        assert!(photo.starred);
        assert_eq!(photo.caption, "hello");
        assert!(photo.pinned());
        assert_eq!(photo.rotation, 180, "a rescan keeps the stored rotation");

        assert_eq!(db.photos(&Nav::Starred, "", Sort::Name).len(), 1);
        assert_eq!(db.photos(&Nav::Album(album), "", Sort::Name).len(), 1);
        assert_eq!(db.photos(&Nav::Library, "hello", Sort::Name).len(), 1);
        assert_eq!(db.photos(&Nav::Places, "", Sort::Name).len(), 2);
        db.clear_pins(&[photo_id]);
        let cleared = db.photo(photo_id).unwrap();
        assert!(!cleared.pinned());
        assert!((cleared.place().unwrap().0 - 37.5).abs() < 1e-6);

        let keep = vec![folder.join("a.jpg").to_string_lossy().into_owned()];
        assert_eq!(db.remove_missing(id, &keep), 1);
        assert_eq!(db.photos(&Nav::Library, "", Sort::Name).len(), 1);
    }

    fn archive_count(db: &Database) -> i64 {
        db.conn.query_row("SELECT COUNT(*) FROM photo_archive", [], |row| row.get(0)).unwrap()
    }

    fn library(tag: &str) -> (Database, i64, PathBuf) {
        let dir = temp_dir(tag);
        let db = Database::open(&dir.join("t.db")).unwrap();
        let folder = dir.join("pics");
        std::fs::create_dir_all(&folder).unwrap();
        let id = db.add_folder(&folder).unwrap();
        (db, id, folder.canonicalize().unwrap())
    }

    #[test]
    fn a_changed_date_taken_outlasts_rescans_and_removal() {
        let (db, folder_id, folder) = library("date-set");
        let id = db.write_batch(&[record(folder_id, &folder, "a.jpg", 10)]).unwrap().ids[0];
        db.shift_taken(&[id], 3600);
        assert_eq!(db.photo(id).unwrap().taken_at, 1_700_003_600);

        // Rescanned as a known file and as a new one, the date stays.
        let mut known = record(folder_id, &folder, "a.jpg", 11);
        known.existing = Some(id);
        db.write_batch(&[known]).unwrap();
        db.write_batch(&[record(folder_id, &folder, "a.jpg", 12)]).unwrap();
        assert_eq!(db.photo(id).unwrap().taken_at, 1_700_003_600);

        // It comes back with the photo after the file returns.
        assert_eq!(db.remove_missing(folder_id, &[]), 1);
        let back = db.write_batch(&[record(folder_id, &folder, "a.jpg", 12)]).unwrap().ids[0];
        assert_eq!(db.adopt_archived(), vec![back]);
        assert_eq!(db.photo(back).unwrap().taken_at, 1_700_003_600);

        db.reset_taken(&[back]);
        assert_eq!(db.photo(back).unwrap().taken_at, 1_700_000_000);
        db.write_batch(&[record(folder_id, &folder, "a.jpg", 13)]).unwrap();
        assert_eq!(db.photo(back).unwrap().taken_at, 1_700_000_000);
    }

    #[test]
    fn excluding_a_subfolder_drops_its_photos_until_included_again() {
        let (db, folder_id, folder) = library("exclude");
        let skip = folder.join("skip");
        std::fs::create_dir_all(skip.join("deep")).unwrap();
        let ids = db
            .write_batch(&[
                record(folder_id, &folder, "a.jpg", 10),
                record(folder_id, &skip, "b.jpg", 10),
                record(folder_id, &skip.join("deep"), "c.jpg", 10),
                record(folder_id, &folder, "skipped.jpg", 10),
            ])
            .unwrap()
            .ids;
        db.set_stars(&[ids[1]], true);

        assert!(db.exclude_dir(&folder).is_err(), "an added folder itself can't be excluded");
        assert!(db.exclude_dir(&std::env::temp_dir()).is_err(), "a folder outside the library can't be excluded");
        let (owner, stored) = db.exclude_dir(&skip).unwrap();
        assert_eq!(owner, folder_id);
        assert_eq!(db.excluded_dirs(), vec![(stored.clone(), folder_id)]);
        let mut left: Vec<i64> = db.photos(&Nav::Folder(folder_id), "", Sort::Name).iter().map(|photo| photo.id).collect();
        left.sort();
        assert_eq!(left, [ids[0], ids[3]]);
        assert_eq!(archive_count(&db), 1);

        assert_eq!(db.include_dir(&stored), Some(folder_id));
        assert!(db.excluded_dirs().is_empty());
        let back = db.write_batch(&[record(folder_id, &skip, "b.jpg", 10)]).unwrap().ids[0];
        assert_eq!(db.adopt_archived(), vec![back]);
        assert!(db.photo(back).unwrap().starred);

        db.exclude_dir(&skip).unwrap();
        db.remove_folder(folder_id);
        assert!(db.excluded_dirs().is_empty());
    }

    #[test]
    fn tracks_are_stored_replaced_and_removed_with_their_folder() {
        use crate::track::Point;
        let (db, folder_id, folder) = library("tracks");
        let track = |name: &str, lat: f64| Track {
            name: name.into(),
            segments: vec![vec![
                Point { lat, lon: 121.0, time: Some(100) },
                Point { lat: lat + 0.01, lon: 121.0, time: Some(160) },
            ]],
        };
        let ride = folder_path(&folder, "ride.gpx");
        let walk = folder_path(&folder.join("skip"), "walk.kml");
        db.store_track(folder_id, &ride, 10, 1, &track("Ride", 25.0)).unwrap();
        db.store_track(folder_id, &walk, 20, 2, &track("Walk", 24.0)).unwrap();
        db.store_track(folder_id, &ride, 11, 3, &track("Ride again", 25.5)).unwrap();
        let stored = db.tracks();
        assert_eq!(stored.len(), 2);
        let (_, owner, path, again) = stored.iter().find(|(_, _, path, _)| *path == ride).unwrap();
        assert_eq!((*owner, path.as_str()), (folder_id, ride.as_str()));
        assert_eq!(again.name, "Ride again");
        assert_eq!(again.segments[0][0], Point { lat: 25.5, lon: 121.0, time: Some(100) });
        assert_eq!(db.track_snapshot(folder_id, &folder.to_string_lossy()).get(&ride), Some(&(11, 3)));

        assert_eq!(db.remove_missing_tracks(folder_id, std::slice::from_ref(&ride)), 1);
        assert_eq!(db.tracks().len(), 1);
        db.store_track(folder_id, &walk, 20, 2, &track("Walk", 24.0)).unwrap();
        std::fs::create_dir_all(folder.join("skip")).unwrap();
        db.exclude_dir(&folder.join("skip")).unwrap();
        assert_eq!(db.tracks().len(), 1);
        assert!(db.forget_track(&ride));
        assert!(!db.forget_track(&ride));
        db.store_track(folder_id, &ride, 10, 1, &track("Ride", 25.0)).unwrap();
        db.remove_folder(folder_id);
        assert!(db.tracks().is_empty());
    }

    #[test]
    fn photo_ids_are_never_used_again() {
        let (db, folder_id, folder) = library("id-floor");
        let ids = db.write_batch(&[record(folder_id, &folder, "a.jpg", 10), record(folder_id, &folder, "b.jpg", 10)])
            .unwrap()
            .ids;
        db.delete_photos(&[ids[1]]);
        let c = db.write_batch(&[record(folder_id, &folder, "c.jpg", 10)]).unwrap().ids[0];
        assert!(c > ids[1], "a trashed photo's id came back");

        let other = folder.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let other_id = db.add_folder(&other).unwrap();
        let d = db.write_batch(&[record(other_id, &other, "d.jpg", 10)]).unwrap().ids[0];
        db.remove_folder(other_id);
        let e = db.write_batch(&[record(folder_id, &folder, "e.jpg", 10)]).unwrap().ids[0];
        assert!(e > d, "an id from a removed folder came back");
        assert_eq!(db.remove_missing(folder_id, &[]), 3);
        let f = db.write_batch(&[record(folder_id, &folder, "f.jpg", 10)]).unwrap().ids[0];
        assert!(f > e, "an id from a rescan came back");
    }

    #[test]
    fn removed_photos_get_their_data_back_at_the_same_path() {
        let (db, folder_id, folder) = library("archive-path");
        let ids = db.write_batch(&[record(folder_id, &folder, "a.jpg", 10), record(folder_id, &folder, "plain.jpg", 10)])
            .unwrap()
            .ids;
        let album = db.create_album("Trip").unwrap();
        db.set_stars(&[ids[0]], true);
        db.set_caption(ids[0], "beach");
        db.rotate(&[ids[0]], 90);
        db.set_pins(&[ids[0]], 1.0, 2.0);
        db.add_to_album(album, &[ids[0]]);
        db.save_edit_history(ids[0], r#"[{"control":"exposure","value":0.5}]"#, 1, 1);

        assert_eq!(db.remove_missing(folder_id, &[]), 2);
        assert_eq!(archive_count(&db), 1, "only photos with user data are archived");
        assert!(db.adopt_archived().is_empty());

        let back = db.write_batch(&[record(folder_id, &folder, "a.jpg", 10)]).unwrap().ids[0];
        assert_eq!(db.adopt_archived(), vec![back]);
        let photo = db.photo(back).unwrap();
        assert!(photo.starred);
        assert_eq!((photo.caption.as_str(), photo.rotation), ("beach", 90));
        assert_eq!((photo.pin_lat, photo.pin_lon), (Some(1.0), Some(2.0)));
        assert_eq!(db.photos(&Nav::Album(album), "", Sort::Name).len(), 1);
        assert_eq!(db.edit_history(back).map(|(_, applied)| applied), Some(1));
        assert_eq!(archive_count(&db), 0);
    }

    #[test]
    fn a_moved_photo_is_found_by_size_time_and_date() {
        let (db, folder_id, folder) = library("archive-moved");
        let id = db.write_batch(&[record(folder_id, &folder, "a.jpg", 10)]).unwrap().ids[0];
        db.set_caption(id, "kept");
        db.delete_photos(&[id]);
        assert_eq!(archive_count(&db), 1);

        let moved = db.write_batch(&[record(folder_id, &folder.join("sub"), "renamed.jpg", 10)]).unwrap().ids[0];
        db.write_batch(&[record(folder_id, &folder, "other.jpg", 11)]).unwrap();
        assert_eq!(db.adopt_archived(), vec![moved]);
        assert_eq!(db.photo(moved).unwrap().caption, "kept");
    }

    #[test]
    fn two_matching_photos_adopt_nothing() {
        let (db, folder_id, folder) = library("archive-twins");
        let id = db.write_batch(&[record(folder_id, &folder, "a.jpg", 10)]).unwrap().ids[0];
        db.set_stars(&[id], true);
        db.delete_photos(&[id]);
        db.write_batch(&[record(folder_id, &folder, "b.jpg", 10), record(folder_id, &folder, "c.jpg", 10)]).unwrap();
        assert!(db.adopt_archived().is_empty());
        assert_eq!(archive_count(&db), 1);
        assert_eq!(db.photos(&Nav::Starred, "", Sort::Name).len(), 0);
    }

    #[test]
    fn old_archive_entries_are_purged() {
        let (db, folder_id, folder) = library("archive-purge");
        let id = db.write_batch(&[record(folder_id, &folder, "a.jpg", 10)]).unwrap().ids[0];
        db.set_stars(&[id], true);
        db.delete_photos(&[id]);
        assert_eq!(db.purge_archive(unix_now() - ARCHIVE_DAYS * 86_400), 0);
        assert_eq!(archive_count(&db), 1);
        assert_eq!(db.purge_archive(unix_now() + 1), 1);
        assert_eq!(archive_count(&db), 0);
    }

    #[test]
    fn placeholder_dates_are_probed_again() {
        let dir = temp_dir("placeholder");
        let folder = dir.join("pics");
        std::fs::create_dir_all(&folder).unwrap();
        let path = |name: &str| folder.canonicalize().unwrap().join(name).to_string_lossy().into_owned();
        {
            let db = Database::open(&dir.join("t.db")).unwrap();
            let id = db.add_folder(&folder).unwrap();
            let folder = folder.canonicalize().unwrap();
            let mut year_one = record(id, &folder, "year1.jpg", 10);
            year_one.photo.taken_at = -62_135_625_600;
            db.write_batch(&[year_one, record(id, &folder, "fine.jpg", 10)]).unwrap();
            let snapshot = db.folder_snapshot(id, &folder.to_string_lossy());
            assert!(snapshot[&path("year1.jpg")].current(10, 10));
        }
        let db = Database::open(&dir.join("t.db")).unwrap();
        let id = db.folders()[0].id;
        let snapshot = db.folder_snapshot(id, &folder.canonicalize().unwrap().to_string_lossy());
        assert!(!snapshot[&path("year1.jpg")].current(10, 10));
        assert!(snapshot[&path("fine.jpg")].current(10, 10));
    }

    #[test]
    fn scan_pending_survives_reopen() {
        let dir = temp_dir("pending");
        let folder = dir.join("pics");
        std::fs::create_dir_all(&folder).unwrap();
        let id = {
            let db = Database::open(&dir.join("t.db")).unwrap();
            let id = db.add_folder(&folder).unwrap();
            assert_eq!(db.pending_folders(), vec![id]);
            id
        };
        let db = Database::open(&dir.join("t.db")).unwrap();
        assert_eq!(db.pending_folders(), vec![id]);
        db.set_scan_pending(id, false);
        assert!(db.pending_folders().is_empty());
    }

    #[test]
    fn upgrades_an_older_library() {
        let dir = temp_dir("migrate");
        let path = dir.join("old.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE folders (id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, added_at INTEGER NOT NULL);
                 INSERT INTO folders VALUES (1, '/pics', 0);
                 CREATE TABLE photos (
                    id INTEGER PRIMARY KEY,
                    folder_id INTEGER NOT NULL,
                    path TEXT NOT NULL UNIQUE,
                    filename TEXT NOT NULL,
                    width INTEGER NOT NULL DEFAULT 0,
                    height INTEGER NOT NULL DEFAULT 0,
                    bytes INTEGER NOT NULL DEFAULT 0,
                    taken_at INTEGER NOT NULL DEFAULT 0,
                    modified_at INTEGER NOT NULL DEFAULT 0,
                    starred INTEGER NOT NULL DEFAULT 0,
                    caption TEXT NOT NULL DEFAULT '',
                    rotation INTEGER NOT NULL DEFAULT 0,
                    camera TEXT NOT NULL DEFAULT '',
                    faces_scanned INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO photos(id, folder_id, path, filename, rotation) VALUES (7, 1, '/pics/a.jpg', 'a.jpg', 90);
                 CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT);
                 CREATE TABLE faces (id INTEGER PRIMARY KEY, photo_id INTEGER, person_id INTEGER);",
            )
            .unwrap();
        }
        let db = Database::open(&path).unwrap();
        for column in ["gps_lat", "gps_lon", "pin_lat", "pin_lon", "indexed_version"] {
            assert!(db.columns("photos").iter().any(|name| name == column), "{column}");
        }
        let tables: Vec<String> = db.map_rows("SELECT name FROM sqlite_master WHERE type='table'", [], |row| row.get(0));
        assert!(!tables.iter().any(|name| name == "faces" || name == "people"));
        assert_eq!(db.pending_folders(), vec![1]);
        assert_eq!(db.photo(7).unwrap().rotation, 90, "rotation from an older build is kept");
        let known = db.folder_snapshot(1, "/pics")["/pics/a.jpg"];
        assert!(!known.current(0, 0), "rows from an older build are probed again");
    }

    #[test]
    fn heif_files_without_a_camera_are_read_again_once() {
        let (db, folder_id, folder) = library("heif-exif");
        for (name, camera) in [("a.heic", ""), ("b.HIF", ""), ("c.heic", "Apple iPhone 15"), ("d.jpg", "")] {
            db.conn
                .execute(
                    "INSERT INTO photos(folder_id, path, filename, camera, indexed_version) VALUES (?1, ?2, ?3, ?4, 2)",
                    params![folder_id, folder_path(&folder, name), name, camera],
                )
                .unwrap();
        }
        db.migrate().unwrap();
        let versions: Vec<(String, i64)> =
            db.map_rows("SELECT filename, indexed_version FROM photos ORDER BY filename", [], |row| Ok((row.get(0)?, row.get(1)?)));
        assert_eq!(
            versions,
            vec![("a.heic".into(), 0), ("b.HIF".into(), 0), ("c.heic".into(), INDEX_VERSION), ("d.jpg".into(), INDEX_VERSION)]
        );
    }

    #[test]
    fn unreadable_files_are_remembered_until_a_complete_scan_drops_them() {
        let (db, folder_id, folder) = library("unreadable");
        let item = |name: &str, is_folder: bool| Unreadable {
            path: folder_path(&folder, name),
            reason: "broken".into(),
            folder: is_folder,
            bytes: 5,
            modified_at: 9,
        };
        let (bad, dir) = (item("bad.jpg", false), item("locked", true));
        db.store_unreadable(folder_id, &[bad.clone(), dir.clone()], true, &HashSet::new());
        assert_eq!(db.unreadable(), vec![(folder_id, bad.clone()), (folder_id, dir.clone())]);
        let snapshot = db.unreadable_snapshot(folder_id);
        assert_eq!(snapshot.len(), 1, "folders are listed again on every scan");
        assert_eq!(snapshot[&bad.path], bad);

        let old = unix_now() - (UNREADABLE_RETRY_DAYS + 1) * 86_400;
        db.conn.execute("UPDATE unreadable SET checked_at=?1", [old]).unwrap();
        let remembered: HashSet<String> = [bad.path.clone()].into();
        db.store_unreadable(folder_id, &[bad.clone()], false, &remembered);
        assert!(db.unreadable_snapshot(folder_id).is_empty(), "a skipped file keeps its check time, so it is tried again");
        assert_eq!(db.unreadable().len(), 2, "a partial scan only adds");

        db.store_unreadable(folder_id, &[bad.clone()], true, &HashSet::new());
        assert_eq!(db.unreadable(), vec![(folder_id, bad.clone())]);
        db.forget_unreadable(&[bad.path]);
        assert!(db.unreadable().is_empty());
    }

    #[test]
    fn search_matches_every_word_against_names_albums_and_dates() {
        let (db, folder_id, folder) = library("search");
        let mut kyoto = record(folder_id, &folder.join("Kyoto trip"), "IMG_1.jpg", 1);
        let mut later = record(folder_id, &folder.join("Kyoto trip"), "IMG_2.jpg", 1);
        let home = record(folder_id, &folder, "home.jpg", 1);
        let (start, _) = date_range("2019-05-17").unwrap();
        kyoto.photo.taken_at = start + 3600;
        later.photo.taken_at = date_range("2020").unwrap().0 + 3600;
        let ids = db.write_batch(&[kyoto, later, home]).unwrap().ids;
        let album = db.create_album("Temples").unwrap();
        db.add_to_album(album, &[ids[2]]);
        db.set_caption(ids[1], "Snow day");

        let found = |query: &str| -> Vec<i64> {
            let mut ids: Vec<i64> = db.search(query, Sort::Name).iter().map(|photo| photo.id).collect();
            ids.sort();
            ids
        };
        assert_eq!(found("kyoto"), [ids[0], ids[1]]);
        assert_eq!(found("kyoto 2019"), [ids[0]]);
        assert_eq!(found("2019-05"), [ids[0]]);
        assert_eq!(found("2019/5/17"), [ids[0]]);
        assert!(found("2019-05-18").is_empty());
        assert_eq!(found("temples"), [ids[2]]);
        assert_eq!(found("snow"), [ids[1]]);
        assert!(found("search").is_empty(), "the library folder's own name doesn't match every photo");
        assert_eq!(found("testcam").len(), 3);
    }

    #[test]
    fn dates_in_searches_cover_a_year_month_or_day() {
        let span = |word: &str| date_range(word).map(|(start, end)| (end - start) / 3600);
        assert_eq!(span("2020"), Some(366 * 24));
        assert!(matches!(span("2019-02"), Some(671..=673)));
        assert!(matches!(span("2019.5.17"), Some(23..=25)));
        assert_eq!(span("2019-13"), None);
        assert_eq!(span("19-05"), None);
        assert_eq!(span("IMG_2019"), None);
        assert_eq!(search_terms("kyoto  2019"), [Term::Text("kyoto".into()), Term::Dates(date_range("2019").unwrap().0, date_range("2019").unwrap().1)]);
    }

    fn folder_path(folder: &Path, name: &str) -> String {
        folder.join(name).to_string_lossy().into_owned()
    }
}
