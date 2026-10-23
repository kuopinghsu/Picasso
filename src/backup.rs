//! Copies of the library database in `backups` next to it. Stars,
//! captions, albums, and edits live only in the library, so Picasso copies it
//! once a week and keeps the last few. A restore is staged as `restore.db`
//! and swapped in the next time Picasso starts, before the library opens.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{Local, NaiveDateTime, TimeZone};
use rusqlite::{Connection, OpenFlags};

const LIBRARY: &str = "picasso.db";
const STAGED: &str = "restore.db";
const PREFIX: &str = "picasso-";
const STAMP: &str = "%Y%m%d-%H%M%S";
/// Backups kept; older ones are deleted after each new one.
pub const KEEP: usize = 5;
/// Age of the newest backup that makes a new one due at startup.
pub const EVERY: Duration = Duration::from_secs(7 * 86_400);

pub struct Backup {
    pub path: PathBuf,
    /// Unix seconds when it was made.
    pub made_at: i64,
    pub bytes: u64,
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("backups")
}

/// Backups in `data_dir`, newest first.
pub fn list(data_dir: &Path) -> Vec<Backup> {
    let Ok(entries) = std::fs::read_dir(dir(data_dir)) else {
        return Vec::new();
    };
    let mut backups: Vec<Backup> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stamp = name.strip_prefix(PREFIX)?.strip_suffix(".db")?;
            let made = NaiveDateTime::parse_from_str(stamp, STAMP).ok()?;
            Some(Backup {
                path: entry.path(),
                made_at: Local.from_local_datetime(&made).earliest()?.timestamp(),
                bytes: entry.metadata().map_or(0, |meta| meta.len()),
            })
        })
        .collect();
    backups.sort_by_key(|backup| std::cmp::Reverse(backup.made_at));
    backups
}

/// True when the newest backup is older than [`EVERY`], or there is none.
pub fn due(data_dir: &Path) -> bool {
    let newest = list(data_dir).first().map(|backup| backup.made_at);
    newest.is_none_or(|made_at| crate::types::unix_now() - made_at >= EVERY.as_secs() as i64)
}

/// Copies the library into `backups` and deletes all but the newest
/// [`KEEP`]. Works next to a running Picasso: the copy is one consistent
/// read through its own read-only connection.
pub fn create(data_dir: &Path) -> Result<PathBuf, String> {
    let folder = dir(data_dir);
    std::fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
    let mut name = format!("{PREFIX}{}.db", Local::now().format(STAMP));
    if folder.join(&name).exists() {
        name = format!("{PREFIX}{}.db", (Local::now() + chrono::Duration::seconds(1)).format(STAMP));
    }
    let target = folder.join(name);
    let partial = target.with_extension("partial");
    let _ = std::fs::remove_file(&partial);
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(data_dir.join(LIBRARY), flags).map_err(|err| err.to_string())?;
    conn.busy_timeout(Duration::from_secs(30)).map_err(|err| err.to_string())?;
    conn.execute("VACUUM INTO ?1", [partial.to_string_lossy()]).map_err(|err| err.to_string())?;
    drop(conn);
    std::fs::rename(&partial, &target).map_err(|err| err.to_string())?;
    for old in list(data_dir).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old.path);
    }
    Ok(target)
}

/// Number of photos in a backup, or why it can't be used.
pub fn check(path: &Path) -> Result<i64, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|err| err.to_string())?;
    let ok: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0)).map_err(|err| err.to_string())?;
    if ok != "ok" {
        return Err(ok);
    }
    conn.query_row("SELECT count(*) FROM photos", [], |row| row.get(0))
        .map_err(|_| crate::i18n::tr("This isn't a Picasso library").to_string())
}

/// Stages `backup` to replace the library the next time Picasso starts.
pub fn stage_restore(data_dir: &Path, backup: &Path) -> Result<(), String> {
    check(backup)?;
    let staged = data_dir.join(STAGED);
    let partial = staged.with_extension("partial");
    std::fs::copy(backup, &partial).map_err(|err| err.to_string())?;
    std::fs::rename(&partial, &staged).map_err(|err| err.to_string())
}

/// Swaps in a staged backup, after backing up the library it replaces.
/// Runs before the library opens. `None` when nothing was staged.
pub fn apply_staged(data_dir: &Path) -> Option<Result<(), String>> {
    let staged = data_dir.join(STAGED);
    if !staged.exists() {
        return None;
    }
    Some(swap_in(data_dir, &staged))
}

fn swap_in(data_dir: &Path, staged: &Path) -> Result<(), String> {
    let library = data_dir.join(LIBRARY);
    if library.exists() {
        // Fails while another Picasso window holds the library open; the
        // staged copy waits for the next start.
        create(data_dir)?;
        let aside = data_dir.join("picasso.db.replaced");
        let _ = std::fs::remove_file(&aside);
        // After Restore the window that asked for it may still be closing.
        let mut tries = 0;
        while let Err(err) = std::fs::rename(&library, &aside) {
            tries += 1;
            if tries == 40 {
                return Err(err.to_string());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(data_dir.join(format!("{LIBRARY}{suffix}")));
        }
        if let Err(err) = std::fs::rename(staged, &library) {
            let _ = std::fs::rename(&aside, &library);
            return Err(err.to_string());
        }
        let _ = std::fs::remove_file(&aside);
        return Ok(());
    }
    std::fs::rename(staged, &library).map_err(|err| err.to_string())
}

/// Starts Picasso again and returns; the caller then closes this window.
pub fn relaunch() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    std::process::Command::new(exe).spawn().map(|_| ()).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso-backup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Database::open(&dir.join(LIBRARY)).unwrap();
        db.create_album("Trip").unwrap();
        dir
    }

    fn albums(dir: &Path) -> i64 {
        let conn = Connection::open(dir.join(LIBRARY)).unwrap();
        conn.query_row("SELECT count(*) FROM albums", [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn backups_are_listed_newest_first_and_only_the_last_few_kept() {
        let dir = library("keep");
        assert!(due(&dir));
        let folder = super::dir(&dir);
        std::fs::create_dir_all(&folder).unwrap();
        for day in 1..=7 {
            std::fs::write(folder.join(format!("{PREFIX}2020010{day}-120000.db")), b"old").unwrap();
        }
        std::fs::write(folder.join("notes.txt"), b"not a backup").unwrap();
        assert!(due(&dir), "a week-old backup makes a new one due");
        let made = create(&dir).unwrap();
        let backups = list(&dir);
        assert_eq!(backups.len(), KEEP);
        assert_eq!(backups[0].path, made);
        assert!(backups[1].path.ends_with(format!("{PREFIX}20200107-120000.db")));
        assert!(!due(&dir));
        assert_eq!(check(&made).unwrap(), 0);
        assert!(check(&folder.join(format!("{PREFIX}20200107-120000.db"))).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_staged_backup_replaces_the_library_at_the_next_start() {
        let dir = library("restore");
        let backup = create(&dir).unwrap();
        crate::db::Database::open(&dir.join(LIBRARY)).unwrap().create_album("Later").unwrap();
        assert_eq!(albums(&dir), 2);
        assert!(apply_staged(&dir).is_none());
        stage_restore(&dir, &backup).unwrap();
        apply_staged(&dir).unwrap().unwrap();
        assert_eq!(albums(&dir), 1, "the backup's library is back");
        assert!(!dir.join(STAGED).exists());
        // The library it replaced was backed up first.
        let newest = &list(&dir)[0];
        let conn = Connection::open(&newest.path).unwrap();
        let count: i64 = conn.query_row("SELECT count(*) FROM albums", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
