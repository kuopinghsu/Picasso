//! The JPEGs Picasso sends to a WordPress site, in one of two sizes
//! (`docs/TRAVEL.md`). Each is rendered from the editing pipeline, with the
//! photo's edit, crop, and rotation applied first and then made to fit; a
//! smaller photo is never enlarged. The camera original is never sent. When
//! asked, the JPEG keeps how the photo was taken ([`KEPT`]): the camera,
//! lens, exposure, and time, never its position, serial numbers, or owner.
//! Otherwise it carries no metadata at all.
//!
//! Publishing to a site uses it; until then only its tests do.
#![cfg_attr(not(test), allow(dead_code))]

use std::path::Path;

use image::{Rgba, RgbaImage};

use crate::edit::Settings;

/// How large the uploaded photos are. Choosing one never permits an upload
/// by itself, and there is no upload of originals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum UploadSize {
    /// Longest side at most 2048 pixels, JPEG quality 85.
    #[default]
    Standard,
    /// Longest side at most 1280 pixels, JPEG quality 80.
    Low,
}

impl UploadSize {
    pub const ALL: [UploadSize; 2] = [UploadSize::Standard, UploadSize::Low];

    /// The longest side, in pixels.
    pub fn edge(self) -> u32 {
        match self {
            UploadSize::Standard => 2048,
            UploadSize::Low => 1280,
        }
    }

    /// The JPEG encoder's quality setting, 1…100. It is a setting of the
    /// encoder, not a share of the original's quality.
    pub fn quality(self) -> u8 {
        match self {
            UploadSize::Standard => 85,
            UploadSize::Low => 80,
        }
    }

    /// How it is saved with a Story and its jobs.
    pub fn code(self) -> &'static str {
        match self {
            UploadSize::Standard => "standard",
            UploadSize::Low => "low",
        }
    }

    pub fn from_code(code: &str) -> Option<UploadSize> {
        UploadSize::ALL.into_iter().find(|size| size.code() == code)
    }

    /// Everything that decides the bytes, for keys of rendered photos: a
    /// photo rendered for one size is never taken for another.
    pub fn key(self) -> String {
        format!("jpeg-{}-q{}-flat-white-v{}", self.edge(), self.quality(), RENDER_VERSION)
    }
}

/// Raised when the same photo and size would come out differently, so
/// photos rendered before aren't taken for the new ones.
const RENDER_VERSION: u32 = 2;

/// Transparent pixels are laid on this before encoding, as JPEG has no
/// transparency: white, as the photo shows on a light page.
const BACKGROUND: [u8; 3] = [255, 255, 255];

/// One photo, ready to upload.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// SHA-256 of `bytes`, in hex: an upload confirmed with these bytes
    /// can be reused, and no other.
    pub hash: String,
    pub size: UploadSize,
}

/// `source` with its edit and catalog `rotation`, in `size`, with the
/// [`KEPT`] EXIF fields when `exif`.
pub fn render(source: &Path, settings: &Settings, rotation: i32, size: UploadSize, exif: bool) -> Result<Rendered, String> {
    let mut rgba = crate::edit::render_edited(source, settings, rotation, Some(size.edge()))?;
    flatten(&mut rgba);
    let bytes = crate::image_io::encode_jpeg(&rgba, size.quality())?;
    let bytes = match crate::photo_info::read_exif(source).filter(|_| exif) {
        Some(exif) => with_exif(bytes, &exif),
        None => bytes,
    };
    let hash = hash(&bytes);
    Ok(Rendered { bytes, width: rgba.width(), height: rgba.height(), hash, size })
}

/// The size a photo of `width` × `height` comes out at, before rendering
/// it: smaller photos keep theirs.
pub fn fitted(width: u32, height: u32, size: UploadSize) -> (u32, u32) {
    let long = width.max(height);
    if long <= size.edge() || long == 0 {
        return (width, height);
    }
    let scale = size.edge() as f32 / long as f32;
    (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1))
}

