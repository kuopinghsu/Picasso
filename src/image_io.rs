//! Image decode, EXIF orientation, and thumbnail encoding.

use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, GenericImageView, ImageBuffer, ImageReader, Rgba};

use crate::types::{is_heic_path, is_raw_path, is_video_path, unix_now, EARLIEST_PHOTO};

/// Bytes read from the start of a file to probe it. JPEG EXIF fits in one
/// 64 KB segment and the frame header follows it; HEIC and TIFF-based RAW
/// files keep their boxes and IFDs near the start too.
const HEAD_BYTES: usize = 256 * 1024;

/// The first `limit` bytes of `path` in one read, and the file's length.
fn read_head(path: &Path, limit: usize) -> Result<(Vec<u8>, u64), String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let len = file.metadata().map_err(|err| err.to_string())?.len();
    let mut head = Vec::with_capacity(limit.min(len as usize));
    file.take(limit as u64).read_to_end(&mut head).map_err(|err| err.to_string())?;
    Ok((head, len))
}

pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub taken_at: i64,
    pub camera: String,
    pub lens: String,
    /// Focal length in millimeters, as on a 35 mm camera when the file says;
    /// 0 when unknown.
    pub focal_mm: f32,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub duration: f64,
    /// False when some metadata could not be read yet.
    pub complete: bool,
}

pub fn probe(path: &Path, modified_at: i64) -> Result<Probe, String> {
    if is_video_path(path) {
        return Ok(crate::video::probe_file(path, modified_at));
    }
    if is_heic_path(path) {
        return probe_heic(path, modified_at);
    }
    if is_raw_path(path) {
        return probe_raw(path, modified_at);
    }
    // One read of the head usually answers both questions; the file is only
    // opened again when the head ends before the frame header or the EXIF.
    let (head, len) = read_head(path, HEAD_BYTES)?;
    let whole = head.len() as u64 >= len;
    let dimensions = ImageReader::new(Cursor::new(&head))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok());
    let (width, height) = match dimensions {
        Some(size) => size,
        None => image::image_dimensions(path).map_err(|err| err.to_string())?,
    };
    let exif = match exif::Reader::new().read_from_container(&mut Cursor::new(&head)) {
        Ok(exif) => bits_from_exif(&exif),
        Err(exif::Error::NotFound(_)) => ExifBits::empty(),
        Err(_) if !whole => read_exif(path),
        Err(_) => ExifBits::empty(),
    };
    Ok(exif.probe(width, height, modified_at))
}

pub fn load_oriented(path: &Path) -> Result<DynamicImage, String> {
    if is_heic_path(path) {
        return load_heic(path);
    }
    if is_raw_path(path) {
        return load_raw(path);
    }
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    decode_oriented(path, &bytes)
}

/// Decodes `path` with its long edge at least `edge` where that is cheaper
/// than a full decode, upright. JPEGs are scaled by 1/2, 1/4, or 1/8 inside
/// the decoder, and HEIC uses its embedded thumbnail when it is big enough.
/// The result can still be larger than `edge`.
pub fn load_at_least(path: &Path, edge: u32) -> Result<DynamicImage, String> {
    load_reduced(path, edge, Side::Long)
}

/// [`load_at_least`] with the short edge at least `edge`, for a tile cut
/// square from the middle of the image.
pub fn load_for_tile(path: &Path, edge: u32) -> Result<DynamicImage, String> {
    load_reduced(path, edge, Side::Short)
}

/// The side of a reduced decode that has to reach the edge asked for.
#[derive(Clone, Copy)]
enum Side {
    Long,
    Short,
}

impl Side {
    fn of(self, width: u32, height: u32) -> u32 {
        match self {
            Side::Long => width.max(height),
            Side::Short => width.min(height),
        }
    }
}

fn load_reduced(path: &Path, edge: u32, side: Side) -> Result<DynamicImage, String> {
    if is_raw_path(path) {
        return load_raw(path);
    }
    if is_heic_path(path) {
        // An iPhone's embedded thumbnail is 320 by 240, a little soft for a
        // tile, but decoding the whole photo instead takes many times longer.
        if let Some(image) = heic_thumbnail_from_file(path, edge) {
            return Ok(image);
        }
        let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
        return heic_image(&bytes);
    }
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    if let Some(image) = jpeg_scaled(&bytes, edge, side) {
        return Ok(image);
    }
    decode_oriented(path, &bytes)
}

