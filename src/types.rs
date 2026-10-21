//! Library records shared by the database, the scanner, and the UI.

use std::path::Path;

/// Image extensions Picasso indexes. Matching is case-insensitive.
pub const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "hif", "3fr", "ari", "arw", "cr2", "cr3",
    "crm", "crw", "dcr", "dcs", "dng", "erf", "fff", "iiq", "kdc", "mef", "mos", "mrw", "nef", "nrw", "orf", "ori", "pef",
    "qtk", "raf", "raw", "rw2", "rwl", "srw", "x3f",
];

const HEIC_EXTS: &[&str] = &["heic", "heif", "hif"];

const RAW_EXTS: &[&str] = &[
    "3fr", "ari", "arw", "cr2", "cr3", "crm", "crw", "dcr", "dcs", "dng", "erf", "fff", "iiq", "kdc", "mef", "mos", "mrw",
    "nef", "nrw", "orf", "ori", "pef", "qtk", "raf", "raw", "rw2", "rwl", "srw", "x3f",
];

#[derive(Clone, Debug)]
pub struct Folder {
    pub id: i64,
    pub path: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct Photo {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub taken_at: i64,
    pub modified_at: i64,
    pub starred: bool,
    pub caption: String,
    pub camera: String,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub pin_lat: Option<f64>,
    pub pin_lon: Option<f64>,
    /// Clockwise turn chosen in Picasso: 0, 90, 180, or 270 degrees. Stored
    /// only in the catalog and applied on screen; the file is never rewritten.
    pub rotation: i32,
}

/// Adds `delta` degrees to a rotation and normalizes it to 0, 90, 180, or 270.
pub fn turn(rotation: i32, delta: i32) -> i32 {
    (rotation + delta).div_euclid(90).rem_euclid(4) * 90
}

impl Photo {
    /// User pin if one was placed, otherwise the camera GPS coordinate.
    pub fn place(&self) -> Option<(f64, f64)> {
        match (self.pin_lat, self.pin_lon) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ => match (self.gps_lat, self.gps_lon) {
                (Some(lat), Some(lon)) => Some((lat, lon)),
                _ => None,
            },
        }
    }

    pub fn pinned(&self) -> bool {
        self.pin_lat.is_some() && self.pin_lon.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    DateNewest,
    DateOldest,
    Name,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav {
    Library,
    Starred,
    Folder(i64),
    Album(i64),
    Places,
}

#[derive(Clone, Debug)]
pub struct NewPhoto {
    pub folder_id: i64,
    pub path: String,
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub taken_at: i64,
    pub modified_at: i64,
    pub camera: String,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
}

pub fn is_image_path(path: &Path) -> bool {
    extension_in(path, IMAGE_EXTS)
}

pub fn is_heic_path(path: &Path) -> bool {
    extension_in(path, HEIC_EXTS)
}

pub fn is_raw_path(path: &Path) -> bool {
    extension_in(path, RAW_EXTS)
}

fn extension_in(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| extensions.iter().any(|known| ext.eq_ignore_ascii_case(known)))
        .unwrap_or(false)
}

pub fn folder_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

/// Pixel size of a photo as displayed: camera orientation first, then the
/// rotation stored in the catalog.
pub fn displayed_size(photo: &Photo) -> (f32, f32) {
    let (w, h) = (photo.width.max(1) as f32, photo.height.max(1) as f32);
    if photo.rotation.rem_euclid(180) == 90 {
        (h, w)
    } else {
        (w, h)
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let n = bytes as f64;
    if n < KB {
        format!("{bytes} B")
    } else if n < MB {
        format!("{:.0} KB", n / KB)
    } else if n < GB {
        format!("{:.1} MB", n / MB)
    } else {
        format!("{:.2} GB", n / GB)
    }
}

pub fn format_day(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%A, %B %-d, %Y")
                .to_string()
        })
        .unwrap_or_else(|| "Unknown date".to_string())
}

pub fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%A, %B %-d, %Y  %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Unknown date".to_string())
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_heic_and_camera_raw() {
        assert!(is_image_path(Path::new("IMG_0001.HEIC")));
        assert!(is_heic_path(Path::new("photo.heif")));
        assert!(is_image_path(Path::new("DSC_0001.NEF")));
        assert!(is_image_path(Path::new("B0001234.3fr")));
        assert!(is_raw_path(Path::new("capture.FFF")));
        assert!(is_raw_path(Path::new("compact.NRW")));
        assert!(!is_raw_path(Path::new("picture.jpg")));
        assert!(!is_heic_path(Path::new("picture.jpg")));
    }

    #[test]
    fn rotation_wraps_in_quarter_turns() {
        assert_eq!(turn(0, 90), 90);
        assert_eq!(turn(270, 90), 0);
        assert_eq!(turn(0, -90), 270);
        assert_eq!(turn(180, -450), 90);
    }
}
