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
        for px in rgba.as_chunks::<4>().0 {
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
        File::open(path)
            .ok()?
            .take(RAW_HEAD_BYTES)
            .read_to_end(&mut head)
            .ok()?;
        if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
            if let Ok(exif) = exif::Reader::new().read_raw(head) {
                return Some(exif);
            }
        }
    }
    let file = File::open(path).ok()?;
    exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .ok()
}

/// Read from a RAW file for its EXIF.
const RAW_HEAD_BYTES: u64 = 1024 * 1024;

fn rows_from_exif(exif: &exif::Exif) -> Vec<(&'static str, String)> {
    use exif::Tag;
    let text = |tag: Tag| -> Option<String> {
        let field = exif
            .get_field(tag, exif::In::PRIMARY)
            .or_else(|| exif.fields().find(|field| field.tag == tag))?;
        let value = match &field.value {
            // Text as written: cameras pad it with empty parts, which the
            // library would show as a row of "".
            exif::Value::Ascii(parts) => ascii_text(parts),
            _ => field.display_value().with_unit(exif).to_string(),
        };
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
    // A number stored as a fraction, as the library would print it in full
    // (f/1.7799999713880652), read to be rounded as people write it.
    let number = |tag: Tag| -> Option<f64> {
        let field = exif
            .get_field(tag, exif::In::PRIMARY)
            .or_else(|| exif.fields().find(|field| field.tag == tag))?;
        let value = match &field.value {
            exif::Value::Rational(values) => values
                .first()
                .filter(|r| r.denom != 0)
                .map(|r| f64::from(r.num) / f64::from(r.denom)),
            exif::Value::SRational(values) => values
                .first()
                .filter(|r| r.denom != 0)
                .map(|r| f64::from(r.num) / f64::from(r.denom)),
            _ => None,
        }?;
        value.is_finite().then_some(value)
    };
    let formatted = |tag: Tag| -> Option<String> {
        match tag {
            Tag::FNumber => number(tag)
                .filter(|f| *f > 0.0)
                .map(|f| format!("f/{}", short(f, 1))),
            Tag::FocalLength => number(tag)
                .filter(|mm| *mm > 0.0)
                .map(|mm| format!("{} mm", short(mm, 1))),
            Tag::ExposureTime => number(tag).filter(|t| *t > 0.0).map(shutter_text),
            Tag::ExposureBiasValue => number(tag).map(|ev| match short(ev, 1).as_str() {
                "0" | "-0" => "0 EV".to_string(),
                text if ev > 0.0 => format!("+{text} EV"),
                text => format!("{text} EV"),
            }),
            Tag::Flash => exif
                .get_field(tag, exif::In::PRIMARY)
                .and_then(|field| field.value.get_uint(0))
                .map(flash_text),
            // A setting written as a number: its name in the app's language,
            // nothing for one not recorded.
            Tag::ExposureProgram | Tag::MeteringMode | Tag::WhiteBalance | Tag::ColorSpace => {
                let code = exif
                    .get_field(tag, exif::In::PRIMARY)
                    .and_then(|field| field.value.get_uint(0))?;
                return mode_name(tag, code).map(|name| crate::i18n::tr(name).to_string());
            }
            Tag::GPSAltitude => number(tag).map(|m| {
                // A reference of 1 is below sea level.
                let below = exif
                    .get_field(Tag::GPSAltitudeRef, exif::In::PRIMARY)
                    .and_then(|field| field.value.get_uint(0))
                    == Some(1);
                format!("{} m", short(if below { -m } else { m }, 0))
            }),
            _ => None,
        }
        .or_else(|| text(tag))
    };
    for (label, tag) in table {
        let value = if tag == Tag::DateTimeOriginal {
            text(tag).and_then(real_date)
        } else {
            formatted(tag)
        };
        if let Some(value) = value {
            rows.push((label, value));
        }
    }
    rows
}

/// The English name, a key for `tr`, of setting `tag`'s value `code`, as
/// EXIF numbers them; `None` for "not recorded" and codes it doesn't define.
fn mode_name(tag: exif::Tag, code: u32) -> Option<&'static str> {
    use exif::Tag;
    Some(match (tag, code) {
        (Tag::ExposureProgram, 1) => "Manual",
        (Tag::ExposureProgram, 2) => "Normal program",
        (Tag::ExposureProgram, 3) => "Aperture priority",
        (Tag::ExposureProgram, 4) => "Shutter priority",
        (Tag::ExposureProgram, 5) => "Creative program",
        (Tag::ExposureProgram, 6) => "Action program",
        (Tag::ExposureProgram, 7) => "Portrait mode",
        (Tag::ExposureProgram, 8) => "Landscape mode",
        (Tag::MeteringMode, 1) => "Average",
        (Tag::MeteringMode, 2) => "Center-weighted average",
        (Tag::MeteringMode, 3) => "Spot",
        (Tag::MeteringMode, 4) => "Multi-spot",
        (Tag::MeteringMode, 5) => "Multi-segment",
        (Tag::MeteringMode, 6) => "Partial",
        (Tag::MeteringMode, 255) => "Other",
        (Tag::WhiteBalance, 0) => "Auto white balance",
        (Tag::WhiteBalance, 1) => "Manual white balance",
        (Tag::ColorSpace, 1) => "sRGB",
        (Tag::ColorSpace, 2) => "Adobe RGB",
        (Tag::ColorSpace, 0xFFFF) => "Uncalibrated",
        _ => return None,
    })
}

