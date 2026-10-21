//! SQLite catalog. The UI and the background indexer each open their own connection.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::faces::{average_embeddings, blobs_to_vec, cosine, vec_to_blob, EMBEDDING_LEN};
use crate::geom::{clamp_rect, rotate_ccw, rotate_cw};
use crate::picasa_ini::unnamed_contact;
use crate::types::{unix_now, Album, Face, Folder, Nav, NewPhoto, Person, Photo, RelRect, Sort};

const PHOTO_COLS: &str = "\
p.id, p.folder_id, p.path, p.filename, p.width, p.height, p.bytes, \
p.taken_at, p.modified_at, p.starred, p.caption, p.rotation, p.camera, \
p.faces_scanned, (SELECT COUNT(*) FROM faces f WHERE f.photo_id = p.id)";

pub struct Database {
    conn: Connection,
}

pub enum Upsert {
    Unchanged(Photo),
    Changed(Photo),
    Inserted(i64),
}

#[derive(Clone, Debug)]
pub struct NewFace {
    pub person_id: i64,
    pub rect: RelRect,
    pub score: f32,
    pub embedding: Vec<f32>,
    pub confirmed: bool,
    pub manual: bool,
}

#[derive(Clone, Debug)]
pub struct Centroid {
    pub person_id: i64,
    pub name: Option<String>,
    pub embedding: Vec<f32>,
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
                    added_at INTEGER NOT NULL
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
                    rotation INTEGER NOT NULL DEFAULT 0,
                    camera TEXT NOT NULL DEFAULT '',
                    faces_scanned INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS people (
                    id INTEGER PRIMARY KEY,
                    name TEXT,
                    external_id TEXT,
                    centroid BLOB,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS faces (
                    id INTEGER PRIMARY KEY,
                    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
                    person_id INTEGER NOT NULL REFERENCES people(id),
                    x REAL NOT NULL,
                    y REAL NOT NULL,
                    w REAL NOT NULL,
                    h REAL NOT NULL,
                    score REAL NOT NULL,
                    embedding BLOB,
                    confirmed INTEGER NOT NULL DEFAULT 0,
                    manual INTEGER NOT NULL DEFAULT 0
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
                CREATE UNIQUE INDEX IF NOT EXISTS idx_people_ext
                    ON people(external_id) WHERE external_id IS NOT NULL;
                CREATE INDEX IF NOT EXISTS idx_photos_folder ON photos(folder_id);
                CREATE INDEX IF NOT EXISTS idx_photos_taken ON photos(taken_at);
                CREATE INDEX IF NOT EXISTS idx_faces_photo ON faces(photo_id);
                CREATE INDEX IF NOT EXISTS idx_faces_person ON faces(person_id);
                ",
            )
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
                "INSERT INTO folders(path, added_at) VALUES(?1, ?2)",
                params![text, unix_now()],
            )
            .map_err(|err| err.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn remove_folder(&self, id: i64) {
        let _ = self.conn.execute("DELETE FROM folders WHERE id=?1", [id]);
        self.delete_empty_people();
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

    pub fn photo(&self, id: i64) -> Option<Photo> {
        let sql = format!("SELECT {PHOTO_COLS} FROM photos p WHERE p.id=?1");
        self.conn
            .query_row(&sql, [id], read_photo)
            .optional()
            .ok()
            .flatten()
    }

    pub fn photo_by_path(&self, path: &str) -> Option<Photo> {
        let sql = format!("SELECT {PHOTO_COLS} FROM photos p WHERE p.path=?1");
        self.conn
            .query_row(&sql, [path], read_photo)
            .optional()
            .ok()
            .flatten()
    }

    pub fn upsert_photo(&self, photo: &NewPhoto) -> Result<Upsert, String> {
        if let Some(existing) = self.photo_by_path(&photo.path) {
            if existing.modified_at == photo.modified_at
                && existing.bytes == photo.bytes
                && existing.width == photo.width
                && existing.height == photo.height
            {
                return Ok(Upsert::Unchanged(existing));
            }
            self.conn
                .execute(
                    "UPDATE photos SET folder_id=?1, filename=?2, width=?3, height=?4, bytes=?5,
                     taken_at=?6, modified_at=?7, camera=?8, faces_scanned=0 WHERE id=?9",
                    params![
                        photo.folder_id,
                        photo.filename,
                        photo.width,
                        photo.height,
                        photo.bytes as i64,
                        photo.taken_at,
                        photo.modified_at,
                        photo.camera,
                        existing.id,
                    ],
                )
                .map_err(|err| err.to_string())?;
            return Ok(Upsert::Changed(
                self.photo(existing.id).unwrap_or(existing),
            ));
        }
        self.conn
            .execute(
                "INSERT INTO photos(
                    folder_id, path, filename, width, height, bytes, taken_at, modified_at, camera
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    photo.folder_id,
                    photo.path,
                    photo.filename,
                    photo.width,
                    photo.height,
                    photo.bytes as i64,
                    photo.taken_at,
                    photo.modified_at,
                    photo.camera,
                ],
            )
            .map_err(|err| err.to_string())?;
        Ok(Upsert::Inserted(self.conn.last_insert_rowid()))
    }

    pub fn set_user_meta(&self, id: i64, star: Option<bool>, caption: Option<&str>, rotation: Option<i32>) {
        if let Some(star) = star {
            self.set_star(id, star);
        }
        if let Some(caption) = caption {
            self.set_caption(id, caption);
        }
        if let Some(rotation) = rotation {
            let _ = self.conn.execute(
                "UPDATE photos SET rotation=?1 WHERE id=?2",
                params![rotation, id],
            );
        }
    }

    pub fn remove_missing(&self, folder_id: i64, seen: &[String]) -> Vec<i64> {
        let tx = match self.conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(_) => return Vec::new(),
        };
        let _ = tx.execute("CREATE TEMP TABLE IF NOT EXISTS seen_paths(path TEXT PRIMARY KEY)", []);
        let _ = tx.execute("DELETE FROM seen_paths", []);
        for path in seen {
            let _ = tx.execute("INSERT OR IGNORE INTO seen_paths(path) VALUES(?1)", [path]);
        }
        let removed = {
            let mut stmt = match tx.prepare(
                "SELECT id FROM photos WHERE folder_id=?1 AND path NOT IN (SELECT path FROM seen_paths)",
            ) {
                Ok(stmt) => stmt,
                Err(_) => return Vec::new(),
            };
            stmt.query_map([folder_id], |row| row.get(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<i64>>())
                .unwrap_or_default()
        };
        let _ = tx.execute(
            "DELETE FROM photos WHERE folder_id=?1 AND path NOT IN (SELECT path FROM seen_paths)",
            [folder_id],
        );
        let _ = tx.commit();
        self.delete_empty_people();
        removed
    }

    pub fn photos(&self, nav: &Nav, search: &str, sort: Sort) -> Vec<Photo> {
        let search = search.trim();
        if !search.is_empty() {
            return self.search(search, sort);
        }
        let order = order_sql(sort);
        match nav {
            Nav::Library | Nav::People => {
                let sql = format!("SELECT {PHOTO_COLS} FROM photos p ORDER BY {order}");
                self.map_rows(&sql, [], read_photo)
            }
            Nav::Starred => {
                let sql = format!("SELECT {PHOTO_COLS} FROM photos p WHERE p.starred=1 ORDER BY {order}");
                self.map_rows(&sql, [], read_photo)
            }
            Nav::Folder(id) => {
                let sql = format!(
                    "SELECT {PHOTO_COLS} FROM photos p WHERE p.folder_id=?1 ORDER BY {order}"
                );
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
            Nav::Person(id) => {
                let sql = format!(
                    "SELECT {PHOTO_COLS} FROM photos p
                     WHERE p.id IN (SELECT photo_id FROM faces WHERE person_id=?1)
                     ORDER BY {order}"
                );
                self.map_rows(&sql, [*id], read_photo)
            }
        }
    }

    fn search(&self, query: &str, sort: Sort) -> Vec<Photo> {
        let pattern = like_pattern(query);
        let order = order_sql(sort);
        let sql = format!(
            "SELECT DISTINCT {PHOTO_COLS}
             FROM photos p
             LEFT JOIN faces f ON f.photo_id = p.id
             LEFT JOIN people pe ON pe.id = f.person_id
             WHERE p.filename LIKE ?1 ESCAPE '\\'
                OR p.caption LIKE ?1 ESCAPE '\\'
                OR p.camera LIKE ?1 ESCAPE '\\'
                OR pe.name LIKE ?1 ESCAPE '\\'
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

    pub fn rotate_photo(&self, id: i64, delta_degrees: i32) -> Vec<i64> {
        let Some(photo) = self.photo(id) else {
            return Vec::new();
        };
        let turns = delta_degrees.div_euclid(90).rem_euclid(4);
        if turns == 0 {
            return Vec::new();
        }
        let rotation = (photo.rotation + turns * 90).rem_euclid(360);
        let _ = self.conn.execute(
            "UPDATE photos SET rotation=?1 WHERE id=?2",
            params![rotation, id],
        );
        let faces = self.faces_for_photo(id);
        for face in &faces {
            let rect = match turns {
                3 => rotate_ccw(face.rect()),
                _ => {
                    let mut rect = face.rect();
                    for _ in 0..turns {
                        rect = rotate_cw(rect);
                    }
                    rect
                }
            };
            let rect = clamp_rect(rect);
            let _ = self.conn.execute(
                "UPDATE faces SET x=?1, y=?2, w=?3, h=?4 WHERE id=?5",
                params![rect.x, rect.y, rect.w, rect.h, face.id],
            );
        }
        faces.into_iter().map(|face| face.id).collect()
    }

    pub fn delete_photos(&self, ids: &[i64]) {
        for id in ids {
            let _ = self.conn.execute("DELETE FROM photos WHERE id=?1", [id]);
        }
        self.delete_empty_people();
    }

    pub fn clear_faces(&self, photo_id: i64) -> Vec<i64> {
        let ids = self
            .conn
            .prepare("SELECT id FROM faces WHERE photo_id=?1")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([photo_id], |row| row.get(0))
                    .ok()
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();
        let _ = self
            .conn
            .execute("DELETE FROM faces WHERE photo_id=?1", [photo_id]);
        self.delete_empty_people();
        ids
    }

    pub fn set_faces_scanned(&self, id: i64, scanned: bool) {
        let _ = self.conn.execute(
            "UPDATE photos SET faces_scanned=?1 WHERE id=?2",
            params![if scanned { 1 } else { 0 }, id],
        );
    }

    pub fn has_manual_faces(&self, photo_id: i64) -> bool {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM faces WHERE photo_id=?1 AND manual=1)",
                [photo_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|v| v == 1)
            .unwrap_or(false)
    }

    pub fn face_progress(&self) -> (i64, i64) {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(faces_scanned), 0), COUNT(*) FROM photos",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((0, 0))
    }

    pub fn faces_for_photo(&self, photo_id: i64) -> Vec<Face> {
        self.map_rows(
            "SELECT f.id, f.photo_id, f.person_id, pe.name, f.x, f.y, f.w, f.h, f.score,
                    f.confirmed, f.manual
             FROM faces f
             JOIN people pe ON pe.id = f.person_id
             WHERE f.photo_id=?1
             ORDER BY f.score DESC",
            [photo_id],
            read_face,
        )
    }

    pub fn embedding(&self, face_id: i64) -> Option<Vec<f32>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT embedding FROM faces WHERE id=?1", [face_id], |row| {
                row.get(0)
            })
            .optional()
            .ok()
            .flatten()
            .flatten();
        blob.map(|bytes| blobs_to_vec(&bytes))
            .filter(|v| v.len() == EMBEDDING_LEN)
    }

    pub fn people(&self) -> Vec<Person> {
        self.map_rows(
            "SELECT pe.id, pe.name,
                    COUNT(DISTINCT f.photo_id),
                    COUNT(f.id),
                    cf.id, p.path, p.rotation, cf.x, cf.y, cf.w, cf.h
             FROM people pe
             JOIN faces f ON f.person_id = pe.id
             JOIN faces cf ON cf.id = (
                SELECT f2.id FROM faces f2 WHERE f2.person_id = pe.id
                ORDER BY f2.score DESC, f2.id LIMIT 1
             )
             JOIN photos p ON p.id = cf.photo_id
             GROUP BY pe.id
             ORDER BY pe.name IS NULL, pe.name COLLATE NOCASE, COUNT(DISTINCT f.photo_id) DESC",
            [],
            |row| {
                Ok(Person {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    photo_count: row.get(2)?,
                    face_count: row.get(3)?,
                    cover_face_id: row.get(4)?,
                    cover_path: row.get(5)?,
                    cover_rotation: row.get::<_, i64>(6)? as i32,
                    cover: RelRect {
                        x: row.get::<_, f64>(7)? as f32,
                        y: row.get::<_, f64>(8)? as f32,
                        w: row.get::<_, f64>(9)? as f32,
                        h: row.get::<_, f64>(10)? as f32,
                    },
                })
            },
        )
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

    pub fn person_for_contact(&self, contact: &str, name: Option<&str>) -> i64 {
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        if unnamed_contact(contact) {
            return self.create_person(name, None);
        }
        let external = format!("picasa:{contact}");
        if let Some(id) = self.person_by_external(&external) {
            if let Some(name) = name {
                let _ = self.conn.execute(
                    "UPDATE people SET name=?1 WHERE id=?2 AND (name IS NULL OR name='')",
                    params![name, id],
                );
            }
            return id;
        }
        self.create_person(name, Some(&external))
    }

    fn person_by_external(&self, external: &str) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT id FROM people WHERE external_id=?1",
                [external],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn person_by_name(&self, name: &str) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT id FROM people WHERE name=?1 COLLATE NOCASE LIMIT 1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn create_person(&self, name: Option<&str>, external: Option<&str>) -> i64 {
        let _ = self.conn.execute(
            "INSERT INTO people(name, external_id, created_at) VALUES(?1, ?2, ?3)",
            params![name, external, unix_now()],
        );
        self.conn.last_insert_rowid()
    }

    pub fn rename_person(&self, id: i64, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            let _ = self
                .conn
                .execute("UPDATE people SET name=NULL WHERE id=?1", [id]);
        } else {
            let _ = self
                .conn
                .execute("UPDATE people SET name=?1 WHERE id=?2", params![name, id]);
        }
    }

    pub fn insert_face(&self, photo_id: i64, face: &NewFace) -> i64 {
        let blob = if face.embedding.len() == EMBEDDING_LEN {
            Some(vec_to_blob(&face.embedding))
        } else {
            None
        };
        let _ = self.conn.execute(
            "INSERT INTO faces(photo_id, person_id, x, y, w, h, score, embedding, confirmed, manual)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                photo_id,
                face.person_id,
                face.rect.x,
                face.rect.y,
                face.rect.w,
                face.rect.h,
                face.score,
                blob,
                if face.confirmed { 1 } else { 0 },
                if face.manual { 1 } else { 0 },
            ],
        );
        let id = self.conn.last_insert_rowid();
        self.recompute_centroid(face.person_id);
        id
    }

    pub fn set_embedding(&self, face_id: i64, embedding: &[f32]) {
        if embedding.len() != EMBEDDING_LEN {
            return;
        }
        let _ = self.conn.execute(
            "UPDATE faces SET embedding=?1 WHERE id=?2",
            params![vec_to_blob(embedding), face_id],
        );
    }

    /// Attach `embedding` to the nearest person, or open a new unnamed person.
    pub fn assign_person(&self, embedding: &[f32]) -> i64 {
        let mut best: Option<(i64, f32)> = None;
        for centroid in self.centroids() {
            let score = cosine(embedding, &centroid.embedding);
            if best.map(|(_, prev)| score > prev).unwrap_or(true) {
                best = Some((centroid.person_id, score));
            }
        }
        if let Some((id, score)) = best {
            if score >= crate::faces::assign_threshold() {
                return id;
            }
        }
        let id = self.create_person(None, None);
        let _ = self.conn.execute(
            "UPDATE people SET centroid=?1 WHERE id=?2",
            params![vec_to_blob(embedding), id],
        );
        id
    }

    pub fn centroids(&self) -> Vec<Centroid> {
        self.map_rows(
            "SELECT id, name, centroid FROM people WHERE centroid IS NOT NULL",
            [],
            |row| {
                let blob: Vec<u8> = row.get(2)?;
                Ok(Centroid {
                    person_id: row.get(0)?,
                    name: row.get(1)?,
                    embedding: blobs_to_vec(&blob),
                })
            },
        )
        .into_iter()
        .filter(|c| c.embedding.len() == EMBEDDING_LEN)
        .collect()
    }

    pub fn recompute_centroid(&self, person_id: i64) {
        let confirmed = self.embeddings_for(person_id, true);
        let rows = if confirmed.is_empty() {
            self.embeddings_for(person_id, false)
        } else {
            confirmed
        };
        if rows.is_empty() {
            let _ = self
                .conn
                .execute("UPDATE people SET centroid=NULL WHERE id=?1", [person_id]);
            return;
        }
        let avg = average_embeddings(&rows);
        let _ = self.conn.execute(
            "UPDATE people SET centroid=?1 WHERE id=?2",
            params![vec_to_blob(&avg), person_id],
        );
    }

    fn embeddings_for(&self, person_id: i64, confirmed_only: bool) -> Vec<Vec<f32>> {
        let sql = if confirmed_only {
            "SELECT embedding FROM faces WHERE person_id=?1 AND confirmed=1 AND embedding IS NOT NULL LIMIT 24"
        } else {
            "SELECT embedding FROM faces WHERE person_id=?1 AND embedding IS NOT NULL LIMIT 24"
        };
        self.map_rows(sql, [person_id], |row| row.get::<_, Vec<u8>>(0))
            .into_iter()
            .map(|blob| blobs_to_vec(&blob))
            .filter(|v| v.len() == EMBEDDING_LEN)
            .collect()
    }

    pub fn move_face(&self, face_id: i64, person_id: i64) {
        let old: Option<i64> = self
            .conn
            .query_row("SELECT person_id FROM faces WHERE id=?1", [face_id], |row| {
                row.get(0)
            })
            .optional()
            .ok()
            .flatten();
        let _ = self.conn.execute(
            "UPDATE faces SET person_id=?1, confirmed=1, manual=1 WHERE id=?2",
            params![person_id, face_id],
        );
        self.recompute_centroid(person_id);
        if let Some(old) = old {
            if old != person_id {
                self.recompute_centroid(old);
            }
        }
        self.delete_empty_people();
    }

    pub fn confirm_face(&self, face_id: i64) {
        let _ = self.conn.execute(
            "UPDATE faces SET confirmed=1, manual=1 WHERE id=?1",
            [face_id],
        );
        if let Some(person_id) = self
            .conn
            .query_row("SELECT person_id FROM faces WHERE id=?1", [face_id], |row| {
                row.get::<_, i64>(0)
            })
            .ok()
        {
            self.recompute_centroid(person_id);
        }
    }

    pub fn split_face(&self, face_id: i64) -> i64 {
        let new_id = self.create_person(None, None);
        self.move_face(face_id, new_id);
        new_id
    }

    /// Pull unnamed, unconfirmed faces into a person the user just named.
    pub fn rematch(&self, person_id: i64) -> usize {
        let Some(centroid) = self
            .centroids()
            .into_iter()
            .find(|c| c.person_id == person_id)
        else {
            return 0;
        };
        let candidates = self.map_rows(
            "SELECT f.id, f.embedding, f.person_id
             FROM faces f
             JOIN people pe ON pe.id = f.person_id
             WHERE pe.name IS NULL AND f.confirmed=0 AND f.person_id!=?1 AND f.embedding IS NOT NULL",
            [person_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, i64>(2)?)),
        );
        let mut moved = 0;
        let mut touched = Vec::new();
        for (face_id, blob, from) in candidates {
            let emb = blobs_to_vec(&blob);
            if cosine(&emb, &centroid.embedding) >= 0.90 {
                let _ = self.conn.execute(
                    "UPDATE faces SET person_id=?1 WHERE id=?2",
                    params![person_id, face_id],
                );
                touched.push(from);
                moved += 1;
            }
        }
        if moved > 0 {
            self.recompute_centroid(person_id);
            touched.sort_unstable();
            touched.dedup();
            for id in touched {
                self.recompute_centroid(id);
            }
            self.delete_empty_people();
        }
        moved
    }

    pub fn reset_detection(&self, photo_ids: &[i64]) {
        for id in photo_ids {
            let _ = self.conn.execute(
                "DELETE FROM faces WHERE photo_id=?1 AND manual=0 AND confirmed=0",
                [id],
            );
            let _ = self.conn.execute(
                "UPDATE photos SET faces_scanned=0 WHERE id=?1",
                [id],
            );
        }
        self.delete_empty_people();
    }

    fn delete_empty_people(&self) {
        let _ = self.conn.execute(
            "DELETE FROM people WHERE id NOT IN (SELECT DISTINCT person_id FROM faces)",
            [],
        );
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
        rotation: row.get::<_, i64>(11)? as i32,
        camera: row.get(12)?,
        faces_scanned: row.get::<_, i64>(13)? != 0,
        face_count: row.get(14)?,
    })
}