/// Decodes a whole file already in memory, falling back to the decoder the
/// extension names when the content is not recognized.
fn decode_oriented(path: &Path, bytes: &[u8]) -> Result<DynamicImage, String> {
    let decoded = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.decode().ok());
    let image = match decoded {
        Some(image) => image,
        None => image::open(path).map_err(|err| err.to_string())?,
    };
    let orientation = exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()
        .and_then(|exif| short_field(&exif, exif::Tag::Orientation))
        .unwrap_or(1);
    Ok(apply_orientation(image, orientation))
}

/// A baseline or progressive JPEG decoded straight to the smallest of 1/8,
/// 1/4, or 1/2 size whose `side` still covers `edge`. `None` for other files,
/// CMYK, 16-bit, and images too small to gain from it.
fn jpeg_scaled(bytes: &[u8], edge: u32, side: Side) -> Option<DynamicImage> {
    if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return None;
    }
    let edge = edge.clamp(1, u32::from(u16::MAX)) as u16;
    let decode = std::panic::AssertUnwindSafe(|| {
        let mut decoder = jpeg_decoder::Decoder::new(bytes);
        decoder.read_info().ok()?;
        let info = decoder.info()?;
        if side.of(u32::from(info.width), u32::from(info.height)) / 2 < u32::from(edge) {
            return None;
        }
        // The decoder stops at the first scale where either side reaches
        // what is asked, so the short side alone is asked for it.
        let (want_w, want_h) = match side {
            Side::Short if info.width > info.height => (u16::MAX, edge),
            Side::Short => (edge, u16::MAX),
            Side::Long => (edge, edge),
        };
        let gray = match info.pixel_format {
            jpeg_decoder::PixelFormat::L8 => true,
            jpeg_decoder::PixelFormat::RGB24 => false,
            _ => return None,
        };
        let (w, h) = decoder.scale(want_w, want_h).ok()?;
        let pixels = decoder.decode().ok()?;
        let (w, h) = (u32::from(w), u32::from(h));
        let image = if gray {
            DynamicImage::ImageLuma8(ImageBuffer::from_raw(w, h, pixels)?)
        } else {
            DynamicImage::ImageRgb8(ImageBuffer::from_raw(w, h, pixels)?)
        };
        let orientation = decoder
            .exif_data()
            .and_then(|tiff| exif::Reader::new().read_raw(tiff.to_vec()).ok())
            .and_then(|exif| short_field(&exif, exif::Tag::Orientation))
            .unwrap_or(1);
        Some(apply_orientation(image, orientation))
    });
    std::panic::catch_unwind(decode).ok().flatten()
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

/// `image` shrunk until its short edge is `edge`, since a square tile is cut
/// from its middle, with the long edge held to three times that so a
/// panorama stays small. A smaller image is kept as it is.
pub fn thumbnail_rgba(image: &DynamicImage, edge: u32) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = image.dimensions();
    let (short, long) = (width.min(height).max(1) as f32, width.max(height).max(1) as f32);
    let scale = (edge as f32 / short).min(3.0 * edge as f32 / long);
    if scale >= 1.0 {
        return image.to_rgba8();
    }
    let size = |n: u32| ((n as f32 * scale).round() as u32).max(1);
    image.thumbnail(size(width), size(height)).to_rgba8()
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

/// Encodes in memory and writes the file in one go. The bytes go to a
/// temporary name first, so a reader never sees half a file.
pub fn save_jpeg(path: &Path, rgba: &ImageBuffer<Rgba<u8>, Vec<u8>>, quality: u8) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let encoded = encode_jpeg(rgba, quality)?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, &encoded).map_err(|err| err.to_string())?;
    std::fs::rename(&temporary, path).map_err(|err| {
        let _ = std::fs::remove_file(&temporary);
        err.to_string()
    })
}

