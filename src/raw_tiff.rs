//! TIFF-based camera RAW files from cameras the RAW decoder doesn't know
//! yet, such as the Hasselblad X2D II 100C. The image sizes and the preview
//! JPEG the camera embeds are found by walking the TIFF directories, so the
//! sensor data is never read.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Directories followed before giving up on a file, against loops.
const MAX_IFDS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preview {
    pub offset: u64,
    pub length: u64,
    /// Zero when the directory doesn't say.
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// Size of the largest image stored as sensor data.
    pub sensor: Option<(u32, u32)>,
    /// The largest embedded JPEG.
    pub preview: Option<Preview>,
}

impl Layout {
    /// Size of the developed photo: the preview's shape at the sensor's
    /// resolution. The preview is cropped like the final photo, while the
    /// sensor keeps a few extra pixels at the edges.
    pub fn photo_size(&self) -> Option<(u32, u32)> {
        let preview = self.preview.filter(|preview| preview.width > 0 && preview.height > 0);
        match (self.sensor, preview) {
            (Some((sw, sh)), Some(preview)) => {
                let aspect = preview.width as f64 / preview.height as f64;
                if sw as f64 / sh as f64 > aspect {
                    Some(((sh as f64 * aspect).round() as u32, sh))
                } else {
                    Some((sw, (sw as f64 / aspect).round() as u32))
                }
            }
            (Some(size), None) => Some(size),
            (None, Some(preview)) => Some((preview.width, preview.height)),
            (None, None) => None,
        }
    }
}

/// Reads the directories in `head`, the start of a TIFF file. Directories
/// beyond the head are skipped.
pub fn layout(head: &[u8]) -> Option<Layout> {
    let tiff = Tiff::new(head)?;
    let mut layout = Layout::default();
    let mut pending = vec![tiff.u32_at(4)? as usize];
    let mut visited = Vec::new();
    while let Some(offset) = pending.pop() {
        if offset == 0 || visited.contains(&offset) || visited.len() >= MAX_IFDS {
            continue;
        }
        visited.push(offset);
        let Some(ifd) = tiff.ifd(offset) else { continue };
        pending.extend(ifd.sub_ifds.iter().map(|&sub| sub as usize));
        if ifd.next != 0 {
            pending.push(ifd.next as usize);
        }
        let area = |(w, h): (u32, u32)| u64::from(w) * u64::from(h);
        if matches!(ifd.photometric, Some(32803 | 34892)) && ifd.width > 0 && ifd.height > 0 {
            let size = (ifd.width, ifd.height);
            if layout.sensor.is_none_or(|known| area(size) > area(known)) {
                layout.sensor = Some(size);
            }
        }
        if let Some(preview) = ifd.preview() {
            let bigger = layout.preview.is_none_or(|known| {
                area((preview.width, preview.height)) > area((known.width, known.height))
                    || (preview.width == 0 && preview.length > known.length)
            });
            if bigger {
                layout.preview = Some(preview);
            }
        }
    }
    (layout.sensor.is_some() || layout.preview.is_some()).then_some(layout)
}

/// The bytes of `preview` in `path`, when they hold a JPEG.
pub fn read_preview(path: &Path, preview: &Preview) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(preview.offset)).ok()?;
    let mut bytes = Vec::with_capacity(preview.length as usize);
    file.take(preview.length).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 == preview.length && bytes.starts_with(&[0xFF, 0xD8])).then_some(bytes)
}

struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

#[derive(Default)]
struct Ifd {
    width: u32,
    height: u32,
    compression: u32,
    photometric: Option<u32>,
    strip: Option<(u32, u32)>,
    jpeg: Option<(u32, u32)>,
    sub_ifds: Vec<u32>,
    next: u32,
}

impl Ifd {
    fn preview(&self) -> Option<Preview> {
        let (offset, length) = match (self.jpeg, self.strip) {
            (Some(jpeg), _) => jpeg,
            (None, Some(strip)) if matches!(self.compression, 6 | 7) && self.photometric != Some(32803) => strip,
            _ => return None,
        };
        (length > 0).then_some(Preview {
            offset: u64::from(offset),
            length: u64::from(length),
            width: self.width,
            height: self.height,
        })
    }
}

impl<'a> Tiff<'a> {
    fn new(data: &'a [u8]) -> Option<Self> {
        let little = match data.get(..4)? {
            [b'I', b'I', 42, 0] => true,
            [b'M', b'M', 0, 42] => false,
            _ => return None,
        };
        Some(Tiff { data, little })
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        let bytes: [u8; 2] = self.data.get(at..at + 2)?.try_into().ok()?;
        Some(if self.little { u16::from_le_bytes(bytes) } else { u16::from_be_bytes(bytes) })
    }

