//! SQLite catalog. The UI and the background indexer each open their own connection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::types::{unix_now, Album, Folder, Nav, NewPhoto, Photo, Sort};

/// Bumped whenever the scanner starts reading new metadata from files, so
/// rows written by an older build are probed once more on the next scan.
pub const INDEX_VERSION: i64 = 1;

const PHOTO_COLS: &str = "\
p.id, p.folder_id, p.path, p.filename, p.width, p.height, p.bytes, \
p.taken_at, p.modified_at, p.starred, p.caption, p.camera, \
p.gps_lat, p.gps_lon, p.pin_lat, p.pin_lon, p.rotation";

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
                CREATE INDEX IF NOT EXISTS idx_photos_folder ON photos(folder_id);
                CREATE INDEX IF NOT EXISTS idx_photos_taken ON photos(taken_at);
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
        ] {
            self.ensure_column("photos", column, decl)?;
        }
        self.ensure_column("folders", "scan_pending", "INTEGER NOT NULL DEFAULT 1")?;
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

    pub fn add_folder(&self, path: &Path) -> Result<i64, String> {
        let path = path
            .canonicalize()
            .map_err(|err| format!("Can't open folder: {err}"))?;
        if !path.is_dir() {
            return Err("That path is not a folder".into());
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
        let mut prefix = folder_path.trim_end_matches(std::path::MAIN_SEPARATOR).to_string();
        prefix.push(std::path::MAIN_SEPARATOR);
        let rows = self.map_rows(
            "SELECT path, id, modified_at, bytes, indexed_version FROM photos
             WHERE folder_id=?1 OR substr(path, 1, ?3) = ?2",
            params![folder_id, prefix, prefix.chars().count() as i64],
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
                     taken_at=?6, modified_at=?7, camera=?8, gps_lat=?9, gps_lon=?10,
                     indexed_version=?11
                     WHERE id=?12",
                )
                .map_err(|err| err.to_string())?;
            let mut insert = tx
                .prepare_cached(
                    "INSERT INTO photos(
                        folder_id, path, filename, width, height, bytes, taken_at, modified_at,
                        camera, gps_lat, gps_lon, indexed_version, starred, caption, rotation
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
                     ON CONFLICT(path) DO UPDATE SET
                        width=excluded.width, height=excluded.height, bytes=excluded.bytes,
                        taken_at=excluded.taken_at, modified_at=excluded.modified_at,
                        camera=excluded.camera, gps_lat=excluded.gps_lat, gps_lon=excluded.gps_lon,
                        indexed_version=excluded.indexed_version
                     RETURNING id",
                )
                .map_err(|err| err.to_string())?;
            for record in records {
                let photo = &record.photo;
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
                                INDEX_VERSION,
                                id,
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
                                INDEX_VERSION,
                                record.star.unwrap_or(false) as i64,
                                record.caption.clone().unwrap_or_default(),
                                crate::types::turn(record.rotation.unwrap_or(0), 0),
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

    pub fn set_pins(&self, ids: &[i64], lat: f64, lon: f64) {
        for id in ids {
            let _ = self.conn.execute(
                "UPDATE photos SET pin_lat=?1, pin_lon=?2 WHERE id=?3",
                params![lat, lon, *id],
            );
        }
    }

    pub fn clear_pins(&self, ids: &[i64]) {
        for id in ids {
            let _ = self.conn.execute("UPDATE photos SET pin_lat=NULL, pin_lon=NULL WHERE id=?1", [*id]);
        }
    }

    /// Deletes rows of `folder_id` whose file was not seen by a complete scan.
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

    pub fn photos(&self, nav: &Nav, search: &str, sort: Sort) -> Vec<Photo> {
        let search = search.trim();
        if !search.is_empty() {
            return self.search(search, sort);
        }
        let order = order_sql(sort);
        match nav {
            Nav::Library => {
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
            Nav::Album(id) => {
                let sql = format!(
                    "SELECT {PHOTO_COLS} FROM photos p
                     JOIN album_photos ap ON ap.photo_id = p.id
                     WHERE ap.album_id=?1 ORDER BY {order}"
                );
                self.map_rows(&sql, [*id], read_photo)
            }
            Nav::Places => {
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

    fn search(&self, query: &str, sort: Sort) -> Vec<Photo> {
        let pattern = like_pattern(query);
        let order = order_sql(sort);
        let sql = format!(
            "SELECT {PHOTO_COLS}
             FROM photos p
             WHERE p.filename LIKE ?1 ESCAPE '\\'
                OR p.caption LIKE ?1 ESCAPE '\\'
                OR p.camera LIKE ?1 ESCAPE '\\'
             ORDER BY {order}"
        );
        self.map_rows(&sql, [pattern], read_photo)
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

    pub fn delete_photos(&self, ids: &[i64]) {
        for id in ids {
            let _ = self.conn.execute("DELETE FROM photos WHERE id=?1", [id]);
        }
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
            return Err("Album name is empty".into());
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
        for id in photo_ids {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO album_photos(album_id, photo_id) VALUES(?1, ?2)",
                params![album_id, id],
            );
        }
    }

    pub fn remove_from_album(&self, album_id: i64, photo_ids: &[i64]) {
        for id in photo_ids {
            let _ = self.conn.execute(
                "DELETE FROM album_photos WHERE album_id=?1 AND photo_id=?2",
                params![album_id, id],
            );
        }
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
    })
}

fn order_sql(sort: Sort) -> &'static str {
    match sort {
        Sort::DateNewest => "p.taken_at DESC, p.filename COLLATE NOCASE ASC",
        Sort::DateOldest => "p.taken_at ASC, p.filename COLLATE NOCASE ASC",
        Sort::Name => "p.filename COLLATE NOCASE ASC",
    }
}

fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
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
                gps_lat: Some(37.5),
                gps_lon: Some(-122.4),
            },
            existing: None,
            star: None,
            caption: None,
            rotation: None,
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
}
