//! Image decode, EXIF orientation, and thumbnail encoding.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, GenericImageView, ImageBuffer, Rgba};

use crate::types::{is_heic_path, is_raw_path, unix_now};

pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub taken_at: i64,
    pub camera: String,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
}

pub fn probe(path: &Path, modified_at: i64) -> Result<Probe, String> {
    if is_heic_path(path) {
        return probe_heic(path, modified_at);
    }
    if is_raw_path(path) {
        return probe_raw(path, modified_at);
    }
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
        gps_lat: exif.gps_lat,
        gps_lon: exif.gps_lon,
    })
}

pub fn load_oriented(path: &Path) -> Result<DynamicImage, String> {
    if is_heic_path(path) {
        return load_heic(path);
    }
    if is_raw_path(path) {
        return load_raw(path);
    }
    let image = image::open(path).map_err(|err| err.to_string())?;
    let orientation = read_exif(path).orientation;
    Ok(apply_orientation(image, orientation))
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
    gps_lat: Option<f64>,
    gps_lon: Option<f64>,
}

fn read_exif(path: &Path) -> ExifBits {
    let Ok(file) = File::open(path) else {
        return ExifBits::empty();
    };
    let mut reader = BufReader::new(file);
    match exif::Reader::new().read_from_container(&mut reader) {
        Ok(exif) => bits_from_exif(&exif),
        Err(_) => ExifBits::empty(),
    }
}

fn bits_from_exif(exif: &exif::Exif) -> ExifBits {
    let model = ascii_field(exif, exif::Tag::Model);
    let make = ascii_field(exif, exif::Tag::Make);
    ExifBits {
        orientation: short_field(exif, exif::Tag::Orientation).unwrap_or(1),
        taken_at: ascii_field(exif, exif::Tag::DateTimeOriginal)
            .or_else(|| ascii_field(exif, exif::Tag::DateTime))
            .as_deref()
            .and_then(parse_exif_time),
        camera: camera_name(make.as_deref().unwrap_or(""), model.as_deref().unwrap_or("")),
        gps_lat: gps_degrees(exif, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef),
        gps_lon: gps_degrees(exif, exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef),
    }
}

impl ExifBits {
    fn empty() -> Self {
        Self {
            orientation: 1,
            taken_at: None,
            camera: String::new(),
            gps_lat: None,
            gps_lon: None,
        }
    }

    fn probe(self, width: u32, height: u32, modified_at: i64) -> Probe {
        let (width, height) = oriented_size(width, height, self.orientation);
        Probe {
            width,
            height,
            taken_at: self.taken_at.unwrap_or(modified_at),
            camera: self.camera,
            gps_lat: self.gps_lat,
            gps_lon: self.gps_lon,
        }
    }
}

fn camera_name(make: &str, model: &str) -> String {
    let make = make.trim();
    let model = model.trim();
    match (make.is_empty(), model.is_empty()) {
        (true, true) => String::new(),
        (true, false) => model.to_string(),
        (false, true) => make.to_string(),
        (false, false) if model.starts_with(make) => model.to_string(),
        (false, false) => format!("{make} {model}"),
    }
}

fn oriented_size(width: u32, height: u32, orientation: u16) -> (u32, u32) {
    if matches!(orientation, 5 | 6 | 7 | 8) {
        (height, width)
    } else {
        (width, height)
    }
}

fn probe_heic(path: &Path, modified_at: i64) -> Result<Probe, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    let info = heic_rs::probe(&bytes).map_err(|err| err.to_string())?;
    let bits = heic_exif(&bytes);
    Ok(Probe {
        width: info.width,
        height: info.height,
        taken_at: bits.taken_at.unwrap_or(modified_at),
        camera: bits.camera,
        gps_lat: bits.gps_lat,
        gps_lon: bits.gps_lon,
    })
}

