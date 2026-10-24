//! Disk thumbnails: one JPEG per photo in `thumbs.db`, a SQLite file in the
//! cache folder. It is kept apart from the library catalog, so the catalog
//! stays small and the cache can be deleted at any time; missing thumbnails
//! are made again.
//!
//! Older builds wrote `thumbs/<id>.jpg` files instead. Those move into the
//! database on first use and in bulk by [`migrate_files`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

/// Rows moved per transaction by [`migrate_files`].
const MIGRATE_BATCH: usize = 500;

/// One connection to `thumbs.db`. Each thread opens its own.
pub struct ThumbStore {
    conn: Connection,
    /// Folder of the old one-file-per-photo cache.
    files: PathBuf,
}

impl ThumbStore {
    /// Opens or creates `thumbs.db` in `cache`. When the file can't be used,
    /// thumbnails are kept in memory for this connection only, so the app
    /// still works, just slower on the next launch.
    pub fn open(cache: &Path) -> ThumbStore {
        let files = cache.join("thumbs");
        let conn = open_file(&cache.join("thumbs.db")).unwrap_or_else(|err| {
            eprintln!("picasso thumbs.db: {err}");
            let conn = Connection::open_in_memory().expect("in-memory thumbnail store");
            let _ = conn.execute_batch(SCHEMA);
            conn
        });
        ThumbStore { conn, files }
    }

    /// JPEG bytes of the photo's thumbnail. One still in an old file is moved
    /// into the database first.
    pub fn get(&self, id: i64) -> Option<Vec<u8>> {
        let stored = self
            .conn
            .query_row("SELECT jpeg FROM thumbs WHERE id=?1", [id], |row| row.get(0))
            .optional()
            .ok()
            .flatten();
        stored.or_else(|| self.adopt_file(id))
    }

    pub fn contains(&self, id: i64) -> bool {
        let stored = self
            .conn
            .query_row("SELECT 1 FROM thumbs WHERE id=?1", [id], |_| Ok(()))
            .optional()
            .ok()
            .flatten()
            .is_some();
        stored || self.file(id).exists()
    }

    pub fn put(&self, id: i64, jpeg: &[u8]) {
        if let Err(err) = self.conn.execute("INSERT OR REPLACE INTO thumbs(id, jpeg) VALUES(?1, ?2)", params![id, jpeg]) {
            eprintln!("picasso thumbs.db: {err}");
        }
    }

    pub fn remove(&self, id: i64) {
        let _ = self.conn.execute("DELETE FROM thumbs WHERE id=?1", [id]);
        let _ = self.conn.execute("DELETE FROM edited WHERE id=?1", [id]);
        let _ = std::fs::remove_file(self.file(id));
    }

