//! The system file manager and trash.

use std::path::Path;

/// `path` without the `\\?\` prefix that canonical Windows paths carry,
/// which Explorer and the trash don't use.
pub fn plain_path(path: &str) -> String {
    if let Some(share) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    }
}

/// Opens the folder that holds `path` with the file selected.
pub fn reveal(path: &str) {
    let path = plain_path(path);
    if let Err(err) = reveal_selected(&path) {
        eprintln!("picasso reveal: {err}");
        if let Some(parent) = Path::new(&path).parent() {
            let _ = open::that(parent);
        }
    }
}

#[cfg(windows)]
fn reveal_selected(path: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // Explorer parses its own command line, so the quotes must reach it as typed.
    std::process::Command::new("explorer").raw_arg(format!("/select,\"{path}\"")).spawn().map(drop)
}

#[cfg(target_os = "macos")]
fn reveal_selected(path: &str) -> std::io::Result<()> {
    std::process::Command::new("open").arg("-R").arg(path).spawn().map(drop)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_selected(path: &str) -> std::io::Result<()> {
    let uri = file_uri(path);
    let path = path.to_string();
    // dbus-send waits for the file manager, so it runs off the UI thread.
    std::thread::spawn(move || {
        let shown = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
            ])
            .arg(format!("array:string:{uri}"))
            .arg("string:")
            .output()
            .is_ok_and(|output| output.status.success());
        if !shown {
            if let Some(parent) = Path::new(&path).parent() {
                let _ = open::that(parent);
            }
        }
    });
    Ok(())
}

/// `file://` URI with everything but unreserved characters and `/` escaped.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// True where files can be put back from the trash.
pub const CAN_RESTORE: bool = cfg!(any(
    target_os = "windows",
    all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))
));

/// Puts the newest trashed copy of each of `paths` back where it was.
/// Returns how many files came back.
#[cfg(any(
    target_os = "windows",
    all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))
))]
pub fn restore(paths: &[String]) -> Result<usize, String> {
    use std::collections::HashMap;

    let key = |path: &Path| {
        let text = plain_path(&path.to_string_lossy());
        if cfg!(windows) { text.to_lowercase() } else { text }
    };
    let wanted: std::collections::HashSet<String> = paths.iter().map(|path| key(Path::new(path))).collect();
    let mut newest: HashMap<String, trash::TrashItem> = HashMap::new();
    for item in trash::os_limited::list().map_err(|err| err.to_string())? {
        let path = key(&item.original_path());
        if !wanted.contains(&path) {
            continue;
        }
        if newest.get(&path).is_none_or(|kept| kept.time_deleted < item.time_deleted) {
            newest.insert(path, item);
        }
    }
    let mut items: Vec<trash::TrashItem> = newest.into_values().collect();
    // A file that is back on disk already stays as it is.
    items.retain(|item| !item.original_path().exists());
    let count = items.len();
    trash::os_limited::restore_all(items).map_err(|err| err.to_string())?;
    Ok(count)
}

#[cfg(not(any(
    target_os = "windows",
    all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))
)))]
pub fn restore(_paths: &[String]) -> Result<usize, String> {
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_drop_the_verbatim_prefix() {
        assert_eq!(plain_path(r"\\?\D:\Photos\a.jpg"), r"D:\Photos\a.jpg");
        assert_eq!(plain_path(r"\\?\UNC\nas\share\a.jpg"), r"\\nas\share\a.jpg");
        assert_eq!(plain_path("/home/me/a.jpg"), "/home/me/a.jpg");
    }

    #[test]
    fn file_uris_escape_spaces_and_commas() {
        assert_eq!(file_uri("/home/me/My Photos/a,b.jpg"), "file:///home/me/My%20Photos/a%2Cb.jpg");
    }
}