fn load_heic(path: &Path) -> Result<DynamicImage, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    let options = heic_rs::DecodeOptions {
        layout: heic_rs::PixelLayout::Rgba8,
        ..heic_rs::DecodeOptions::default()
    };
    let image = heic_rs::decode(&bytes, &options).map_err(|err| err.to_string())?;
    let buffer = image::ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(image.width, image.height, image.data)
        .ok_or_else(|| "HEIC decode returned an unexpected pixel buffer".to_string())?;
    Ok(DynamicImage::ImageRgba8(buffer))
}

fn heic_exif(bytes: &[u8]) -> ExifBits {
    let Ok(context) = heic_rs::context::Context::open(bytes) else {
        return ExifBits::empty();
    };
    let Ok(Some(tiff)) = context.exif(context.meta.primary) else {
        return ExifBits::empty();
    };
    exif::Reader::new()
        .read_raw(tiff.to_vec())
        .map(|exif| bits_from_exif(&exif))
        .unwrap_or_else(|_| ExifBits::empty())
}

/// Reads RAW dimensions and metadata from the file headers. The decoder runs
/// in dummy mode, so the sensor data stays on disk and a probe takes
/// milliseconds; the slower full decode is only used as a fallback.
fn probe_raw(path: &Path, modified_at: i64) -> Result<Probe, String> {
    let owned = path.to_path_buf();
    let quick = std::panic::catch_unwind(move || -> Result<(usize, usize, ExifBits), String> {
        let source = rawler::rawsource::RawSource::new(&owned).map_err(|err| err.to_string())?;
        let decoder = rawler::get_decoder(&source).map_err(|err| err.to_string())?;
        let params = rawler::decoders::RawDecodeParams::default();
        let bits = decoder
            .raw_metadata(&source, &params)
            .map(|metadata| bits_from_raw(&metadata))
            .unwrap_or_else(|_| ExifBits::empty());
        let raw = decoder.raw_image(&source, &params, true).map_err(|err| err.to_string())?;
        let (width, height) = match raw.crop_area {
            Some(area) if area.d.w > 0 && area.d.h > 0 => (area.d.w, area.d.h),
            _ => (raw.width, raw.height),
        };
        Ok((width, height, bits))
    });
    if let Ok(Ok((width, height, bits))) = quick {
        if width > 0 && height > 0 {
            return Ok(bits.probe(width as u32, height as u32, modified_at));
        }
    }
    let (image, bits) = raw_picture(path)?;
    Ok(bits.probe(image.width(), image.height(), modified_at))
}

fn load_raw(path: &Path) -> Result<DynamicImage, String> {
    let (image, bits) = raw_picture(path)?;
    Ok(apply_orientation(image, bits.orientation))
}

fn raw_picture(path: &Path) -> Result<(DynamicImage, ExifBits), String> {
    let source = rawler::rawsource::RawSource::new(path).map_err(|err| err.to_string())?;
    let decoder = rawler::get_decoder(&source).map_err(|err| err.to_string())?;
    let params = rawler::decoders::RawDecodeParams::default();
    let bits = decoder
        .raw_metadata(&source, &params)
        .map(|metadata| bits_from_raw(&metadata))
        .unwrap_or_else(|_| ExifBits::empty());
    let embedded = embedded_raw(decoder.as_ref(), &source);
    let usable = embedded.filter(|image| image.width().max(image.height()) >= 800);
    let image = match usable {
        Some(image) => image,
        None => develop_raw(path)?,
    };
    Ok((image, bits))
}

use rawler::decoders::Decoder;

fn embedded_raw(decoder: &dyn Decoder, source: &rawler::rawsource::RawSource) -> Option<DynamicImage> {
    let params = rawler::decoders::RawDecodeParams::default();
    [decoder.preview_image(source, &params), decoder.full_image(source, &params), decoder.thumbnail_image(source, &params)]
        .into_iter()
        .filter_map(|image| image.ok().flatten())
        .max_by_key(|image| u64::from(image.width()) * u64::from(image.height()))
}

