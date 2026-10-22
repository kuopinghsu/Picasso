//! Library records shared by the database, the scanner, and the UI.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// Video extensions Picasso indexes and plays through FFmpeg.
pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "qt", "avi", "mkv", "webm", "wmv", "asf", "flv", "mpg", "mpeg", "m2v", "mts", "m2ts", "3gp", "3g2", "vob", "ogv", "dv",
];

const HEIC_EXTS: &[&str] = &["heic", "heif", "hif"];

/// 1826-01-01 UTC, before the oldest surviving photograph. An earlier date
/// taken from a file is a placeholder, not when the photo was taken.
pub const EARLIEST_PHOTO: i64 = -4_544_208_000;

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
    /// Running time in seconds for a video, 0 for a still photo.
    pub duration: f64,
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

    pub fn is_video(&self) -> bool {
        is_video_path(Path::new(&self.path))
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
    /// A subfolder inside an added folder, with everything below it.
    Dir(String),
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
    pub duration: f64,
}

pub fn is_video_path(path: &Path) -> bool {
    extension_in(path, VIDEO_EXTS)
}

/// Files the library indexes: photos and videos of the types turned on in
/// Settings.
pub fn is_media_path(path: &Path) -> bool {
    FileType::of(path).is_some_and(FileType::enabled)
}

/// Groups of the extensions Picasso indexes, each turned on or off in
/// Settings. Matching is case-insensitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Jpeg,
    Png,
    Gif,
    Webp,
    Bmp,
    Tiff,
    Heic,
    Raw,
    Video,
}

/// Bit per [`FileType`] that the scanner indexes; all of them by default.
static ENABLED_TYPES: AtomicU32 = AtomicU32::new(u32::MAX);

impl FileType {
    pub const ALL: [FileType; 9] = [
        FileType::Jpeg,
        FileType::Png,
        FileType::Gif,
        FileType::Webp,
        FileType::Bmp,
        FileType::Tiff,
        FileType::Heic,
        FileType::Raw,
        FileType::Video,
    ];

    /// Name kept in the settings file.
    pub fn code(self) -> &'static str {
        match self {
            FileType::Jpeg => "jpeg",
            FileType::Png => "png",
            FileType::Gif => "gif",
            FileType::Webp => "webp",
            FileType::Bmp => "bmp",
            FileType::Tiff => "tiff",
            FileType::Heic => "heic",
            FileType::Raw => "raw",
            FileType::Video => "video",
        }
    }

    pub fn from_code(code: &str) -> Option<FileType> {
        FileType::ALL.into_iter().find(|kind| kind.code() == code)
    }

    /// English name, translated by the caller.
    pub fn label(self) -> &'static str {
        match self {
            FileType::Jpeg => "JPEG",
            FileType::Png => "PNG",
            FileType::Gif => "GIF",
            FileType::Webp => "WebP",
            FileType::Bmp => "BMP",
            FileType::Tiff => "TIFF",
            FileType::Heic => "HEIC / HEIF",
            FileType::Raw => "Camera RAW",
            FileType::Video => "Videos",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            FileType::Jpeg => &["jpg", "jpeg"],
            FileType::Png => &["png"],
            FileType::Gif => &["gif"],
            FileType::Webp => &["webp"],
            FileType::Bmp => &["bmp"],
            FileType::Tiff => &["tif", "tiff"],
            FileType::Heic => HEIC_EXTS,
            FileType::Raw => RAW_EXTS,
            FileType::Video => VIDEO_EXTS,
        }
    }

    pub fn of(path: &Path) -> Option<FileType> {
        FileType::ALL.into_iter().find(|kind| extension_in(path, kind.extensions()))
    }

    fn bit(self) -> u32 {
        1 << FileType::ALL.iter().position(|kind| *kind == self).unwrap_or(0)
    }

    pub fn enabled(self) -> bool {
        ENABLED_TYPES.load(Ordering::Relaxed) & self.bit() != 0
    }

    /// Sets the types the scanner indexes from now on.
    pub fn set_enabled(types: &[FileType]) {
        ENABLED_TYPES.store(types.iter().fold(0, |bits, kind| bits | kind.bit()), Ordering::Relaxed);
    }
}

/// `m:ss`, or `h:mm:ss` from one hour up.
pub fn format_duration(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let (hours, minutes, secs) = (total / 3600, (total / 60) % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
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
        .map(|dt| long_date(&dt.with_timezone(&chrono::Local), crate::i18n::current()))
        .unwrap_or_else(|| crate::i18n::tr("Unknown date").to_string())
}

pub fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| {
            let dt = dt.with_timezone(&chrono::Local);
            format!("{}  {}", long_date(&dt, crate::i18n::current()), dt.format("%H:%M"))
        })
        .unwrap_or_else(|| crate::i18n::tr("Unknown date").to_string())
}

