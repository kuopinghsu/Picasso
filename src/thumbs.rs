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
        let _ = std::fs::remove_file(self.file(id));
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
        let stale: Vec<i64> = self.ids().into_iter().filter(|id| !photos.contains(id)).collect();
        if stale.is_empty() {
            return;
        }
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return;
        };
        for id in &stale {
            let _ = tx.execute("DELETE FROM thumbs WHERE id=?1", [id]);
        }
        let _ = tx.commit();
        for id in stale {
            let _ = std::fs::remove_file(self.file(id));
        }
    }

    /// Deletes every thumbnail and shrinks the file, so the space goes back
    /// to the disk.
    pub fn clear(&self) {
        if let Err(err) = self.conn.execute_batch("DELETE FROM thumbs; VACUUM; PRAGMA wal_checkpoint(TRUNCATE);") {
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

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS thumbs (id INTEGER PRIMARY KEY, jpeg BLOB NOT NULL);";

fn open_file(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let conn = Connection::open(path).map_err(|err| err.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(8)).map_err(|err| err.to_string())?;
    // A thumbnail is about 13 KB, so larger pages keep each in fewer of them.
    // The page size only takes effect while the file is still empty.
    conn.execute_batch(&format!(
        "PRAGMA page_size=16384;
         PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         {SCHEMA}"
    ))
    .map_err(|err| err.to_string())?;
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