    /// JPEG bytes of the photo's edited thumbnail, if it was made for `edit`.
    pub fn get_edited(&self, id: i64, edit: &str) -> Option<Vec<u8>> {
        self.conn
            .query_row("SELECT jpeg FROM edited WHERE id=?1 AND edit=?2", params![id, edit], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    /// Stores the thumbnail of the photo with `edit` applied, replacing the
    /// one made for an earlier edit.
    pub fn put_edited(&self, id: i64, edit: &str, jpeg: &[u8]) {
        if let Err(err) = self
            .conn
            .execute("INSERT OR REPLACE INTO edited(id, edit, jpeg) VALUES(?1, ?2, ?3)", params![id, edit, jpeg])
        {
            eprintln!("picasso thumbs.db: {err}");
        }
    }

    /// Drops the edited thumbnail of a photo that is no longer edited.
    pub fn remove_edited(&self, id: i64) {
        let stored = self
            .conn
            .query_row("SELECT 1 FROM edited WHERE id=?1", [id], |_| Ok(()))
            .optional()
            .ok()
            .flatten()
            .is_some();
        if stored {
            let _ = self.conn.execute("DELETE FROM edited WHERE id=?1", [id]);
        }
    }

    /// Photos that have a thumbnail, in the database or in an old file.
    pub fn ids(&self) -> HashSet<i64> {
        let mut ids: HashSet<i64> = self
            .conn
            .prepare("SELECT id FROM thumbs")
            .and_then(|mut stmt| stmt.query_map([], |row| row.get(0))?.collect())
            .unwrap_or_default();
        ids.extend(file_ids(&self.files));
        ids
    }

    /// Deletes the thumbnails of photos no longer in the library.
    pub fn retain(&self, photos: &HashSet<i64>) {
        let mut ids = self.ids();
        ids.extend(
            self.conn
                .prepare("SELECT id FROM edited")
                .and_then(|mut stmt| stmt.query_map([], |row| row.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>())
                .unwrap_or_default(),
        );
        let stale: Vec<i64> = ids.into_iter().filter(|id| !photos.contains(id)).collect();
        if stale.is_empty() {
            return;
        }
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return;
        };
        for id in &stale {
            let _ = tx.execute("DELETE FROM thumbs WHERE id=?1", [id]);
            let _ = tx.execute("DELETE FROM edited WHERE id=?1", [id]);
        }
        let _ = tx.commit();
        for id in stale {
            let _ = std::fs::remove_file(self.file(id));
        }
    }

    /// Deletes every thumbnail and shrinks the file, so the space goes back
    /// to the disk.
    pub fn clear(&self) {
        if let Err(err) = self
            .conn
            .execute_batch("DELETE FROM thumbs; DELETE FROM edited; VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        {
            eprintln!("picasso thumbs.db: {err}");
        }
        let _ = std::fs::remove_dir_all(&self.files);
    }

    fn file(&self, id: i64) -> PathBuf {
        self.files.join(format!("{id}.jpg"))
    }

    fn adopt_file(&self, id: i64) -> Option<Vec<u8>> {
        let path = self.file(id);
        let jpeg = std::fs::read(&path).ok()?;
        self.put(id, &jpeg);
        let _ = std::fs::remove_file(path);
        Some(jpeg)
    }
}

/// `edited` holds one thumbnail per edited photo, with the edit it shows,
/// so a different edit misses it.
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS thumbs (id INTEGER PRIMARY KEY, jpeg BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS edited (id INTEGER PRIMARY KEY, edit TEXT NOT NULL, jpeg BLOB NOT NULL);";

/// Size of the thumbnails in the store. Ones from an older version are
/// deleted on open and made again as they are needed: version 1 was 256
/// pixels on the long edge, too small for tiles on high-density screens.
const VERSION: i32 = 2;

fn open_file(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let conn = Connection::open(path).map_err(|err| err.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(8)).map_err(|err| err.to_string())?;
    // A thumbnail is about 30 KB, so larger pages keep each in fewer of them.
    // The page size only takes effect while the file is still empty.
    conn.execute_batch(&format!(
        "PRAGMA page_size=16384;
         PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         {SCHEMA}"
    ))
    .map_err(|err| err.to_string())?;
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).map_err(|err| err.to_string())?;
    if version < VERSION {
        conn.execute_batch(&format!("DELETE FROM thumbs; DELETE FROM edited; PRAGMA user_version={VERSION};"))
            .map_err(|err| err.to_string())?;
    }
    Ok(conn)
}

fn file_ids(dir: &Path) -> impl Iterator<Item = i64> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.strip_suffix(".jpg")?.parse().ok())
}