/// What EXIF's flash code says, in a few words: whether it fired, and with
/// red-eye reduction. How the camera could sense the light coming back, and
/// a mode it didn't record, are left out, as no one looks for them.
fn flash_text(code: u32) -> String {
    use crate::i18n::tr;
    if code & 0x20 != 0 {
        return tr("No flash").to_string();
    }
    match (code & 0x01 != 0, code & 0x40 != 0) {
        (true, true) => format!("{}, {}", tr("Fired"), tr("red-eye reduction")),
        (true, false) => tr("Fired").to_string(),
        (false, _) => tr("Did not fire").to_string(),
    }
}

/// `value` with at most `decimals` decimals, without trailing zeros: 1.78 is
/// "1.8" at one decimal, 4.0 is "4".
fn short(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// An exposure time as photographers write it: "1/4608 s" when short,
/// "0.5 s" or "2.5 s" from 0.3 s, where fractions stop being read as such.
fn shutter_text(seconds: f64) -> String {
    if seconds >= 0.3 {
        format!("{} s", short(seconds, 1))
    } else {
        format!("1/{} s", (1.0 / seconds).round() as u64)
    }
}

/// The words of an EXIF text field, which may be split into parts at NUL
/// bytes: the parts with anything in them, each trimmed of spaces, NULs and
/// quotes, joined with "; ", each once. Empty when none has anything.
fn ascii_text(parts: &[Vec<u8>]) -> String {
    let mut kept: Vec<String> = Vec::new();
    for part in parts {
        let text = String::from_utf8_lossy(part);
        let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0' || c == '"');
        if !text.is_empty() && !kept.iter().any(|seen| seen == text) {
            kept.push(text.to_string());
        }
    }
    kept.join("; ")
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
    let Ok(meta) = decoder.raw_metadata(&source, &rawler::decoders::RawDecodeParams::default())
    else {
        return Vec::new();
    };
    let exif = &meta.exif;
    let ratio = |r: &rawler::formats::tiff::Rational| (r.d != 0).then(|| r.n as f64 / r.d as f64);
    let mut rows = Vec::new();
    let camera = camera_label(&meta.make, &meta.model);
    if !camera.is_empty() {
        rows.push(("Camera", camera));
    }
    if let Some(lens) = exif
        .lens_model
        .clone()
        .filter(|lens| !lens.trim().is_empty())
    {
        rows.push(("Lens", lens));
    }
    if let Some(taken) = [&exif.date_time_original, &exif.create_date]
        .into_iter()
        .find_map(|date| real_date(date.clone()?))
    {
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
        rows.push((
            "Exposure bias",
            format!("{:+.1} EV", bias.n as f64 / bias.d as f64),
        ));
    }
    if let Some(flash) = exif.flash {
        rows.push(("Flash", flash_text(u32::from(flash))));
    }
    if let Some(artist) = exif.artist.clone().filter(|text| !text.trim().is_empty()) {
        rows.push(("Artist", artist));
    }
    if let Some(copyright) = exif
        .copyright
        .clone()
        .filter(|text| !text.trim().is_empty())
    {
        rows.push(("Copyright", copyright));
    }
    rows
}

fn camera_label(make: &str, model: &str) -> String {
    crate::image_io::camera_name(make, model)
}

fn shutter_label(n: u32, d: u32) -> String {
    shutter_text(n as f64 / d as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_stored_as_fractions_read_as_people_write_them() {
        use exif::{Field, In, Rational, SRational, Tag, Value};
        let field = |tag, value| Field {
            tag,
            ifd_num: In::PRIMARY,
            value,
        };
        let rational = |num, denom| Value::Rational(vec![Rational { num, denom }]);
        // As an iPhone 15 Pro Max writes them.
        let source = [
            field(Tag::FNumber, rational(178, 100)),
            field(Tag::ExposureTime, rational(1, 4608)),
            field(Tag::PhotographicSensitivity, Value::Short(vec![64])),
            field(Tag::FocalLength, rational(6765, 1000)),
            field(
                Tag::ExposureBiasValue,
                Value::SRational(vec![SRational { num: -1, denom: 3 }]),
            ),
            field(Tag::GPSAltitudeRef, Value::Byte(vec![1])),
            field(Tag::GPSAltitude, rational(12345, 1000)),
            // Not fired, no return light detection, mode unknown.
            field(Tag::Flash, Value::Short(vec![0])),
        ];
        let mut writer = exif::experimental::Writer::new();
        source.iter().for_each(|field| writer.push_field(field));
        let mut tiff = std::io::Cursor::new(Vec::new());
        writer.write(&mut tiff, false).unwrap();
        let exif = exif::Reader::new().read_raw(tiff.into_inner()).unwrap();
        let rows = rows_from_exif(&exif);
        let value = |label: &str| {
            rows.iter()
                .find(|(name, _)| *name == label)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(value("Aperture"), Some("f/1.8"));
        assert_eq!(value("Exposure time"), Some("1/4608 s"));
        assert_eq!(value("ISO"), Some("64"));
        assert_eq!(value("Focal length"), Some("6.8 mm"));
        assert_eq!(value("Exposure bias"), Some("-0.3 EV"));
        assert_eq!(value("Altitude"), Some("-12 m"), "below sea level");
        assert_eq!(
            value("Flash"),
            Some("Did not fire"),
            "only whether it fired"
        );
        assert_eq!(
            (
                flash_text(0x19),
                flash_text(0x59),
                flash_text(0x10),
                flash_text(0x20)
            ),
            (
                "Fired".into(),
                "Fired, red-eye reduction".into(),
                "Did not fire".into(),
                "No flash".into()
            )
        );
        assert_eq!(
            (
                short(4.0, 1),
                short(2.25, 1),
                shutter_text(0.5),
                shutter_text(2.0)
            ),
            ("4".into(), "2.2".into(), "0.5 s".into(), "2 s".into())
        );
    }

    #[test]
    fn settings_written_as_numbers_are_named_in_every_language() {
        use exif::Tag;
        assert_eq!(
            mode_name(Tag::ExposureProgram, 3),
            Some("Aperture priority")
        );
        assert_eq!(mode_name(Tag::WhiteBalance, 0), Some("Auto white balance"));
        assert_eq!(mode_name(Tag::MeteringMode, 5), Some("Multi-segment"));
        assert_eq!(mode_name(Tag::ColorSpace, 1), Some("sRGB"));
        assert_eq!(
            (
                mode_name(Tag::ExposureProgram, 0),
                mode_name(Tag::MeteringMode, 0),
                mode_name(Tag::ExposureProgram, 99)
            ),
            (None, None, None),
            "not recorded, or not defined: no row"
        );
        // Every name but the colour spaces' is in every language's table.
        let zh = crate::i18n::Lang::TraditionalChinese;
        for (tag, codes) in [
            (Tag::ExposureProgram, 1..=8),
            (Tag::MeteringMode, 1..=6),
            (Tag::WhiteBalance, 0..=1),
        ] {
            for code in codes {
                let name = mode_name(tag, code).unwrap();
                assert!(
                    crate::i18n::table(zh).contains_key(name),
                    "{name} has no Chinese"
                );
            }
        }
        assert_eq!(
            crate::i18n::table(zh).get("Aperture priority"),
            Some(&"光圈先決")
        );
        assert_eq!(
            crate::i18n::table(zh).get("Auto white balance"),
            Some(&"自動白平衡")
        );
    }

    #[test]
    fn text_padded_with_empty_parts_shows_only_its_words() {
        use exif::{Field, In, Tag, Value};
        let field = |tag, parts: &[&[u8]]| Field {
            tag,
            ifd_num: In::PRIMARY,
            value: Value::Ascii(parts.iter().map(|part| part.to_vec()).collect()),
        };
        // As some cameras write them: the words, then empty parts or spaces.
        let source = [
            field(
                Tag::LensModel,
                &[b"NIKKOR Z 180-600mm f/5.6-6.3 VR", b"", b"", b""],
            ),
            field(Tag::Artist, &[b"", b"", b"", b"    "]),
            field(Tag::Copyright, &[b"", b"", b""]),
            field(Tag::Software, &[b"Ver.03.10", b"", b""]),
        ];
        let mut writer = exif::experimental::Writer::new();
        source.iter().for_each(|field| writer.push_field(field));
        let mut tiff = std::io::Cursor::new(Vec::new());
        writer.write(&mut tiff, false).unwrap();
        let exif = exif::Reader::new().read_raw(tiff.into_inner()).unwrap();
        let rows = rows_from_exif(&exif);
        assert_eq!(
            rows,
            [
                ("Lens", "NIKKOR Z 180-600mm f/5.6-6.3 VR".to_string()),
                ("Software", "Ver.03.10".to_string())
            ],
            "no empty parts, and no row for a field with nothing in it"
        );
        assert_eq!(
            ascii_text(&[
                b"Ann".to_vec(),
                b"\0".to_vec(),
                b"Ann".to_vec(),
                b"\"Studio\"".to_vec()
            ]),
            "Ann; Studio"
        );
    }

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
        assert_eq!(camera_label("NIKON CORPORATION", "NIKON Z 6"), "Nikon Z 6");
        assert_eq!(
            camera_label("Hasselblad", "Hasselblad X2D 100C"),
            "Hasselblad X2D 100C"
        );
        assert_eq!(shutter_label(1, 250), "1/250 s");
        assert_eq!(shutter_label(2, 1), "2 s");
    }
}