fn read_face(row: &rusqlite::Row<'_>) -> rusqlite::Result<Face> {
    Ok(Face {
        id: row.get(0)?,
        photo_id: row.get(1)?,
        person_id: row.get(2)?,
        person_name: row.get(3)?,
        x: row.get::<_, f64>(4)? as f32,
        y: row.get::<_, f64>(5)? as f32,
        w: row.get::<_, f64>(6)? as f32,
        h: row.get::<_, f64>(7)? as f32,
        score: row.get::<_, f64>(8)? as f32,
        confirmed: row.get::<_, i64>(9)? != 0,
        manual: row.get::<_, i64>(10)? != 0,
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

    fn temp_db() -> Database {
        let dir = std::env::temp_dir().join(format!("picasso-db-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Database::open(&dir.join("t.db")).unwrap()
    }

    #[test]
    fn catalog_roundtrip() {
        let db = temp_db();
        let dir = std::env::temp_dir().join(format!("picasso-db-{}", std::process::id()));
        let folder = dir.join("pics");
        std::fs::create_dir_all(&folder).unwrap();
        let id = db.add_folder(&folder).unwrap();
        assert_eq!(db.add_folder(&folder).unwrap(), id);
        let inserted = db
            .upsert_photo(&NewPhoto {
                folder_id: id,
                path: folder.join("a.jpg").to_string_lossy().into_owned(),
                filename: "a.jpg".into(),
                width: 100,
                height: 80,
                bytes: 10,
                taken_at: 1_700_000_000,
                modified_at: 10,
                camera: "TestCam".into(),
            })
            .unwrap();
        let photo_id = match inserted {
            Upsert::Inserted(id) => id,
            _ => panic!("expected insert"),
        };
        db.set_star(photo_id, true);
        db.set_caption(photo_id, "hello");
        let person = db.create_person(Some("Ada"), None);
        db.insert_face(
            photo_id,
            &NewFace {
                person_id: person,
                rect: RelRect {
                    x: 0.1,
                    y: 0.2,
                    w: 0.3,
                    h: 0.3,
                },
                score: 4.0,
                embedding: vec![0.0; EMBEDDING_LEN],
                confirmed: true,
                manual: true,
            },
        );
        let album = db.create_album("Trip").unwrap();
        db.add_to_album(album, &[photo_id]);
        assert_eq!(db.photos(&Nav::Starred, "", Sort::Name).len(), 1);
        assert_eq!(db.photos(&Nav::Album(album), "", Sort::Name).len(), 1);
        assert_eq!(db.photos(&Nav::Person(person), "", Sort::Name).len(), 1);
        assert_eq!(db.photos(&Nav::Library, "ada", Sort::Name).len(), 1);
        assert_eq!(db.faces_for_photo(photo_id).len(), 1);
        db.rotate_photo(photo_id, 90);
        let face = &db.faces_for_photo(photo_id)[0];
        assert!(face.x > 0.4);
        db.delete_photos(&[photo_id]);
        assert!(db.people().is_empty());
    }
}