/// JPEG bytes of `rgba` with the alpha channel dropped.
pub fn encode_jpeg(rgba: &ImageBuffer<Rgba<u8>, Vec<u8>>, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = DynamicImage::ImageRgba8(rgba.clone()).into_rgb8();
    let mut encoded = Vec::with_capacity(rgb.as_raw().len() / 8);
    JpegEncoder::new_with_quality(&mut encoded, quality)
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
        .map_err(|err| err.to_string())?;
    Ok(encoded)
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
    lens: String,
    focal_mm: f32,
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
        taken_at: [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized, exif::Tag::DateTime]
            .into_iter()
            .find_map(|tag| parse_exif_time(&ascii_field(exif, tag)?)),
        camera: camera_name(make.as_deref().unwrap_or(""), model.as_deref().unwrap_or("")),
        lens: ascii_field(exif, exif::Tag::LensModel).map(|lens| lens.trim().to_string()).unwrap_or_default(),
        focal_mm: short_field(exif, exif::Tag::FocalLengthIn35mmFilm)
            .filter(|mm| *mm > 0)
            .map(f32::from)
            .or_else(|| rational_field(exif, exif::Tag::FocalLength))
            .unwrap_or(0.0),
        gps_lat: gps_degrees(exif, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef),
        gps_lon: gps_degrees(exif, exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef),
    }
}

/// A positive rational EXIF field, such as the focal length.
fn rational_field(exif: &exif::Exif, tag: exif::Tag) -> Option<f32> {
    match &exif.get_field(tag, exif::In::PRIMARY)?.value {
        exif::Value::Rational(values) => values.first().map(|r| r.to_f64() as f32).filter(|value| value.is_finite() && *value > 0.0),
        _ => None,
    }
}

