//! Library records shared by the database, the scanner, and the UI.

use std::path::Path;

/// Image extensions Picasso indexes. Matching is case-insensitive.
pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"];

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
pub struct Person {
    pub id: i64,
    pub name: Option<String>,
    pub photo_count: i64,
    pub face_count: i64,
    pub cover_face_id: i64,
    pub cover_path: String,
    pub cover_rotation: i32,
    pub cover: RelRect,
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
    pub rotation: i32,
    pub camera: String,
    pub faces_scanned: bool,
    pub face_count: i64,
}

#[derive(Clone, Debug)]
pub struct Face {
    pub id: i64,
    pub photo_id: i64,
    pub person_id: i64,
    pub person_name: Option<String>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub confirmed: bool,
    pub manual: bool,
}

impl Face {
    pub fn rect(&self) -> RelRect {
        RelRect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
}

/// Face or crop rectangle in displayed-image space, each edge in `0..1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
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
    People,
    Person(i64),
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
}

pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| IMAGE_EXTS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
        .unwrap_or(false)
}

pub fn folder_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

/// Pixel size of a photo after the user's rotation is applied.
pub fn displayed_size(photo: &Photo) -> (f32, f32) {
    let (w, h) = (photo.width.max(1) as f32, photo.height.max(1) as f32);
    if photo.rotation % 180 == 0 {
        (w, h)
    } else {
        (h, w)
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
