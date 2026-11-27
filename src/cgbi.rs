//! Apple's "CgBI" PNGs, which older iPhones and iOS apps saved: a `CgBI`
//! chunk before the header, image data compressed without the zlib wrapper,
//! and pixels stored blue first with their colors already multiplied by
//! their alpha. Other PNG readers refuse them, so they are rewritten as a
//! standard PNG in memory, decoded, and their colors put back.

use std::io::{Read, Write};

use image::DynamicImage;

const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Whether `bytes` start like an Apple CgBI PNG.
pub fn is_cgbi(bytes: &[u8]) -> bool {
    bytes.starts_with(SIGNATURE) && bytes.get(12..16) == Some(b"CgBI")
}

/// Width and height from the header, which needs only the file's start.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !is_cgbi(bytes) {
        return None;
    }
    let header = chunks(bytes).find(|(kind, _)| kind == b"IHDR")?.1;
    let number = |at: usize| Some(u32::from_be_bytes(header.get(at..at + 4)?.try_into().ok()?));
    Some((number(0)?, number(4)?))
}

/// Whether a CgBI PNG can have transparent pixels: an alpha channel, or a
/// `tRNS` chunk, which comes before the image data. Read from the file's
/// start; `None` for any other file.
pub fn has_alpha(bytes: &[u8]) -> Option<bool> {
    if !is_cgbi(bytes) {
        return None;
    }
    let mut alpha = false;
    for (kind, data) in chunks(bytes) {
        match &kind {
            b"IHDR" => alpha |= matches!(data.get(9), Some(4 | 6)),
            b"tRNS" => alpha = true,
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
    }
    Some(alpha)
}

/// Decodes a whole CgBI PNG.
pub fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
    let png = standard_png(bytes)?;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|err| err.to_string())?;
    Ok(match image {
        DynamicImage::ImageRgba8(mut rgba) => {
            for pixel in rgba.pixels_mut() {
                let [blue, green, red, alpha] = pixel.0;
                pixel.0 = [
                    unmultiply(red, alpha),
                    unmultiply(green, alpha),
                    unmultiply(blue, alpha),
                    alpha,
                ];
            }
            DynamicImage::ImageRgba8(rgba)
        }
        DynamicImage::ImageRgb8(mut rgb) => {
            for pixel in rgb.pixels_mut() {
                pixel.0.swap(0, 2);
            }
            DynamicImage::ImageRgb8(rgb)
        }
        // Gray and palette images keep their order.
        other => other,
    })
}

/// A color multiplied by `alpha`, divided back out.
fn unmultiply(color: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    let value = (u32::from(color) * 255 + u32::from(alpha) / 2) / u32::from(alpha);
    value.min(255) as u8
}

/// The chunks of a PNG after its signature, as type and data, until one
/// runs past the end.
fn chunks(bytes: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut at = SIGNATURE.len();
    std::iter::from_fn(move || {
        let length = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = bytes.get(at + 4..at + 8)?.try_into().ok()?;
        let data = bytes.get(at + 8..(at + 8).checked_add(length)?)?;
        at += 12 + length;
        Some((kind, data))
    })
}

/// The same image as a PNG any reader takes: without the `CgBI` chunk, and
/// with its image data inflated and compressed again with the zlib wrapper.
fn standard_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut deflated = Vec::new();
    let mut kept = Vec::new();
    for (kind, data) in chunks(bytes) {
        match &kind {
            b"IDAT" => deflated.extend_from_slice(data),
            b"IHDR" | b"PLTE" | b"tRNS" => kept.push((kind, data)),
            b"IEND" => break,
            _ => {}
        }
    }
    if kept.first().map(|(kind, _)| kind) != Some(b"IHDR") || deflated.is_empty() {
        return Err("The PNG has no image header or data".into());
    }
    let limit = raw_size(kept[0].1).ok_or("The PNG's image header is damaged")?;
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(deflated.as_slice())
        .take(limit + 1)
        .read_to_end(&mut raw)
        .map_err(|err| err.to_string())?;
    if raw.len() as u64 > limit {
        return Err("The PNG holds more image data than its size allows".into());
    }
    let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    zlib.write_all(&raw).map_err(|err| err.to_string())?;
    let data = zlib.finish().map_err(|err| err.to_string())?;

    let mut png = SIGNATURE.to_vec();
    for (kind, data) in &kept {
        write_chunk(&mut png, kind, data);
    }
    write_chunk(&mut png, b"IDAT", &data);
    write_chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