/// About how many bytes photos of these dimensions take in `size`, before
/// they are rendered. Only an estimate, shown as one; the rendered bytes
/// replace it before anything is sent.
pub fn estimated_bytes(dimensions: &[(u32, u32)], size: UploadSize) -> u64 {
    // Bits per pixel of photos at these qualities: about 2.4 at 80, 2.8 at 85.
    let bits_x10: u64 = match size {
        UploadSize::Standard => 28,
        UploadSize::Low => 24,
    };
    dimensions
        .iter()
        .map(|&(w, h)| {
            let (w, h) = fitted(w, h, size);
            u64::from(w) * u64::from(h) * bits_x10 / 80
        })
        .sum()
}

/// The EXIF fields an uploaded photo keeps: how it was taken, for a page
/// to show beside it. Not the GPS position, the camera's or lens's serial
/// number, the owner or artist, the maker notes, or the orientation, which
/// the rendering has already applied.
const KEPT: [exif::Tag; 13] = [
    exif::Tag::Make,
    exif::Tag::Model,
    exif::Tag::LensMake,
    exif::Tag::LensModel,
    exif::Tag::FocalLength,
    exif::Tag::FocalLengthIn35mmFilm,
    exif::Tag::FNumber,
    exif::Tag::ExposureTime,
    exif::Tag::PhotographicSensitivity,
    exif::Tag::ExposureBiasValue,
    exif::Tag::DateTimeOriginal,
    exif::Tag::OffsetTimeOriginal,
    exif::Tag::Flash,
];

/// `jpeg` with the [`KEPT`] fields of `exif` in an APP1 segment after its
/// start marker. Without any of them, or if they can't be written, `jpeg`
/// as it was.
fn with_exif(jpeg: Vec<u8>, exif: &exif::Exif) -> Vec<u8> {
    let fields: Vec<&exif::Field> = KEPT.iter().filter_map(|&tag| exif.get_field(tag, exif::In::PRIMARY)).collect();
    if fields.is_empty() || !jpeg.starts_with(&[0xFF, 0xD8]) {
        return jpeg;
    }
    let mut writer = exif::experimental::Writer::new();
    for field in &fields {
        writer.push_field(field);
    }
    let mut tiff = std::io::Cursor::new(Vec::new());
    if writer.write(&mut tiff, false).is_err() {
        return jpeg;
    }
    let tiff = tiff.into_inner();
    // The segment's length counts itself and "Exif\0\0", and fits in 16 bits.
    let Ok(length) = u16::try_from(2 + 6 + tiff.len()) else {
        return jpeg;
    };
    let mut out = Vec::with_capacity(jpeg.len() + usize::from(length) + 2);
    out.extend_from_slice(&jpeg[..2]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&tiff);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// Whether the file at `path` can hold transparent pixels, read from its
/// header alone: only formats that keep transparency are looked at.
pub fn has_transparency(path: &Path) -> bool {
    let kind = path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !matches!(kind.as_str(), "png" | "webp" | "gif" | "tif" | "tiff") {
        return false;
    }
    image::ImageReader::open(path)
        .ok()
        .and_then(|reader| reader.with_guessed_format().ok())
        .and_then(|reader| reader.into_decoder().ok())
        .is_some_and(|decoder| image::ImageDecoder::color_type(&decoder).has_alpha())
}

/// Lays transparent pixels on [`BACKGROUND`], as the page would show them.
fn flatten(rgba: &mut RgbaImage) {
    for Rgba(pixel) in rgba.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        if alpha == 255 {
            continue;
        }
        for (channel, background) in pixel.iter_mut().take(3).zip(BACKGROUND) {
            *channel = ((u32::from(*channel) * alpha + u32::from(background) * (255 - alpha) + 127) / 255) as u8;
        }
        pixel[3] = 255;
    }
}

