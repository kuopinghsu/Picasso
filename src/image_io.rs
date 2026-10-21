//! Image decode, EXIF orientation, and thumbnail encoding.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, GenericImageView, ImageBuffer, Rgba};

use crate::types::{unix_now, RelRect};

pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub taken_at: i64,
    pub camera: String,
}

pub fn probe(path: &Path, modified_at: i64) -> Result<Probe, String> {
    let (mut width, mut height) = image::image_dimensions(path).map_err(|err| err.to_string())?;
    let exif = read_exif(path);
    if matches!(exif.orientation, 5 | 6 | 7 | 8) {
        std::mem::swap(&mut width, &mut height);
    }
    Ok(Probe {
        width,
        height,
        taken_at: exif.taken_at.unwrap_or(modified_at),
        camera: exif.camera,
    })
}

pub fn load_oriented(path: &Path) -> Result<DynamicImage, String> {
    let image = image::open(path).map_err(|err| err.to_string())?;
    let orientation = read_exif(path).orientation;
    Ok(apply_orientation(image, orientation))
}

pub fn load_displayed(path: &Path, rotation: i32) -> Result<DynamicImage, String> {
    Ok(apply_turns(load_oriented(path)?, rotation))
}

pub fn apply_turns(image: DynamicImage, rotation: i32) -> DynamicImage {
    match rotation.rem_euclid(360) {
        90 => image.rotate90(),
        180 => image.rotate180(),
        270 => image.rotate270(),
        _ => image,
    }
}

fn apply_orientation(image: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

pub fn thumbnail_rgba(image: &DynamicImage, edge: u32) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    image.thumbnail(edge, edge).to_rgba8()
}

pub fn limit_rgba(image: DynamicImage, max_edge: u32) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = image.dimensions();
    let long = width.max(height);
    if long <= max_edge || long == 0 {
        return image.to_rgba8();
    }
    let scale = max_edge as f32 / long as f32;
    let nw = ((width as f32) * scale).round().max(1.0) as u32;
    let nh = ((height as f32) * scale).round().max(1.0) as u32;
    image
        .resize_exact(nw, nh, image::imageops::FilterType::Triangle)
        .to_rgba8()
}

pub fn face_crop(image: &DynamicImage, rect: RelRect, edge: u32) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = image.dimensions();
    let iw = width as f32;
    let ih = height as f32;
    let cx = (rect.x + rect.w * 0.5) * iw;
    let cy = (rect.y + rect.h * 0.5) * ih;
    let mut side = (rect.w * iw).max(rect.h * ih) * 1.25;
    side = side.max(8.0).min(iw).min(ih);
    let mut x = (cx - side * 0.5).clamp(0.0, (iw - 1.0).max(0.0));
    let mut y = (cy - side * 0.5).clamp(0.0, (ih - 1.0).max(0.0));
    if x + side > iw {
        x = (iw - side).max(0.0);
    }
    if y + side > ih {
        y = (ih - side).max(0.0);
    }
    let side = side.min(iw - x).min(ih - y).max(1.0);
    image
        .crop_imm(x as u32, y as u32, side as u32, side as u32)
        .resize_exact(edge, edge, image::imageops::FilterType::Triangle)
        .to_rgba8()
}

pub fn save_jpeg(path: &Path, rgba: &ImageBuffer<Rgba<u8>, Vec<u8>>, quality: u8) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let rgb = DynamicImage::ImageRgba8(rgba.clone()).into_rgb8();
    let mut file = File::create(path).map_err(|err| err.to_string())?;
    let mut encoder = JpegEncoder::new_with_quality(&mut file, quality);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|err| err.to_string())
}

pub fn file_mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_else(unix_now)
}

struct ExifBits {
    orientation: u16,
    taken_at: Option<i64>,
    camera: String,
}

fn read_exif(path: &Path) -> ExifBits {
    let mut bits = ExifBits {
        orientation: 1,
        taken_at: None,
        camera: String::new(),
    };
    let Ok(file) = File::open(path) else {
        return bits;
    };
    let mut reader = BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return bits;
    };
    if let Some(value) = short_field(&exif, exif::Tag::Orientation) {
        bits.orientation = value;
    }
    bits.taken_at = ascii_field(&exif, exif::Tag::DateTimeOriginal)
        .or_else(|| ascii_field(&exif, exif::Tag::DateTime))
        .as_deref()
        .and_then(parse_exif_time);
    let model = ascii_field(&exif, exif::Tag::Model);
    let make = ascii_field(&exif, exif::Tag::Make);
    bits.camera = match (make, model) {
        (Some(make), Some(model)) if !model.starts_with(&make) => format!("{make} {model}"),
        (_, Some(model)) => model,
        (Some(make), _) => make,
        _ => String::new(),
    };
    bits
}

fn short_field(exif: &exif::Exif, tag: exif::Tag) -> Option<u16> {
    let field = exif.get_field(tag, exif::In::PRIMARY)?;
    match &field.value {
        exif::Value::Short(values) => values.first().copied(),
        _ => None,
    }
}

fn ascii_field(exif: &exif::Exif, tag: exif::Tag) -> Option<String> {
    let field = exif.get_field(tag, exif::In::PRIMARY)?;
    match &field.value {
        exif::Value::Ascii(chunks) => chunks.first().map(|bytes| {
            String::from_utf8_lossy(bytes)
                .trim()
                .trim_matches(|c| c == '"' || c == '\0')
                .trim()
                .to_string()
        }),
        _ => None,
    }
    .filter(|text| !text.is_empty())
}

fn parse_exif_time(text: &str) -> Option<i64> {
    let naive = chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y:%m:%d %H:%M:%S").ok()?;
    naive
        .and_local_timezone(chrono::Local)
        .single()
        .map(|dt| dt.timestamp())
        .or_else(|| Some(naive.and_utc().timestamp()))
}
