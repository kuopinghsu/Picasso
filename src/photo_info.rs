//! EXIF table and tone histogram shown beside the photo viewer.
//!
//! Both are produced on a thumbnail thread together with the full-size
//! decode, so opening a photo never waits on them.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::types::{is_heic_path, is_raw_path};

/// Bin counts for red, green, blue, and luminance.
#[derive(Clone)]
pub struct Histogram {
    pub bins: [[u32; 256]; 4],
}

impl Histogram {
    pub const RED: usize = 0;
    pub const GREEN: usize = 1;
    pub const BLUE: usize = 2;
    pub const LUMA: usize = 3;

    /// Counts every opaque pixel of an RGBA buffer.
    pub fn from_rgba(rgba: &[u8]) -> Self {
        let mut bins = [[0u32; 256]; 4];
        for px in rgba.chunks_exact(4) {
            if px[3] == 0 {
                continue;
            }
            let (r, g, b) = (px[0] as u32, px[1] as u32, px[2] as u32);
            bins[Self::RED][r as usize] += 1;
            bins[Self::GREEN][g as usize] += 1;
            bins[Self::BLUE][b as usize] += 1;
            // Rec. 601 luma in fixed point; the weights sum to 1024.
            let luma = (r * 306 + g * 601 + b * 117) >> 10;
            bins[Self::LUMA][luma.min(255) as usize] += 1;
        }
        Self { bins }
    }

    pub fn total(&self) -> u64 {
        self.bins[Self::LUMA].iter().map(|&n| n as u64).sum()
    }

    /// Height used to scale the plot. The two end bins are left out so a
    /// clipped sky or black border does not flatten the rest of the curve.
    pub fn peak(&self) -> u32 {
        self.bins
            .iter()
            .flat_map(|channel| channel[1..255].iter().copied())
            .max()
            .unwrap_or(0)
            .max(1)
    }

    /// Share of pixels at pure black and pure white, as fractions.
    pub fn clipping(&self) -> (f32, f32) {
        let total = self.total().max(1) as f32;
        let luma = &self.bins[Self::LUMA];
        (luma[0] as f32 / total, luma[255] as f32 / total)
    }
}

/// Labeled EXIF values in display order. Empty when the file carries none.
pub fn exif_rows(path: &Path) -> Vec<(&'static str, String)> {
    if crate::types::is_video_path(path) {
        return match crate::video::info(path) {
            Ok(info) => crate::video::rows(&info),
            Err(err) => vec![("Video", err)],
        };
    }
    if let Some(exif) = read_exif(path) {
        let rows = rows_from_exif(&exif);
        if !rows.is_empty() {
            return rows;
        }
    }
    if is_raw_path(path) {
        let owned = path.to_path_buf();
        return std::panic::catch_unwind(move || rows_from_raw(&owned)).unwrap_or_default();
    }
    Vec::new()
}

pub(crate) fn read_exif(path: &Path) -> Option<exif::Exif> {
    if is_heic_path(path) {
        let bytes = std::fs::read(path).ok()?;
        let context = heic_rs::context::Context::open(&bytes).ok()?;
        let tiff = context.exif(context.meta.primary).ok()??;
        return exif::Reader::new().read_raw(tiff.to_vec()).ok();
    }
    if is_raw_path(path) {
        // TIFF-based RAW files keep their EXIF near the start, and reading
        // the whole container would load the sensor data too.
        let mut head = Vec::new();
        File::open(path).ok()?.take(RAW_HEAD_BYTES).read_to_end(&mut head).ok()?;
        if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
            if let Ok(exif) = exif::Reader::new().read_raw(head) {
                return Some(exif);
            }
        }
    }
    let file = File::open(path).ok()?;
    exif::Reader::new().read_from_container(&mut BufReader::new(file)).ok()
}

/// Read from a RAW file for its EXIF.
const RAW_HEAD_BYTES: u64 = 1024 * 1024;

