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
    if let Err(err) = conn.execute("VACUUM INTO ?1", [partial.to_string_lossy()]) {
        let _ = std::fs::remove_file(&partial);
        return Err(err.to_string());
    }
    drop(conn);
    std::fs::rename(&partial, &target).map_err(|err| err.to_string())?;
    for old in list(data_dir).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old.path);
    }
    Ok(target)
}

/// Number of photos in a backup or library, or why it can't be used.
pub fn check(path: &Path) -> Result<i64, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|err| err.to_string())?;
    conn.busy_timeout(Duration::from_secs(8)).map_err(|err| err.to_string())?;
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

/// Moves a library that can't be opened aside as
/// `picasso-damaged-YYYYMMDD-HHMMSS.db`, with its `-wal` and `-shm` files, so
/// it is kept for repair tools, then puts `backup` in its place, or nothing,
/// in which case Picasso starts a new library. Returns the moved library.
pub fn replace_damaged(data_dir: &Path, backup: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(backup) = backup {
        check(backup)?;
    }
    let library = data_dir.join(LIBRARY);
    if !library.is_file() {
        return Err(format!("{} doesn't exist", library.display()));
    }
    let aside = data_dir.join(format!("picasso-damaged-{}.db", Local::now().format(STAMP)));
    if aside.exists() {
        return Err(format!("{} already exists", aside.display()));
    }
    move_with_sidecars(&library, &aside).map_err(|err| err.to_string())?;
    if let Some(backup) = backup {
        let partial = library.with_extension("partial");
        let copied = std::fs::copy(backup, &partial).and_then(|_| std::fs::rename(&partial, &library));
        if let Err(err) = copied {
            let _ = std::fs::remove_file(&partial);
            let _ = move_with_sidecars(&aside, &library);
            return Err(err.to_string());
        }
    }
    Ok(aside)
}