/// Moves every thumbnail an older build left in `cache/thumbs` into
/// `thumbs.db`, then deletes the folder.
pub fn migrate_files(cache: &Path) {
    let store = ThumbStore::open(cache);
    if !store.files.is_dir() {
        return;
    }
    let ids: Vec<i64> = file_ids(&store.files).collect();
    for chunk in ids.chunks(MIGRATE_BATCH) {
        let Ok(tx) = store.conn.unchecked_transaction() else {
            return;
        };
        let mut moved = Vec::with_capacity(chunk.len());
        for &id in chunk {
            let Ok(jpeg) = std::fs::read(store.file(id)) else {
                continue;
            };
            if tx.execute("INSERT OR IGNORE INTO thumbs(id, jpeg) VALUES(?1, ?2)", params![id, jpeg]).is_ok() {
                moved.push(id);
            }
        }
        if tx.commit().is_err() {
            return;
        }
        for id in moved {
            let _ = std::fs::remove_file(store.file(id));
        }
    }
    // Only leftovers such as interrupted `.tmp` writes remain.
    if file_ids(&store.files).next().is_none() {
        let _ = std::fs::remove_dir_all(&store.files);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso-thumbs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn stores_replaces_and_removes_thumbnails() {
        let cache = temp_cache("store");
        let store = ThumbStore::open(&cache);
        assert_eq!(store.get(7), None);
        store.put(7, b"first");
        store.put(7, b"second");
        assert_eq!(store.get(7).as_deref(), Some(&b"second"[..]));
        assert!(store.contains(7));
        store.put(8, b"other");
        store.retain(&HashSet::from([8]));
        assert!(!store.contains(7));
        assert_eq!(store.ids(), HashSet::from([8]));
        store.remove(8);
        assert!(store.ids().is_empty());
        store.put(9, &[7; 20_000]);
        store.clear();
        assert!(store.ids().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn drops_thumbnails_of_an_older_size() {
        let cache = temp_cache("version");
        let store = ThumbStore::open(&cache);
        store.put(1, b"small");
        store.put_edited(1, "exposure 1", b"small edited");
        drop(store);
        assert_eq!(ThumbStore::open(&cache).get(1).as_deref(), Some(&b"small"[..]), "kept while the version matches");
        Connection::open(cache.join("thumbs.db")).unwrap().execute_batch("PRAGMA user_version=1;").unwrap();
        let store = ThumbStore::open(&cache);
        assert_eq!(store.get(1), None);
        assert_eq!(store.get_edited(1, "exposure 1"), None);
        drop(store);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn edited_thumbnails_follow_the_edit() {
        let cache = temp_cache("edited");
        let store = ThumbStore::open(&cache);
        store.put(5, b"plain");
        store.put_edited(5, "exposure 1", b"brighter");
        assert_eq!(store.get_edited(5, "exposure 1").as_deref(), Some(&b"brighter"[..]));
        assert_eq!(store.get_edited(5, "exposure 2"), None, "another edit misses");
        store.put_edited(5, "exposure 2", b"brightest");
        assert_eq!(store.get_edited(5, "exposure 1"), None, "replaced by the newer edit");
        store.remove_edited(5);
        assert_eq!(store.get_edited(5, "exposure 2"), None);
        assert_eq!(store.get(5).as_deref(), Some(&b"plain"[..]), "the plain thumbnail stays");
        store.put_edited(5, "exposure 1", b"brighter");
        store.put_edited(6, "exposure 1", b"other");
        store.remove(5);
        assert_eq!(store.get_edited(5, "exposure 1"), None);
        store.retain(&HashSet::new());
        assert_eq!(store.get_edited(6, "exposure 1"), None);
        drop(store);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn moves_old_thumbnail_files_into_the_database() {
        let cache = temp_cache("migrate");
        let files = cache.join("thumbs");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(cache.join("tiles")).unwrap();
        let count = MIGRATE_BATCH as i64 + 3;
        for id in 1..=count {
            std::fs::write(files.join(format!("{id}.jpg")), format!("jpeg {id}")).unwrap();
        }
        std::fs::write(files.join("9.jpg.1-0.tmp"), "partial").unwrap();

        let store = ThumbStore::open(&cache);
        assert_eq!(store.ids().len(), count as usize);
        assert_eq!(store.get(2).as_deref(), Some(&b"jpeg 2"[..]));
        assert!(!files.join("2.jpg").exists());

        migrate_files(&cache);
        assert!(!files.exists());
        assert!(cache.join("thumbs.db").is_file());
        assert!(cache.join("tiles").is_dir());
        assert_eq!(store.get(1).as_deref(), Some(&b"jpeg 1"[..]));
        assert_eq!(store.ids(), (1..=count).collect());
        drop(store);
        let _ = std::fs::remove_dir_all(&cache);
    }
}