/// The most bytes the image data of header `ihdr` can inflate to: a filter
/// byte and the pixels of each row, with room for the shorter rows of an
/// interlaced image. `None` for a header that is damaged or too large.
fn raw_size(ihdr: &[u8]) -> Option<u64> {
    let number = |at: usize| {
        Some(u64::from(u32::from_be_bytes(
            ihdr.get(at..at + 4)?.try_into().ok()?,
        )))
    };
    let (width, height) = (number(0)?, number(4)?);
    let depth = u64::from(*ihdr.get(8)?);
    let channels = match ihdr.get(9)? {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    let row = width.checked_mul(channels * depth)?.div_ceil(8) + 1;
    // Interlacing's seven passes add at most a filter byte and a partly
    // filled byte to each of their rows.
    let size = row
        .checked_mul(height)?
        .checked_add(height.checked_mul(14)?)?;
    // As much as the PNG decoder itself allows for an image.
    (size <= 512 * 1024 * 1024).then_some(size)
}

fn write_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    png.extend_from_slice(&crc.sum().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CgBI PNG as Apple writes one: 2 by 1 pixels, opaque red and
    /// half-transparent green, blue first and premultiplied.
    fn sample() -> Vec<u8> {
        let rows = [0u8, 0, 0, 255, 255, 0, 64, 0, 128];
        let mut deflate =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        deflate.write_all(&rows).unwrap();
        let mut header = Vec::new();
        header.extend_from_slice(&2u32.to_be_bytes());
        header.extend_from_slice(&1u32.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut png = SIGNATURE.to_vec();
        write_chunk(&mut png, b"CgBI", &[0x50, 0x00, 0x20, 0x02]);
        write_chunk(&mut png, b"IHDR", &header);
        write_chunk(&mut png, b"IDAT", &deflate.finish().unwrap());
        write_chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn an_apple_cgbi_png_decodes_with_its_colors_put_back() {
        let png = sample();
        assert!(
            image::load_from_memory(&png).is_err(),
            "other readers refuse it"
        );
        assert!(is_cgbi(&png));
        assert!(!is_cgbi(&standard_png(&png).unwrap()));
        assert_eq!(dimensions(&png), Some((2, 1)));
        assert_eq!(has_alpha(&png), Some(true));
        assert_eq!(has_alpha(&standard_png(&png).unwrap()), None);

        // An RGB one is opaque unless a tRNS chunk names a transparent color.
        // The signature and CgBI chunk take 24 bytes, IHDR's data starts at 32.
        let mut rgb = png[..24].to_vec();
        let mut header = png[32..32 + 13].to_vec();
        header[9] = 2;
        write_chunk(&mut rgb, b"IHDR", &header);
        assert_eq!(has_alpha(&rgb), Some(false));
        write_chunk(&mut rgb, b"tRNS", &[0, 0, 0, 0, 0, 0]);
        assert_eq!(has_alpha(&rgb), Some(true));
        let rgba = decode(&png).unwrap().to_rgba8();
        assert_eq!(rgba.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(rgba.get_pixel(1, 0).0, [0, 128, 0, 128]);
    }

    #[test]
    fn a_cgbi_png_inflating_past_its_size_is_refused() {
        let mut png = sample();
        // The header says 2 by 1 pixels; the data holds 1,000 rows of them.
        let at = png.windows(4).position(|kind| kind == b"IDAT").unwrap() - 4;
        png.truncate(at);
        let mut deflate =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        deflate.write_all(&[0u8; 9 * 1000]).unwrap();
        write_chunk(&mut png, b"IDAT", &deflate.finish().unwrap());
        write_chunk(&mut png, b"IEND", &[]);
        assert!(decode(&png).unwrap_err().contains("more image data"));
        assert_eq!(
            raw_size(&[0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0, 0, 0]),
            Some(9 + 14)
        );
        assert_eq!(raw_size(&[0xFF; 13]), None);
    }

    #[test]
    fn a_cut_short_cgbi_png_is_an_error_not_a_panic() {
        let png = sample();
        for end in [8, 20, 40, png.len() - 20] {
            let _ = dimensions(&png[..end]);
            assert!(decode(&png[..end]).is_err(), "cut at {end}");
        }
    }
}