/// Weekday and date, written the way the language writes dates.
fn long_date<Tz: chrono::TimeZone>(dt: &chrono::DateTime<Tz>, lang: crate::i18n::Lang) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Datelike;
    use crate::i18n::Lang;
    let weekday = dt.weekday().num_days_from_sunday() as usize;
    let (year, month, day) = (dt.year(), dt.month(), dt.day());
    match lang {
        Lang::English => dt.format("%A, %B %-d, %Y").to_string(),
        Lang::TraditionalChinese | Lang::SimplifiedChinese => {
            const DAYS: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];
            format!("{year}年{month}月{day}日 星期{}", DAYS[weekday])
        }
        Lang::Japanese => {
            const DAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];
            format!("{year}年{month}月{day}日（{}）", DAYS[weekday])
        }
        Lang::Korean => {
            const DAYS: [&str; 7] = ["일", "월", "화", "수", "목", "금", "토"];
            format!("{year}년 {month}월 {day}일 {}요일", DAYS[weekday])
        }
        Lang::German => {
            const DAYS: [&str; 7] = ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"];
            const MONTHS: [&str; 12] = [
                "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November",
                "Dezember",
            ];
            format!("{}, {day}. {} {year}", DAYS[weekday], MONTHS[month as usize - 1])
        }
        Lang::Spanish => {
            const DAYS: [&str; 7] = ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"];
            const MONTHS: [&str; 12] = [
                "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre",
                "noviembre", "diciembre",
            ];
            format!("{}, {day} de {} de {year}", DAYS[weekday], MONTHS[month as usize - 1])
        }
        Lang::French => {
            const DAYS: [&str; 7] = ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"];
            const MONTHS: [&str; 12] = [
                "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre",
                "novembre", "décembre",
            ];
            let day = if day == 1 { "1er".to_string() } else { day.to_string() };
            format!("{} {day} {} {year}", DAYS[weekday], MONTHS[month as usize - 1])
        }
        Lang::Italian => {
            const DAYS: [&str; 7] = ["domenica", "lunedì", "martedì", "mercoledì", "giovedì", "venerdì", "sabato"];
            const MONTHS: [&str; 12] = [
                "gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno", "luglio", "agosto", "settembre", "ottobre",
                "novembre", "dicembre",
            ];
            format!("{} {day} {} {year}", DAYS[weekday], MONTHS[month as usize - 1])
        }
        Lang::Portuguese => {
            const DAYS: [&str; 7] = [
                "domingo", "segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado",
            ];
            const MONTHS: [&str; 12] = [
                "janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro",
                "novembro", "dezembro",
            ];
            format!("{}, {day} de {} de {year}", DAYS[weekday], MONTHS[month as usize - 1])
        }
    }
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
        assert_eq!(FileType::of(Path::new("IMG_0001.HEIC")), Some(FileType::Heic));
        assert!(is_heic_path(Path::new("photo.heif")));
        assert_eq!(FileType::of(Path::new("DSC_0001.NEF")), Some(FileType::Raw));
        assert_eq!(FileType::of(Path::new("B0001234.3fr")), Some(FileType::Raw));
        assert!(is_raw_path(Path::new("capture.FFF")));
        assert!(is_raw_path(Path::new("compact.NRW")));
        assert!(!is_raw_path(Path::new("picture.jpg")));
        assert!(!is_heic_path(Path::new("picture.jpg")));
        assert!(is_video_path(Path::new("clip.MOV")));
        assert!(is_media_path(Path::new("clip.mp4")));
        assert_eq!(FileType::of(Path::new("clip.mp4")), Some(FileType::Video));
    }

    #[test]
    fn every_extension_has_one_file_type() {
        let all: Vec<&str> = FileType::ALL.iter().flat_map(|kind| kind.extensions().iter().copied()).collect();
        for ext in &all {
            assert_eq!(all.iter().filter(|other| *other == ext).count(), 1, "{ext}");
            assert_eq!(*ext, ext.to_ascii_lowercase());
        }
        assert_eq!(FileType::of(Path::new("B0000008.3FR")), Some(FileType::Raw));
        assert_eq!(FileType::of(Path::new("a.JPEG")), Some(FileType::Jpeg));
        assert_eq!(FileType::of(Path::new("notes.txt")), None);
        for kind in FileType::ALL {
            assert_eq!(FileType::from_code(kind.code()), Some(kind));
        }
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(0.4), "0:00");
        assert_eq!(format_duration(75.9), "1:15");
        assert_eq!(format_duration(3725.0), "1:02:05");
    }

    #[test]
    fn writes_dates_per_language() {
        use crate::i18n::Lang;
        let dt = chrono::DateTime::from_timestamp(1_785_888_000, 0).unwrap();
        assert_eq!(long_date(&dt, Lang::English), "Wednesday, August 5, 2026");
        assert_eq!(long_date(&dt, Lang::TraditionalChinese), "2026年8月5日 星期三");
        assert_eq!(long_date(&dt, Lang::Japanese), "2026年8月5日（水）");
        assert_eq!(long_date(&dt, Lang::Korean), "2026년 8월 5일 수요일");
        assert_eq!(long_date(&dt, Lang::German), "Mittwoch, 5. August 2026");
        assert_eq!(long_date(&dt, Lang::Spanish), "miércoles, 5 de agosto de 2026");
        assert_eq!(long_date(&dt, Lang::French), "mercredi 5 août 2026");
        assert_eq!(long_date(&dt, Lang::Italian), "mercoledì 5 agosto 2026");
        assert_eq!(long_date(&dt, Lang::Portuguese), "quarta-feira, 5 de agosto de 2026");
    }

    #[test]
    fn rotation_wraps_in_quarter_turns() {
        assert_eq!(turn(0, 90), 90);
        assert_eq!(turn(270, 90), 0);
        assert_eq!(turn(0, -90), 270);
        assert_eq!(turn(180, -450), 90);
    }
}
