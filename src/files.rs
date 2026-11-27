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

/// Whether the file system holding `dir` updates a folder's modification
/// time whenever a file is added to it, removed or renamed, so a folder
/// whose time hasn't moved can be taken as unchanged. FAT and exFAT, as on
/// memory cards and many USB drives, don't always, nor do file systems this
/// can't name, so their folders are always listed.
pub fn folder_times_kept(dir: &Path) -> bool {
    file_system(dir).is_some_and(|name| keeps_folder_times(&name))
}

/// File systems known to keep folder times: names as macOS and Windows give
/// them, and the magic numbers Linux does, in hexadecimal.
fn keeps_folder_times(name: &str) -> bool {
    const KEPT: &[&str] = &[
        "apfs", "hfs", "ufs", "zfs", "smbfs", "nfs", "ntfs", "refs",
        // ext2, 3 and 4; Btrfs; XFS; ZFS; NFS; SMB2; CIFS; F2FS; tmpfs;
        // overlayfs, which passes them on from the file systems beneath.
        "ef53", "9123683e", "58465342", "2fc12fc1", "6969", "fe534d42", "ff534d42", "f2f52010",
        "1021994", "794c7630",
    ];
    KEPT.contains(&name.to_ascii_lowercase().as_str())
}

#[cfg(target_os = "macos")]
fn file_system(dir: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: a zeroed statfs is valid, and `path` ends in a zero.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    // SAFETY: the system fills the name with a string ending in a zero.
    let name = unsafe { std::ffi::CStr::from_ptr(stat.f_fstypename.as_ptr()) };
    Some(name.to_string_lossy().into_owned())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn file_system(dir: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: a zeroed statfs is valid, and `path` ends in a zero.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    // The magic number is a 32-bit value in a field whose width differs.
    #[allow(clippy::unnecessary_cast)]
    let magic = stat.f_type as u64 as u32;
    Some(format!("{magic:x}"))
}

#[cfg(windows)]
fn file_system(dir: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetVolumePathNameW(file: *const u16, volume: *mut u16, length: u32) -> i32;
        #[allow(clippy::too_many_arguments)]
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_length: u32,
            serial: *mut u32,
            longest_name: *mut u32,
            flags: *mut u32,
            file_system: *mut u16,
            file_system_length: u32,
        ) -> i32;
    }
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut volume = vec![0u16; wide.len() + 64];
    // SAFETY: both buffers end in a zero and their lengths are passed.
    let found =
        unsafe { GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) };
    if found == 0 {
        return None;
    }
    let mut name = [0u16; 64];
    use std::ptr::null_mut;
    // SAFETY: `volume` ends in a zero, `name`'s length is passed, and the
    // values not wanted are null, which the call accepts.
    let read = unsafe {
        GetVolumeInformationW(
            volume.as_ptr(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if read == 0 {
        return None;
    }
    let end = name
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(name.len());
    Some(String::from_utf16_lossy(&name[..end]))
}

#[cfg(not(any(unix, windows)))]
fn file_system(_dir: &Path) -> Option<String> {
    None
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

    #[test]
    fn folder_times_are_trusted_only_on_file_systems_that_keep_them() {
        for kept in ["apfs", "HFS", "smbfs", "NTFS", "ef53", "ff534d42"] {
            assert!(keeps_folder_times(kept), "{kept}");
        }
        for unknown in [
            "msdos", "exfat", "FAT32", "exFAT", "4d44", "2011bab0", "osxfuse", "",
        ] {
            assert!(!keeps_folder_times(unknown), "{unknown}");
        }
        // The disk the tests run from is one of them on macOS and Linux.
        #[cfg(target_os = "macos")]
        assert!(folder_times_kept(&std::env::temp_dir()));
    }
}
