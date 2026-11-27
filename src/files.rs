//! Files the library points at that may be out of reach: deleted, moved,
//! or on a drive that isn't connected, so reading them says which in
//! words rather than with the system's own message.

use std::path::Path;

use crate::i18n::tr;

/// Where drives are mounted: in these folders, or in a folder of each
/// user's in the last two, such as /media/name.
const MOUNTS: [&str; 4] = ["/Volumes", "/mnt", "/media", "/run/media"];

/// Why `path` couldn't be read, from the error reading it gave.
pub fn read_error(path: &Path, err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::NotFound {
        missing_message(path)
    } else {
        err.to_string()
    }
}

/// Why `path` can't be read, when it isn't there.
pub fn missing(path: &Path) -> Option<String> {
    (!path.exists()).then(|| missing_message(path))
}

/// The words for a missing `path`: on a drive that isn't connected, or
/// not found.
fn missing_message(path: &Path) -> String {
    if on_missing_drive(path) {
        tr("The file's folder can't be reached. Is its drive connected?").to_string()
    } else {
        tr("The file wasn't found").to_string()
    }
}

/// Whether the missing `path` is on a drive that isn't mounted: the
/// nearest of its folders that is there is where drives are mounted, or
/// none is, as for a drive letter that is gone.
fn on_missing_drive(path: &Path) -> bool {
    on_missing_drive_in(path, &MOUNTS.map(Path::new))
}

/// [`on_missing_drive`] with drives mounted in `mounts`, the last two of
/// which also hold a folder for each user.
fn on_missing_drive_in(path: &Path, mounts: &[&Path; 4]) -> bool {
    // A name on its own is in the current folder, which is there.
    let nearest = path
        .ancestors()
        .skip(1)
        .find(|dir| !dir.as_os_str().is_empty() && dir.is_dir());
    let Some(nearest) = nearest else {
        return !path.is_relative();
    };
    mounts.contains(&nearest)
        || nearest
            .parent()
            .is_some_and(|parent| mounts[2..].contains(&parent))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_says_whether_its_drive_is_connected() {
        let dir = std::env::temp_dir().join(format!("picasso-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let not_found = "The file wasn't found";
        // Deleted, moved, or mistyped, even with its folder.
        assert_eq!(missing(&dir.join("gone.jpg")).unwrap(), not_found);
        assert_eq!(missing(&dir.join("gone/trip/2009.gpx")).unwrap(), not_found);
        assert_eq!(
            missing(Path::new("/no-such-root-xyz/a.mov")).unwrap(),
            not_found
        );
        // A name on its own is looked for in the current folder.
        assert_eq!(
            missing(Path::new("picasso-no-such-file.gpx")).unwrap(),
            not_found
        );
        assert!(missing(&dir).is_none(), "one that is there");
        let gone = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(read_error(&dir.join("gone.heic"), &gone), not_found);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(read_error(&dir, &denied), denied.to_string());
        // On a drive that isn't mounted, in folders made here for where
        // drives are mounted.
        let names = ["Volumes", "mnt", "media", "run-media"];
        std::fs::create_dir_all(dir.join("media/someone")).unwrap();
        for name in names {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        let mounts = names.map(|name| dir.join(name));
        let mounts = [&*mounts[0], &*mounts[1], &*mounts[2], &*mounts[3]];
        let unplugged = |path: &str| on_missing_drive_in(&dir.join(path), &mounts);
        assert!(unplugged("Volumes/backup/Images/a.jpg"));
        assert!(unplugged("mnt/backup/a.jpg"));
        assert!(unplugged("media/someone/backup/a.jpg"));
        assert!(
            !unplugged("gone/a.jpg"),
            "a folder deleted from a disk that is there"
        );
        assert!(!on_missing_drive_in(Path::new("a.jpg"), &mounts));
        #[cfg(windows)]
        assert!(on_missing_drive(Path::new(r"Q:\Images\a.jpg")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