/// Renames a database together with its `-wal` and `-shm` files, which hold
/// changes not yet written into it. If one can't be moved, those already
/// moved go back, so a database is never parted from its `-wal`.
fn move_with_sidecars(from: &Path, to: &Path) -> std::io::Result<()> {
    let side = |path: &Path, suffix: &str| PathBuf::from(format!("{}{suffix}", path.display()));
    let moves: Vec<(PathBuf, PathBuf)> = std::iter::once((from.to_path_buf(), to.to_path_buf()))
        .chain(["-wal", "-shm"].into_iter().map(|suffix| (side(from, suffix), side(to, suffix))))
        .filter(|(source, _)| source.exists())
        .collect();
    for (done, (source, target)) in moves.iter().enumerate() {
        if let Err(err) = std::fs::rename(source, target) {
            for (source, target) in moves[..done].iter().rev() {
                let _ = std::fs::rename(target, source);
            }
            return Err(err);
        }
    }
    Ok(())
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

    #[test]
    fn a_restored_backup_brings_back_everything_the_user_set() {
        use crate::db::{Database, ScanRecord};
        use crate::track::{Point, Track};
        use crate::types::{Nav, NewPhoto, Sort, Unreadable};

        let dir = library("everything");
        let photos = dir.join("photos");
        std::fs::create_dir_all(photos.join("skip")).unwrap();
        let db = Database::open(&dir.join(LIBRARY)).unwrap();
        let folder_id = db.add_folder(&photos).unwrap();
        let photos = photos.canonicalize().unwrap();
        let path = |name: &str| photos.join(name).to_string_lossy().into_owned();
        let record = |name: &str| ScanRecord {
            photo: NewPhoto {
                folder_id,
                path: path(name),
                filename: name.into(),
                width: 4000,
                height: 3000,
                bytes: 1234,
                taken_at: 1_700_000_000,
                modified_at: 1_700_000_100,
                camera: "Camera".into(),
                lens: "Lens".into(),
                focal_mm: 35.0,
                gps_lat: Some(25.03),
                gps_lon: Some(121.56),
                duration: 0.0,
            },
            existing: None,
            star: None,
            caption: None,
            rotation: None,
            pin: None,
            complete: true,
        };
        let ids = db.write_batch(&[record("a.jpg"), record("b.jpg"), record("gone.jpg")]).unwrap().ids;
        let (a, b) = (ids[0], ids[1]);
        db.set_stars(&[a], true);
        db.set_caption(a, "Sunset");
        db.rotate(&[a], 90);
        db.set_pins(&[b], 1.5, 2.5);
        db.shift_taken(&[b], 3600);
        let album = db.albums()[0].id;
        db.add_to_album(album, &[a, b]);
        let steps = r#"[{"control":"exposure","value":0.5}]"#;
        db.save_edit_history(a, steps, 1, 1);
        db.set_setting("user_presets", r#"[{"name":"Mine"}]"#);
        let track = Track {
            name: "Walk".into(),
            segments: vec![vec![Point { lat: 25.0, lon: 121.0, time: Some(1) }, Point { lat: 25.1, lon: 121.1, time: Some(60) }]],
        };
        db.store_track(folder_id, &path("walk.gpx"), 10, 1, &track).unwrap();
        db.exclude_dir(&photos.join("skip")).unwrap();
        let broken = Unreadable { path: path("broken.jpg"), reason: "damaged".into(), folder: false, bytes: 5, modified_at: 6 };
        db.store_unreadable(folder_id, &[broken], true, &Default::default());
        let trip = db.create_trip(&crate::trip::TripDraft::new("Taipei", "Asia/Taipei")).unwrap();
        let members = db.add_to_trip(trip, &[a, b]).unwrap();
        let day = db.add_trip_day(trip, None, "Arrival").unwrap();
        db.move_to_day(trip, Some(day), &members[..1], crate::trip::DaySource::User).unwrap();
        db.link_trip_track(trip, db.tracks()[0].0, Some(day)).unwrap();
        let story = db.create_story(trip, "Rain").unwrap();
        let blocks = [crate::trip::StoryBlock::new(crate::trip::Block::Photo(crate::trip::MediaUse::new(members[1])))];
        db.save_story_blocks(story, 1, &blocks).unwrap();
        db.set_caption(ids[2], "kept for 90 days");
        db.remove_missing(folder_id, &[path("a.jpg"), path("b.jpg")]);
        drop(db);

        let backup = create(&dir).unwrap();
        let clean = library("everything-restored");
        std::fs::remove_file(clean.join(LIBRARY)).unwrap();
        stage_restore(&clean, &backup).unwrap();
        apply_staged(&clean).unwrap().unwrap();

        let db = Database::open(&clean.join(LIBRARY)).unwrap();
        let a = db.photo(a).unwrap();
        assert!(a.starred);
        assert_eq!((a.caption.as_str(), a.rotation, a.camera.as_str(), a.lens.as_str()), ("Sunset", 90, "Camera", "Lens"));
        assert_eq!((a.gps_lat, a.gps_lon), (Some(25.03), Some(121.56)));
        let b = db.photo(b).unwrap();
        assert_eq!((b.pin_lat, b.pin_lon, b.taken_at), (Some(1.5), Some(2.5), 1_700_003_600));
        assert_eq!(db.photos(&Nav::Album(album), "", Sort::Name).len(), 2);
        assert_eq!(db.albums()[0].name, "Trip");
        assert_eq!(db.edit_history(a.id), Some((steps.to_string(), 1)));
        assert_eq!(db.setting("user_presets").as_deref(), Some(r#"[{"name":"Mine"}]"#));
        assert_eq!(db.tracks()[0].3, track);
        assert_eq!(db.excluded_dirs().len(), 1);
        assert_eq!(db.unreadable().len(), 1);
        assert_eq!(db.folders().len(), 1);
        let trips = db.trips(&crate::trip::TripQuery::default());
        assert_eq!(trips.iter().map(|trip| trip.title.as_str()).collect::<Vec<_>>(), ["Taipei"], "trips came along");
        assert_eq!(db.trip_days(trip)[0].title, "Arrival");
        assert_eq!(db.count_trip_members(trip, &crate::trip::MemberQuery::default()), 2);
        assert_eq!(db.trip_tracks(trip)[0].day_id, Some(day));
        assert_eq!(db.story(story).unwrap().title, "Rain");
        assert_eq!(db.story_blocks(story), blocks, "with the story's blocks and their ids");
        let back = db.write_batch(&[record("gone.jpg")]).unwrap().ids[0];
        assert_eq!(db.adopt_archived(), vec![back], "the archive of removed photos came along");
        assert_eq!(db.photo(back).unwrap().caption, "kept for 90 days");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&clean);
    }

    #[test]
    fn a_backup_that_cannot_be_written_reports_it_and_leaves_nothing_behind() {
        let dir = library("unwritable");
        std::fs::write(super::dir(&dir), b"a file where the backups folder goes").unwrap();
        assert!(create(&dir).is_err());

        let missing = library("no-library");
        std::fs::remove_file(missing.join(LIBRARY)).unwrap();
        assert!(create(&missing).is_err());
        let left: Vec<_> = std::fs::read_dir(super::dir(&missing)).unwrap().flatten().collect();
        assert!(left.is_empty(), "no partial copy is left");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&missing);
    }

    fn damage(dir: &Path) -> Vec<u8> {
        let garbage = b"not a database, just some bytes that SQLite cannot read".repeat(200);
        std::fs::write(dir.join(LIBRARY), &garbage).unwrap();
        assert!(crate::db::Database::open(&dir.join(LIBRARY)).is_err());
        std::fs::write(dir.join(format!("{LIBRARY}-wal")), b"wal").unwrap();
        garbage
    }

    fn damaged_copies(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("picasso-damaged-"))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn a_damaged_library_is_kept_aside_and_a_backup_takes_its_place() {
        let dir = library("damaged");
        let backup = create(&dir).unwrap();
        let garbage = damage(&dir);
        let aside = replace_damaged(&dir, Some(&backup)).unwrap();
        assert_eq!(std::fs::read(&aside).unwrap(), garbage, "the damaged library is kept as it was");
        let wal = PathBuf::from(format!("{}-wal", aside.display()));
        assert_eq!(std::fs::read(&wal).unwrap(), b"wal", "unsaved changes go with it");
        assert!(damaged_copies(&dir).contains(&aside));
        assert!(!dir.join(format!("{LIBRARY}-wal")).exists());
        let db = crate::db::Database::open(&dir.join(LIBRARY)).unwrap();
        assert_eq!(db.albums().len(), 1, "the backup's album is back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_library_can_make_way_for_a_new_one() {
        let dir = library("damaged-new");
        let garbage = damage(&dir);
        let aside = replace_damaged(&dir, None).unwrap();
        assert_eq!(std::fs::read(&aside).unwrap(), garbage);
        let db = crate::db::Database::open(&dir.join(LIBRARY)).unwrap();
        assert!(db.albums().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_library_another_program_is_writing_to_passes_the_check() {
        let dir = library("busy");
        let writer = Connection::open(dir.join(LIBRARY)).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE; INSERT INTO albums (name, created_at) VALUES ('Busy', 0);").unwrap();
        assert!(check(&dir.join(LIBRARY)).is_ok(), "a busy library is not damaged, so it is never replaced");
        writer.execute_batch("ROLLBACK").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn there_is_nothing_to_replace_without_a_library() {
        let dir = library("no-library-to-replace");
        std::fs::remove_file(dir.join(LIBRARY)).unwrap();
        assert!(replace_damaged(&dir, None).is_err());
        assert!(damaged_copies(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_backup_leaves_the_damaged_library_where_it_was() {
        let dir = library("damaged-bad-backup");
        let garbage = damage(&dir);
        let bad = dir.join("bad.db");
        std::fs::write(&bad, b"also not a database").unwrap();
        assert!(replace_damaged(&dir, Some(&bad)).is_err());
        assert_eq!(std::fs::read(dir.join(LIBRARY)).unwrap(), garbage);
        assert!(damaged_copies(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