/// SHA-256 of `bytes`, in hex, as an upload is known by.
pub fn hash(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_format_with_transparency_is_said_to_have_it() {
        let dir = std::env::temp_dir().join(format!("picasso-alpha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clear = dir.join("clear.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0])).save(&clear).unwrap();
        let solid = dir.join("solid.png");
        image::RgbImage::from_pixel(4, 4, image::Rgb([9, 9, 9])).save(&solid).unwrap();
        let jpeg = dir.join("photo.jpg");
        image::RgbImage::from_pixel(4, 4, image::Rgb([9, 9, 9])).save(&jpeg).unwrap();
        assert!(super::has_transparency(&clear));
        assert!(!super::has_transparency(&solid) && !super::has_transparency(&jpeg));
        let _ = std::fs::remove_dir_all(dir);
    }

    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso-upload-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn photo(dir: &Path, name: &str, width: u32, height: u32) -> std::path::PathBuf {
        let path = dir.join(name);
        let image = RgbaImage::from_fn(width, height, |x, y| Rgba([(x % 256) as u8, (y % 256) as u8, 120, 255]));
        image.save(&path).unwrap();
        path
    }

    #[test]
    fn the_two_sizes_are_what_they_promise() {
        assert_eq!((UploadSize::Standard.edge(), UploadSize::Standard.quality()), (2048, 85));
        assert_eq!((UploadSize::Low.edge(), UploadSize::Low.quality()), (1280, 80));
        assert_eq!(UploadSize::default(), UploadSize::Standard);
        for size in UploadSize::ALL {
            assert_eq!(UploadSize::from_code(size.code()), Some(size));
        }
        assert_ne!(UploadSize::Standard.key(), UploadSize::Low.key(), "a large render is never reused for a small one");
    }

    #[test]
    fn photos_fit_inside_the_size_and_are_never_enlarged() {
        let dir = temp_dir("fit");
        let landscape = photo(&dir, "wide.png", 3000, 2000);
        let portrait = photo(&dir, "tall.png", 1500, 2600);
        let small = photo(&dir, "small.png", 900, 600);
        let settings = Settings::default();
        for size in UploadSize::ALL {
            let wide = render(&landscape, &settings, 0, size, true).unwrap();
            assert_eq!(wide.width, size.edge());
            assert_eq!((wide.width, wide.height), fitted(3000, 2000, size));
            let tall = render(&portrait, &settings, 0, size, true).unwrap();
            assert_eq!(tall.height, size.edge());
            assert!(tall.width < tall.height);
            let little = render(&small, &settings, 0, size, true).unwrap();
            assert_eq!((little.width, little.height), (900, 600), "not enlarged");
        }
        // The catalog's rotation is applied before sending.
        let turned = render(&landscape, &settings, 90, UploadSize::Low, true).unwrap();
        assert_eq!((turned.width, turned.height), (853, 1280));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_photo_without_exif_is_sent_without_it_and_is_named_by_its_bytes() {
        let dir = temp_dir("bytes");
        let source = photo(&dir, "a.png", 400, 300);
        let rendered = render(&source, &Settings::default(), 0, UploadSize::Standard, true).unwrap();
        assert_eq!(&rendered.bytes[..2], &[0xFF, 0xD8]);
        let has = |marker: &[u8]| rendered.bytes.windows(marker.len()).any(|window| window == marker);
        assert!(!has(b"Exif") && !has(b"http://ns.adobe.com/xap"), "no EXIF or XMP");
        assert_eq!(rendered.hash.len(), 64);
        let again = render(&source, &Settings::default(), 0, UploadSize::Standard, true).unwrap();
        assert_eq!(again.hash, rendered.hash);
        let low = render(&source, &Settings::default(), 0, UploadSize::Low, true).unwrap();
        assert_ne!(low.hash, rendered.hash, "the quality is part of the bytes");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_how_the_photo_was_taken_is_kept() {
        use exif::{Field, In, Tag, Value};
        let field = |tag, value| Field { tag, ifd_num: In::PRIMARY, value };
        let ascii = |text: &str| Value::Ascii(vec![text.as_bytes().to_vec()]);
        let source = [
            field(Tag::Make, ascii("FUJIFILM")),
            field(Tag::Model, ascii("X-T5")),
            field(Tag::FNumber, Value::Rational(vec![(56, 10).into()])),
            field(Tag::ExposureTime, Value::Rational(vec![(1, 250).into()])),
            field(Tag::PhotographicSensitivity, Value::Short(vec![400])),
            field(Tag::DateTimeOriginal, ascii("2025:07:10 18:42:05")),
            field(Tag::BodySerialNumber, ascii("SERIAL-123")),
            field(Tag::Artist, ascii("Someone")),
            field(Tag::GPSLatitudeRef, ascii("N")),
            field(Tag::GPSLatitude, Value::Rational(vec![(37, 1).into(), (10, 1).into(), (0, 1).into()])),
        ];
        let mut writer = exif::experimental::Writer::new();
        source.iter().for_each(|field| writer.push_field(field));
        let mut tiff = std::io::Cursor::new(Vec::new());
        writer.write(&mut tiff, false).unwrap();
        let exif = exif::Reader::new().read_raw(tiff.into_inner()).unwrap();

        let jpeg = crate::image_io::encode_jpeg(&RgbaImage::from_pixel(8, 8, Rgba([90, 120, 150, 255])), 85).unwrap();
        let sent = with_exif(jpeg.clone(), &exif);
        assert_eq!(&sent[..4], &[0xFF, 0xD8, 0xFF, 0xE1]);
        assert!(image::load_from_memory(&sent).is_ok(), "still a JPEG");
        let kept = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(&sent)).unwrap();
        let text = |tag| kept.get_field(tag, In::PRIMARY).map(|f| f.display_value().to_string());
        assert_eq!(text(Tag::Model).as_deref(), Some("\"X-T5\""));
        assert_eq!(text(Tag::PhotographicSensitivity).as_deref(), Some("400"));
        assert!(text(Tag::FNumber).is_some() && text(Tag::ExposureTime).is_some() && text(Tag::DateTimeOriginal).is_some());
        for gone in [Tag::BodySerialNumber, Tag::Artist, Tag::GPSLatitude, Tag::GPSLatitudeRef] {
            assert!(text(gone).is_none(), "{gone} is not sent");
        }
        let has = |marker: &[u8]| sent.windows(marker.len()).any(|window| window == marker);
        assert!(!has(b"SERIAL-123") && !has(b"Someone"));

        // Nothing worth keeping: the JPEG as it was.
        let mut writer = exif::experimental::Writer::new();
        writer.push_field(&source[6]);
        let mut tiff = std::io::Cursor::new(Vec::new());
        writer.write(&mut tiff, false).unwrap();
        let serial_only = exif::Reader::new().read_raw(tiff.into_inner()).unwrap();
        assert_eq!(with_exif(jpeg.clone(), &serial_only), jpeg);
    }

    #[test]
    fn transparent_pixels_come_out_white() {
        let mut image = RgbaImage::from_pixel(2, 1, Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 0, Rgba([0, 0, 0, 128]));
        flatten(&mut image);
        assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [127, 127, 127, 255]);
    }

    #[test]
    fn the_estimate_follows_the_size_and_leaves_small_photos_alone() {
        let standard = estimated_bytes(&[(6000, 4000)], UploadSize::Standard);
        let low = estimated_bytes(&[(6000, 4000)], UploadSize::Low);
        assert!(low < standard / 2, "{low} {standard}");
        assert_eq!(estimated_bytes(&[(800, 600)], UploadSize::Low) * 28, estimated_bytes(&[(800, 600)], UploadSize::Standard) * 24);
    }
}