impl ExifBits {
    fn empty() -> Self {
        Self {
            orientation: 1,
            taken_at: None,
            camera: String::new(),
            lens: String::new(),
            focal_mm: 0.0,
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
            lens: self.lens,
            focal_mm: self.focal_mm,
            gps_lat: self.gps_lat,
            gps_lon: self.gps_lon,
            duration: 0.0,
            complete: true,
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

/// Reads the container from the head of the file. The whole file is read
/// only when the head ends before the `meta` box or the EXIF item.
fn probe_heic(path: &Path, modified_at: i64) -> Result<Probe, String> {
    let (head, len) = read_head(path, HEAD_BYTES)?;
    if let Some(probe) = heic_probe_bytes(&head, modified_at) {
        return Ok(probe);
    }
    if !heif_brand(&head) {
        return Err("Not a HEIF image".into());
    }
    let bytes = if head.len() as u64 >= len { head } else { std::fs::read(path).map_err(|err| err.to_string())? };
    let info = heic_rs::probe(&bytes).map_err(|err| err.to_string())?;
    let bits = heic_exif(&bytes).unwrap_or_else(ExifBits::empty);
    Ok(heic_probe(&info, bits, modified_at))
}

/// False when the file's type box lists no HEIF brand, as in a QuickTime
/// movie saved with a .heic name, so the rest of the file need not be read.
/// A file without a readable type box is left to the decoder.
fn heif_brand(head: &[u8]) -> bool {
    const BRANDS: [&[u8]; 12] = [
        b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"hevm", b"hevs", b"mif1", b"msf1", b"miaf", b"avif",
    ];
    if head.len() < 16 || &head[4..8] != b"ftyp" {
        return true;
    }
    let size = (u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize).clamp(16, head.len());
    // The major brand, then compatible brands after the minor version.
    std::iter::once(&head[8..12])
        .chain(head[16..size].as_chunks::<4>().0.iter().map(|brand| brand.as_slice()))
        .any(|brand| BRANDS.contains(&brand))
}

fn heic_probe_bytes(bytes: &[u8], modified_at: i64) -> Option<Probe> {
    let info = heic_rs::probe(bytes).ok()?;
    let bits = heic_exif(bytes)?;
    Some(heic_probe(&info, bits, modified_at))
}

fn heic_probe(info: &heic_rs::ImageInfo, bits: ExifBits, modified_at: i64) -> Probe {
    Probe {
        width: info.width,
        height: info.height,
        taken_at: bits.taken_at.unwrap_or(modified_at),
        camera: bits.camera,
        lens: bits.lens,
        focal_mm: bits.focal_mm,
        gps_lat: bits.gps_lat,
        gps_lon: bits.gps_lon,
        duration: 0.0,
        complete: true,
    }
}

fn load_heic(path: &Path) -> Result<DynamicImage, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    heic_image(&bytes)
}

fn heic_image(bytes: &[u8]) -> Result<DynamicImage, String> {
    let options = heic_rs::DecodeOptions {
        layout: heic_rs::PixelLayout::Rgba8,
        ..heic_rs::DecodeOptions::default()
    };
    let image = heic_rs::decode(bytes, &options).map_err(|err| err.to_string())?;
    let buffer = image::ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(image.width, image.height, image.data)
        .ok_or_else(|| "HEIC decode returned an unexpected pixel buffer".to_string())?;
    Ok(DynamicImage::ImageRgba8(buffer))
}

/// Head read for HEIC; the `meta` box with the item locations comes first and
/// takes a few kilobytes.
const HEIC_HEAD_BYTES: usize = 256 * 1024;

/// [`heic_thumbnail`] after reading only the head of the file and the
/// thumbnail's bytes. A camera HEIC runs to tens of megabytes, and on a hard
/// disk reading all of it takes far longer than decoding the thumbnail. The
/// bytes sit in a zeroed buffer of the file's length, so offsets still hold;
/// the OS hands out those zero pages without touching them.
fn heic_thumbnail_from_file(path: &Path, edge: u32) -> Option<DynamicImage> {
    use std::io::{Seek, SeekFrom};
    let (head, len) = read_head(path, HEIC_HEAD_BYTES).ok()?;
    let len = usize::try_from(len).ok()?;
    if len <= head.len() {
        return heic_thumbnail(&head, edge);
    }
    let ranges = heic_thumbnail_ranges(&head)?;
    let mut sparse = vec![0u8; len];
    sparse[..head.len()].copy_from_slice(&head);
    let mut file = File::open(path).ok()?;
    for (start, length) in ranges {
        let start = usize::try_from(start).ok()?;
        let end = start.checked_add(usize::try_from(length).ok()?)?;
        if end > len {
            return None;
        }
        file.seek(SeekFrom::Start(start as u64)).ok()?;
        file.read_exact(&mut sparse[start..end]).ok()?;
    }
    heic_thumbnail(&sparse, edge)
}

/// File ranges, as start and length, holding the primary's thumbnail item.
/// `None` when the `meta` box isn't within `head` or the item is stored some
/// other way than as plain runs of the file.
fn heic_thumbnail_ranges(head: &[u8]) -> Option<Vec<(u64, u64)>> {
    use heic_rs::meta::iloc::Construction;
    use heic_rs::meta::iref::RefKind;
    let meta = heic_rs::meta::parse(head).ok()?;
    let thumb = meta
        .refs
        .iter()
        .find(|reference| reference.kind == RefKind::Thmb && reference.to.contains(&meta.primary))?
        .from;
    let location = meta.locations.iter().find(|location| location.id == thumb)?;
    if !matches!(location.construction, Construction::File) {
        return None;
    }
    location
        .extents
        .iter()
        .map(|extent| {
            let start = location.base_offset.checked_add(extent.offset)?;
            (extent.length > 0).then_some((start, extent.length))
        })
        .collect()
}

/// The thumbnail item stored beside the primary image, turned like the
/// primary. iPhones store a 320 px HEVC one; cameras such as Hasselblad
/// store a JPEG of several thousand pixels, decoded here at reduced scale,
/// which spares a full decode of a 100-megapixel tiled image. `None` when
/// there is none, it is smaller than `edge`, or its shape does not match.
fn heic_thumbnail(bytes: &[u8], edge: u32) -> Option<DynamicImage> {
    use heic_rs::meta::iref::RefKind;
    use heic_rs::Transform;
    let decode = std::panic::AssertUnwindSafe(|| {
        let context = heic_rs::context::Context::open(bytes).ok()?;
        let primary = context.meta.primary;
        let thumb = context
            .meta
            .refs
            .iter()
            .find(|reference| reference.kind == RefKind::Thmb && reference.to.contains(&primary))?
            .from;
        let props = context.props(thumb).ok()?;
        let data = context.item_data(thumb).ok()?;
        let image = if &context.meta.item(thumb)?.item_type == b"jpeg" {
            let decoded = jpeg_scaled(&data, edge, Side::Long)
                .or_else(|| image::load_from_memory_with_format(&data, image::ImageFormat::Jpeg).ok())?
                .into_rgba8();
            heic_rs::Image {
                width: decoded.width(),
                height: decoded.height(),
                data: decoded.into_raw(),
                layout: heic_rs::PixelLayout::Rgba8,
            }
        } else {
            let hvcc = props.hvcc.as_ref()?;
            let nals = hvcc.split_nals(&data).ok()?;
            let frame = heic_rs::hevc::decode_still(&hvcc.parameter_sets(), &nals).ok()?;
            let nclx = props.nclx.unwrap_or_default();
            heic_rs::color::convert(&frame, None, nclx, heic_rs::PixelLayout::Rgba8, heic_rs::DEFAULT_MAX_PIXELS, Some(1)).ok()?
        };
        // Encoders attach `irot`/`imir` to the thumbnail as well; when one
        // does not, the primary's turn still applies, but not its crop.
        let transforms: Vec<Transform> = if props.transforms.is_empty() {
            let primary_props = context.props(primary).ok()?;
            primary_props.transforms.into_iter().filter(|transform| !matches!(transform, Transform::Crop(_))).collect()
        } else {
            props.transforms.clone()
        };
        let image = heic_rs::transform::apply_all(image, &transforms).ok()?;
        if image.width.max(image.height) < edge {
            return None;
        }
        let info = heic_rs::probe(bytes).ok()?;
        let photo = info.width as f32 / info.height.max(1) as f32;
        let small = image.width as f32 / image.height.max(1) as f32;
        if (photo / small - 1.0).abs() > 0.03 {
            return None;
        }
        let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(image.width, image.height, image.data)?;
        Some(DynamicImage::ImageRgba8(buffer))
    });
    std::panic::catch_unwind(decode).ok().flatten()
}

/// EXIF of the primary item. `None` when the item lies past the end of
/// `bytes`, so a truncated read can ask for more.
fn heic_exif(bytes: &[u8]) -> Option<ExifBits> {
    let context = heic_rs::context::Context::open(bytes).ok()?;
    match context.exif(context.meta.primary) {
        Ok(Some(tiff)) => Some(
            exif::Reader::new()
                .read_raw(tiff.to_vec())
                .map(|exif| bits_from_exif(&exif))
                .unwrap_or_else(|_| ExifBits::empty()),
        ),
        Ok(None) => Some(ExifBits::empty()),
        Err(_) => None,
    }
}

/// Reads RAW dimensions and metadata from the file headers. The decoder runs
/// in dummy mode, so the sensor data stays on disk and a probe takes
/// milliseconds; the slower full decode is only used as a fallback.
fn probe_raw(path: &Path, modified_at: i64) -> Result<Probe, String> {
    // TIFF-based RAW files keep their IFDs and EXIF before the sensor data,
    // so only the head is read. The decoder still checks that the sensor
    // data's range exists, so the head sits in a zeroed buffer of the file's
    // length; the OS hands out those zero pages without touching them.
    if let Ok((head, len)) = read_head(path, RAW_HEAD_BYTES) {
        let source = match usize::try_from(len) {
            Ok(len) if len > head.len() => {
                let mut padded = vec![0u8; len];
                padded[..head.len()].copy_from_slice(&head);
                rawler::rawsource::RawSource::new_from_shared_vec(std::sync::Arc::new(padded))
            }
            _ => rawler::rawsource::RawSource::new_from_slice(&head),
        };
        match std::panic::catch_unwind(|| raw_quick(&source)) {
            Ok(Ok((width, height, bits))) if width > 0 && height > 0 && bits.taken_at.is_some() => {
                return Ok(bits.probe(width as u32, height as u32, modified_at));
            }
            Ok(Err(rawler::RawlerError::Unsupported { .. })) => return probe_unknown_raw(&head, modified_at),
            _ => {}
        }
    }
    let owned = path.to_path_buf();
    let quick = std::panic::catch_unwind(move || {
        let source = rawler::rawsource::RawSource::new(&owned)?;
        raw_quick(&source)
    });
    if let Ok(Ok((width, height, bits))) = quick {
        if width > 0 && height > 0 {
            return Ok(bits.probe(width as u32, height as u32, modified_at));
        }
    }
    let (image, bits) = raw_picture(path)?;
    Ok(bits.probe(image.width(), image.height(), modified_at))
}

/// Head read for RAW probes; maker notes make their IFDs larger than a JPEG's.
const RAW_HEAD_BYTES: usize = 1024 * 1024;

/// Dimensions and metadata with the decoder in dummy mode, so the sensor
/// data is not read.
fn raw_quick(source: &rawler::rawsource::RawSource) -> Result<(usize, usize, ExifBits), rawler::RawlerError> {
    let decoder = rawler::get_decoder(source)?;
    let params = rawler::decoders::RawDecodeParams::default();
    let bits = decoder
        .raw_metadata(source, &params)
        .map(|metadata| bits_from_raw(&metadata))
        .unwrap_or_else(|_| ExifBits::empty());
    let raw = decoder.raw_image(source, &params, true)?;
    let (width, height) = match raw.crop_area {
        Some(area) if area.d.w > 0 && area.d.h > 0 => (area.d.w, area.d.h),
        _ => (raw.width, raw.height),
    };
    Ok((width, height, bits))
}

fn load_raw(path: &Path) -> Result<DynamicImage, String> {
    let (image, bits) = raw_picture(path)?;
    Ok(apply_orientation(image, bits.orientation))
}

fn raw_picture(path: &Path) -> Result<(DynamicImage, ExifBits), String> {
    let source = rawler::rawsource::RawSource::new(path).map_err(|err| err.to_string())?;
    let decoder = match rawler::get_decoder(&source) {
        Ok(decoder) => decoder,
        Err(rawler::RawlerError::Unsupported { .. }) => return unknown_raw_picture(path),
        Err(err) => return Err(err.to_string()),
    };
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

/// Probes a TIFF-based RAW file from a camera the decoder doesn't know, from
/// its directories and EXIF alone.
fn probe_unknown_raw(head: &[u8], modified_at: i64) -> Result<Probe, String> {
    let layout = crate::raw_tiff::layout(head).ok_or("This camera's raw files are not supported yet")?;
    let (width, height) = layout.photo_size().ok_or("This camera's raw files are not supported yet")?;
    Ok(raw_head_exif(head).probe(width, height, modified_at))
}

/// The preview JPEG a camera the decoder doesn't know embeds in its RAW
/// files, upright.
fn unknown_raw_picture(path: &Path) -> Result<(DynamicImage, ExifBits), String> {
    let unsupported = || "This camera's raw files are not supported yet".to_string();
    let (head, _) = read_head(path, RAW_HEAD_BYTES)?;
    let preview = crate::raw_tiff::layout(&head).and_then(|layout| layout.preview).ok_or_else(unsupported)?;
    let jpeg = crate::raw_tiff::read_preview(path, &preview).ok_or_else(unsupported)?;
    let image = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).map_err(|err| err.to_string())?;
    let bits = raw_head_exif(&head);
    Ok((apply_orientation(image, bits.orientation), bits))
}

/// EXIF from the start of a TIFF-based RAW file.
fn raw_head_exif(head: &[u8]) -> ExifBits {
    exif::Reader::new()
        .read_raw(head.to_vec())
        .map(|exif| bits_from_exif(&exif))
        .unwrap_or_else(|_| ExifBits::empty())
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
        taken_at: [&metadata.exif.date_time_original, &metadata.exif.create_date]
            .into_iter()
            .find_map(|date| parse_exif_time(date.as_deref()?)),
        camera: camera_name(&metadata.make, &metadata.model),
        lens: metadata.exif.lens_model.as_deref().map(str::trim).unwrap_or_default().to_string(),
        focal_mm: metadata
            .exif
            .focal_length
            .as_ref()
            .filter(|r| r.d != 0)
            .map(|r| r.n as f32 / r.d as f32)
            .filter(|mm| mm.is_finite() && *mm > 0.0)
            .unwrap_or(0.0),
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

/// Reads an EXIF date such as `2025:11:03 06:06:57` as local time. Dates
/// older than [`EARLIEST_PHOTO`] are placeholders, such as the
/// `0001:01:01 00:00:00` some photo services write, and read as none.
fn parse_exif_time(text: &str) -> Option<i64> {
    let naive = chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y:%m:%d %H:%M:%S").ok()?;
    let stamp = naive
        .and_local_timezone(chrono::Local)
        .single()
        .map_or_else(|| naive.and_utc().timestamp(), |dt| dt.timestamp());
    (stamp >= EARLIEST_PHOTO).then_some(stamp)
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
    fn heic_names_on_other_files_are_caught_from_the_type_box() {
        let ftyp = |major: &[u8], compatible: &[&[u8]]| {
            let mut box_ = ((16 + 4 * compatible.len()) as u32).to_be_bytes().to_vec();
            box_.extend_from_slice(b"ftyp");
            box_.extend_from_slice(major);
            box_.extend_from_slice(&[0; 4]);
            for brand in compatible {
                box_.extend_from_slice(brand);
            }
            box_.extend_from_slice(&[0; 8]);
            box_
        };
        assert!(heif_brand(&ftyp(b"heic", &[b"mif1", b"heic"])));
        assert!(heif_brand(&ftyp(b"isom", &[b"mif1"])), "a compatible HEIF brand counts");
        assert!(!heif_brand(&ftyp(b"qt  ", &[b"qt  "])), "a QuickTime movie");
        assert!(!heif_brand(&ftyp(b"isom", &[b"iso2", b"mp41"])), "an MP4 video");
        assert!(heif_brand(b"no type box here at all"), "unknown files go to the decoder");
    }

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

    #[test]
    fn placeholder_exif_dates_read_as_none() {
        assert!(parse_exif_time("2025:11:03 06:06:57").is_some());
        assert!(parse_exif_time("1890:05:01 12:00:00").is_some());
        assert_eq!(parse_exif_time("0001:01:01 00:00:00"), None);
        assert_eq!(parse_exif_time("0000:00:00 00:00:00"), None);
        assert_eq!(parse_exif_time("    :  :     :  :  "), None);
        let earliest = chrono::NaiveDate::from_ymd_opt(1826, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
        assert_eq!(earliest.and_utc().timestamp(), EARLIEST_PHOTO);
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let rgb = image::RgbImage::from_fn(width, height, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90]));
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 85)
            .encode(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    #[test]
    fn jpeg_decodes_at_the_smallest_scale_that_covers_the_edge() {
        let bytes = jpeg(2048, 1024);
        let image = jpeg_scaled(&bytes, 256, Side::Long).unwrap();
        assert_eq!(image.dimensions(), (256, 128));
        let image = jpeg_scaled(&bytes, 300, Side::Long).unwrap();
        assert_eq!(image.dimensions(), (512, 256));
        let image = jpeg_scaled(&bytes, 200, Side::Short).unwrap();
        assert_eq!(image.dimensions(), (512, 256));
        let tall = jpeg(1024, 2048);
        assert_eq!(jpeg_scaled(&tall, 200, Side::Short).unwrap().dimensions(), (256, 512));
        // Less than half the size: a full decode is as fast.
        assert!(jpeg_scaled(&bytes, 1500, Side::Long).is_none());
        assert!(jpeg_scaled(&bytes, 600, Side::Short).is_none());
        assert!(jpeg_scaled(b"not a jpeg", 256, Side::Long).is_none());
    }

    #[test]
    fn probe_and_save_round_trip_through_the_head() {
        let dir = std::env::temp_dir().join(format!("picasso-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let source = dir.join("photo.jpg");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, jpeg(640, 480)).unwrap();
        let probe = probe(&source, 42).unwrap();
        assert_eq!((probe.width, probe.height, probe.taken_at), (640, 480, 42));

        let thumb = thumbnail_rgba(&load_for_tile(&source, 256).unwrap(), 256);
        let target = dir.join("thumbs").join("1.jpg");
        save_jpeg(&target, &thumb, 80).unwrap();
        let names: Vec<_> = std::fs::read_dir(target.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, ["1.jpg"]);
        assert_eq!(image::image_dimensions(&target).unwrap(), (341, 256));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thumbnails_keep_the_short_edge_a_tile_needs() {
        let size = |w, h, edge| thumbnail_rgba(&DynamicImage::new_rgb8(w, h), edge).dimensions();
        assert_eq!(size(6000, 4000, 320), (480, 320));
        assert_eq!(size(4000, 6000, 320), (320, 480));
        assert_eq!(size(12000, 1000, 320), (960, 80), "a panorama's long edge stops at three times the edge");
        assert_eq!(size(200, 150, 320), (200, 150), "never enlarged");
    }
}