    fn u32_at(&self, at: usize) -> Option<u32> {
        let bytes: [u8; 4] = self.data.get(at..at + 4)?.try_into().ok()?;
        Some(if self.little { u32::from_le_bytes(bytes) } else { u32::from_be_bytes(bytes) })
    }

    /// The SHORT or LONG values of the entry at `entry`.
    fn values(&self, entry: usize) -> Vec<u32> {
        let (Some(kind), Some(count)) = (self.u16_at(entry + 2), self.u32_at(entry + 4)) else {
            return Vec::new();
        };
        let size = match kind {
            3 => 2,
            4 | 13 => 4,
            _ => return Vec::new(),
        };
        let count = count.min(64) as usize;
        let start = if count * size <= 4 {
            entry + 8
        } else {
            match self.u32_at(entry + 8) {
                Some(pointer) => pointer as usize,
                None => return Vec::new(),
            }
        };
        (0..count)
            .map_while(|index| {
                let at = start + index * size;
                if size == 2 { self.u16_at(at).map(u32::from) } else { self.u32_at(at) }
            })
            .collect()
    }

    fn ifd(&self, offset: usize) -> Option<Ifd> {
        let count = self.u16_at(offset)? as usize;
        let mut ifd = Ifd::default();
        for index in 0..count {
            let entry = offset + 2 + index * 12;
            let tag = self.u16_at(entry)?;
            let values = self.values(entry);
            let first = values.first().copied();
            match tag {
                256 => ifd.width = first.unwrap_or(0),
                257 => ifd.height = first.unwrap_or(0),
                259 => ifd.compression = first.unwrap_or(0),
                262 => ifd.photometric = first,
                273 if values.len() == 1 => ifd.strip = Some((values[0], ifd.strip.map_or(0, |strip| strip.1))),
                279 if values.len() == 1 => ifd.strip = Some((ifd.strip.map_or(0, |strip| strip.0), values[0])),
                330 => ifd.sub_ifds = values,
                513 => ifd.jpeg = Some((first.unwrap_or(0), ifd.jpeg.map_or(0, |jpeg| jpeg.1))),
                514 => ifd.jpeg = Some((ifd.jpeg.map_or(0, |jpeg| jpeg.0), first.unwrap_or(0))),
                _ => {}
            }
        }
        ifd.next = self.u32_at(offset + 2 + count * 12).unwrap_or(0);
        Some(ifd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little-endian TIFF shaped like a Hasselblad 3FR: IFD0 holds a JPEG
    /// preview and points to a SubIFD with the CFA sensor data.
    fn hasselblad_like() -> Vec<u8> {
        let mut out = b"II\x2a\x00".to_vec();
        out.extend(8u32.to_le_bytes());
        let entry = |tag: u16, kind: u16, value: u32| {
            let mut bytes = Vec::new();
            bytes.extend(tag.to_le_bytes());
            bytes.extend(kind.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend(value.to_le_bytes());
            bytes
        };
        let ifd = |entries: &[Vec<u8>], next: u32| {
            let mut bytes = (entries.len() as u16).to_le_bytes().to_vec();
            for entry in entries {
                bytes.extend(entry);
            }
            bytes.extend(next.to_le_bytes());
            bytes
        };
        let root_len = 2 + 7 * 12 + 4;
        let sub_at = 8 + root_len as u32;
        out.extend(ifd(
            &[
                entry(256, 4, 3888),
                entry(257, 4, 2918),
                entry(259, 3, 7),
                entry(262, 3, 6),
                entry(273, 4, 5_000),
                entry(279, 4, 1_200),
                entry(330, 4, sub_at),
            ],
            0,
        ));
        out.extend(ifd(
            &[entry(256, 4, 11904), entry(257, 4, 8842), entry(259, 3, 1), entry(262, 3, 32803), entry(273, 4, 400)],
            0,
        ));
        out
    }

    #[test]
    fn finds_the_preview_and_the_sensor() {
        let layout = layout(&hasselblad_like()).unwrap();
        assert_eq!(layout.sensor, Some((11904, 8842)));
        assert_eq!(layout.preview, Some(Preview { offset: 5_000, length: 1_200, width: 3888, height: 2918 }));
        // The preview's 4:3 shape at the sensor's height.
        assert_eq!(layout.photo_size(), Some((11781, 8842)));
    }

    #[test]
    fn ignores_files_that_are_not_tiff() {
        assert_eq!(layout(b"\xFF\xD8\xFF\xE1 not a tiff"), None);
        assert_eq!(layout(b"II\x2a\x00\xFF\xFF\xFF\x7F"), None);
    }
}
