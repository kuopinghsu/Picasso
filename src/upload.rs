//! The JPEGs Picasso sends to a WordPress site, in one of two sizes
//! (`docs/TRAVEL.md`). Each is rendered from the editing pipeline, with the
//! photo's edit, crop, and rotation applied first and then made to fit; a
//! smaller photo is never enlarged. The camera original is never sent, and
//! the JPEG carries no metadata at all.
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
const RENDER_VERSION: u32 = 1;

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

/// `source` with its edit and catalog `rotation`, in `size`.
pub fn render(source: &Path, settings: &Settings, rotation: i32, size: UploadSize) -> Result<Rendered, String> {
    let mut rgba = crate::edit::render_edited(source, settings, rotation, Some(size.edge()))?;
    flatten(&mut rgba);
    let bytes = crate::image_io::encode_jpeg(&rgba, size.quality())?;
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
            let wide = render(&landscape, &settings, 0, size).unwrap();
            assert_eq!(wide.width, size.edge());
            assert_eq!((wide.width, wide.height), fitted(3000, 2000, size));
            let tall = render(&portrait, &settings, 0, size).unwrap();
            assert_eq!(tall.height, size.edge());
            assert!(tall.width < tall.height);
            let little = render(&small, &settings, 0, size).unwrap();
            assert_eq!((little.width, little.height), (900, 600), "not enlarged");
        }
        // The catalog's rotation is applied before sending.
        let turned = render(&landscape, &settings, 90, UploadSize::Low).unwrap();
        assert_eq!((turned.width, turned.height), (853, 1280));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_jpeg_carries_no_metadata_and_is_named_by_its_bytes() {
        let dir = temp_dir("bytes");
        let source = photo(&dir, "a.png", 400, 300);
        let rendered = render(&source, &Settings::default(), 0, UploadSize::Standard).unwrap();
        assert_eq!(&rendered.bytes[..2], &[0xFF, 0xD8]);
        let has = |marker: &[u8]| rendered.bytes.windows(marker.len()).any(|window| window == marker);
        assert!(!has(b"Exif") && !has(b"http://ns.adobe.com/xap"), "no EXIF or XMP");
        assert_eq!(rendered.hash.len(), 64);
        let again = render(&source, &Settings::default(), 0, UploadSize::Standard).unwrap();
        assert_eq!(again.hash, rendered.hash);
        let low = render(&source, &Settings::default(), 0, UploadSize::Low).unwrap();
        assert_ne!(low.hash, rendered.hash, "the quality is part of the bytes");
        let _ = std::fs::remove_dir_all(dir);
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