fn develop_raw(path: &Path) -> Result<DynamicImage, String> {
    let path = path.to_path_buf();
    let developed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<DynamicImage, String> {
        let raw = rawler::decode_file(&path).map_err(|err| err.to_string())?;
        let image = rawler::imgop::develop::RawDevelop::default()
            .develop_intermediate(&raw)
            .map_err(|err| err.to_string())?;
        image
            .to_dynamic_image()
            .ok_or_else(|| "The raw file produced no pixels".to_string())
    }));
    match developed {
        Ok(result) => result,
        Err(_) => Err("This raw file could not be developed".into()),
    }
}

fn bits_from_raw(metadata: &rawler::decoders::RawMetadata) -> ExifBits {
    let gps = metadata.exif.gps.as_ref();
    ExifBits {
        orientation: metadata.exif.orientation.unwrap_or(1),
        taken_at: metadata
            .exif
            .date_time_original
            .as_deref()
            .or(metadata.exif.create_date.as_deref())
            .and_then(parse_exif_time),
        camera: camera_name(&metadata.make, &metadata.model),
        gps_lat: gps.and_then(raw_latitude),
        gps_lon: gps.and_then(raw_longitude),
    }
}

fn raw_latitude(gps: &rawler::exif::ExifGPS) -> Option<f64> {
    let degrees = dms_rationals(gps.gps_latitude.as_ref()?)?;
    Some(if gps.gps_latitude_ref.as_deref().is_some_and(|r| r.eq_ignore_ascii_case("S")) {
        -degrees.abs()
    } else {
        degrees
    })
}

fn raw_longitude(gps: &rawler::exif::ExifGPS) -> Option<f64> {
    let degrees = dms_rationals(gps.gps_longitude.as_ref()?)?;
    Some(if gps.gps_longitude_ref.as_deref().is_some_and(|r| r.eq_ignore_ascii_case("W")) {
        -degrees.abs()
    } else {
        degrees
    })
}

fn dms_rationals(values: &[rawler::formats::tiff::Rational; 3]) -> Option<f64> {
    if values.iter().any(|value| value.d == 0) {
        return None;
    }
    Some(f64::from(values[0].as_f32()) + f64::from(values[1].as_f32()) / 60.0 + f64::from(values[2].as_f32()) / 3600.0)
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

fn field_any<'a>(exif: &'a exif::Exif, tag: exif::Tag) -> Option<&'a exif::Field> {
    exif.get_field(tag, exif::In::PRIMARY)
        .or_else(|| exif.fields().find(|field| field.tag == tag))
}

fn gps_degrees(exif: &exif::Exif, tag: exif::Tag, reference: exif::Tag) -> Option<f64> {
    let field = field_any(exif, tag)?;
    let degrees = match &field.value {
        exif::Value::Rational(values) => dms_to_deg(values)?,
        _ => return None,
    };
    let south_or_west = field_any(exif, reference)
        .and_then(|field| match &field.value {
            exif::Value::Ascii(chunks) => chunks.first().and_then(|bytes| bytes.first().copied()),
            _ => None,
        })
        .is_some_and(|byte| matches!(byte, b'S' | b's' | b'W' | b'w'));
    Some(if south_or_west { -degrees.abs() } else { degrees })
}

fn dms_to_deg(values: &[exif::Rational]) -> Option<f64> {
    if values.len() >= 3 {
        Some(values[0].to_f64() + values[1].to_f64() / 60.0 + values[2].to_f64() / 3600.0)
    } else if values.len() == 1 {
        Some(values[0].to_f64())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dms_to_decimal_degrees() {
        let values = [
            exif::Rational { num: 37, denom: 1 },
            exif::Rational { num: 30, denom: 1 },
            exif::Rational { num: 0, denom: 1 },
        ];
        let degrees = dms_to_deg(&values).unwrap();
        assert!((degrees - 37.5).abs() < 1e-9);
    }
}