fn rows_from_exif(exif: &exif::Exif) -> Vec<(&'static str, String)> {
    use exif::Tag;
    let text = |tag: Tag| -> Option<String> {
        let field = exif
            .get_field(tag, exif::In::PRIMARY)
            .or_else(|| exif.fields().find(|field| field.tag == tag))?;
        let value = field.display_value().with_unit(exif).to_string();
        let value = value.trim().trim_matches('"').trim().to_string();
        (!value.is_empty() && value != "unknown").then_some(value)
    };
    let mut rows = Vec::new();
    let make = text(Tag::Make).unwrap_or_default();
    let model = text(Tag::Model).unwrap_or_default();
    let camera = camera_label(&make, &model);
    if !camera.is_empty() {
        rows.push(("Camera", camera));
    }
    let table: [(&'static str, Tag); 17] = [
        ("Lens", Tag::LensModel),
        ("Taken", Tag::DateTimeOriginal),
        ("Exposure time", Tag::ExposureTime),
        ("Aperture", Tag::FNumber),
        ("ISO", Tag::PhotographicSensitivity),
        ("Focal length", Tag::FocalLength),
        ("35 mm equiv.", Tag::FocalLengthIn35mmFilm),
        ("Exposure bias", Tag::ExposureBiasValue),
        ("Program", Tag::ExposureProgram),
        ("Metering", Tag::MeteringMode),
        ("Flash", Tag::Flash),
        ("White balance", Tag::WhiteBalance),
        ("Color space", Tag::ColorSpace),
        ("Altitude", Tag::GPSAltitude),
        ("Software", Tag::Software),
        ("Artist", Tag::Artist),
        ("Copyright", Tag::Copyright),
    ];
    for (label, tag) in table {
        let value = if tag == Tag::DateTimeOriginal { text(tag).and_then(real_date) } else { text(tag) };
        if let Some(value) = value {
            rows.push((label, value));
        }
    }
    rows
}

/// `date` unless its year is a placeholder, such as `0001-01-01 00:00:00`.
fn real_date(date: String) -> Option<String> {
    let year: i32 = date.get(..4)?.parse().ok()?;
    (year >= 1826).then_some(date)
}

fn rows_from_raw(path: &Path) -> Vec<(&'static str, String)> {
    let Ok(source) = rawler::rawsource::RawSource::new(path) else {
        return Vec::new();
    };
    let Ok(decoder) = rawler::get_decoder(&source) else {
        return Vec::new();
    };
    let Ok(meta) = decoder.raw_metadata(&source, &rawler::decoders::RawDecodeParams::default()) else {
        return Vec::new();
    };
    let exif = &meta.exif;
    let ratio = |r: &rawler::formats::tiff::Rational| (r.d != 0).then(|| r.n as f64 / r.d as f64);
    let mut rows = Vec::new();
    let camera = camera_label(&meta.make, &meta.model);
    if !camera.is_empty() {
        rows.push(("Camera", camera));
    }
    if let Some(lens) = exif.lens_model.clone().filter(|lens| !lens.trim().is_empty()) {
        rows.push(("Lens", lens));
    }
    if let Some(taken) = [&exif.date_time_original, &exif.create_date].into_iter().find_map(|date| real_date(date.clone()?)) {
        rows.push(("Taken", taken));
    }
    if let Some(r) = exif.exposure_time.as_ref() {
        if r.n > 0 && r.d > 0 {
            rows.push(("Exposure time", shutter_label(r.n, r.d)));
        }
    }
    if let Some(f) = exif.fnumber.as_ref().and_then(ratio) {
        rows.push(("Aperture", format!("f/{f:.1}")));
    }
    if let Some(iso) = exif.iso_speed_ratings.map(u32::from).or(exif.iso_speed) {
        rows.push(("ISO", iso.to_string()));
    }
    if let Some(mm) = exif.focal_length.as_ref().and_then(ratio) {
        rows.push(("Focal length", format!("{mm:.0} mm")));
    }
    if let Some(bias) = exif.exposure_bias.as_ref().filter(|r| r.d != 0) {
        rows.push(("Exposure bias", format!("{:+.1} EV", bias.n as f64 / bias.d as f64)));
    }
    if let Some(flash) = exif.flash {
        let fired = crate::i18n::tr(if flash & 1 == 1 { "Fired" } else { "Did not fire" });
        rows.push(("Flash", fired.to_string()));
    }
    if let Some(artist) = exif.artist.clone().filter(|text| !text.trim().is_empty()) {
        rows.push(("Artist", artist));
    }
    if let Some(copyright) = exif.copyright.clone().filter(|text| !text.trim().is_empty()) {
        rows.push(("Copyright", copyright));
    }
    rows
}

fn camera_label(make: &str, model: &str) -> String {
    let (make, model) = (make.trim(), model.trim());
    if model.is_empty() {
        make.to_string()
    } else if make.is_empty() || model.to_lowercase().starts_with(&make.to_lowercase()) {
        model.to_string()
    } else {
        format!("{make} {model}")
    }
}

fn shutter_label(n: u32, d: u32) -> String {
    let seconds = n as f64 / d as f64;
    if seconds >= 0.3 {
        format!("{seconds:.1} s")
    } else {
        format!("1/{:.0} s", 1.0 / seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_counts_channels_and_luma() {
        let rgba = [255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 9, 9, 9, 0];
        let hist = Histogram::from_rgba(&rgba);
        assert_eq!(hist.total(), 3);
        assert_eq!(hist.bins[Histogram::RED][255], 2);
        assert_eq!(hist.bins[Histogram::GREEN][0], 2);
        assert_eq!(hist.bins[Histogram::LUMA][0], 1);
        assert_eq!(hist.bins[Histogram::LUMA][255], 1);
        let (shadows, highlights) = hist.clipping();
        assert!((shadows - 1.0 / 3.0).abs() < 1e-6);
        assert!((highlights - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn formats_camera_and_shutter() {
        assert_eq!(camera_label("NIKON CORPORATION", "NIKON Z 6"), "NIKON CORPORATION NIKON Z 6");
        assert_eq!(camera_label("Hasselblad", "Hasselblad X2D 100C"), "Hasselblad X2D 100C");
        assert_eq!(shutter_label(1, 250), "1/250 s");
        assert_eq!(shutter_label(2, 1), "2.0 s");
    }
}
