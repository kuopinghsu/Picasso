//! Automatic adjustments measured on the editor's preview image.

use image::RgbaImage;

use super::color::{decode_table, srgb_to_linear};
use super::pipeline::{TEMPERATURE_STOPS, TINT_STOPS};
use super::settings::Control;

/// Mean linear color in a square of `radius` pixels around the image
/// fractions `fx`, `fy`.
pub fn sample(image: &RgbaImage, fx: f32, fy: f32, radius: u32) -> [f32; 3] {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return [0.18; 3];
    }
    let x = ((fx * w as f32) as i64).clamp(0, w as i64 - 1);
    let y = ((fy * h as f32) as i64).clamp(0, h as i64 - 1);
    let r = radius as i64;
    let decode = decode_table();
    let mut sum = [0.0f32; 3];
    let mut count = 0.0;
    for sy in (y - r).max(0)..=(y + r).min(h as i64 - 1) {
        for sx in (x - r).max(0)..=(x + r).min(w as i64 - 1) {
            let pixel = image.get_pixel(sx as u32, sy as u32);
            for c in 0..3 {
                sum[c] += decode[pixel[c] as usize];
            }
            count += 1.0;
        }
    }
    sum.map(|value| value / count)
}

/// Temperature and tint, in slider units, that make `rgb` neutral gray.
pub fn white_balance_for(rgb: [f32; 3]) -> (f32, f32) {
    let [r, g, b] = rgb.map(|value| value.max(1e-4).log2());
    let temperature = (b - r) / (2.0 * TEMPERATURE_STOPS);
    let tint = (g - r - TEMPERATURE_STOPS * temperature) / TINT_STOPS;
    (
        (temperature * 100.0).clamp(-100.0, 100.0).round(),
        (tint * 100.0).clamp(-100.0, 100.0).round(),
    )
}

/// Gray world: the white balance that makes the average midtone neutral.
pub fn auto_white_balance(image: &RgbaImage) -> (f32, f32) {
    let decode = decode_table();
    let mut sum = [0.0f64; 3];
    for pixel in image.pixels().step_by(3) {
        let max = pixel[0].max(pixel[1]).max(pixel[2]);
        if !(16..=245).contains(&max) {
            continue;
        }
        for c in 0..3 {
            sum[c] += f64::from(decode[pixel[c] as usize]);
        }
    }
    if sum.iter().any(|value| *value <= 0.0) {
        return (0.0, 0.0);
    }
    white_balance_for(sum.map(|value| value as f32))
}

/// Light settings that spread the photo's tones over the full range.
pub fn auto_tone(image: &RgbaImage) -> Vec<(Control, f32)> {
    let mut bins = [0u64; 256];
    for pixel in image.pixels() {
        let luma = (u32::from(pixel[0]) * 306 + u32::from(pixel[1]) * 601 + u32::from(pixel[2]) * 117) >> 10;
        bins[luma.min(255) as usize] += 1;
    }
    let total = bins.iter().sum::<u64>().max(1) as f64;
    let percentile = |share: f64| -> f32 {
        let mut seen = 0u64;
        for (code, count) in bins.iter().enumerate() {
            seen += count;
            if seen as f64 >= share * total {
                return code as f32;
            }
        }
        255.0
    };
    let share = |range: std::ops::RangeInclusive<usize>| (bins[range].iter().sum::<u64>() as f64 / total) as f32;

    let median = percentile(0.5).max(1.0);
    let exposure = ((srgb_to_linear(118.0 / 255.0) / srgb_to_linear(median / 255.0)).log2() * 0.7).clamp(-2.0, 2.0);
    let gain = exposure.exp2();
    let shifted = |code: f32| super::color::linear_to_srgb((srgb_to_linear(code / 255.0) * gain).min(1.0)) * 255.0;
    let (low, high) = (shifted(percentile(0.005)), shifted(percentile(0.995)));
    let spread = shifted(percentile(0.95)) - shifted(percentile(0.05));

    let whites = if high < 245.0 { ((245.0 - high) * 0.5).min(40.0) } else { 0.0 };
    let blacks = if low > 8.0 { -((low - 5.0) * 1.5).min(40.0) } else { 0.0 };
    let bright = share(250..=255);
    let highlights = if bright > 0.01 || exposure > 0.3 { -(bright * 400.0 + exposure.max(0.0) * 25.0).min(60.0) } else { 0.0 };
    let dark = share(0..=30);
    let shadows = if dark > 0.12 { (dark * 120.0).min(50.0) } else { 0.0 };
    let contrast = if spread < 150.0 { ((150.0 - spread) * 0.25).min(30.0) } else { 0.0 };
    vec![
        (Control::EXPOSURE, (exposure * 100.0).round() / 100.0),
        (Control::CONTRAST, contrast.round()),
        (Control::HIGHLIGHTS, highlights.round()),
        (Control::SHADOWS, shadows.round()),
        (Control::WHITES, whites.round()),
        (Control::BLACKS, blacks.round()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::pipeline::white_balance_gains;

    #[test]
    fn picked_gray_becomes_neutral() {
        let cast = [0.30, 0.22, 0.12];
        let (temperature, tint) = white_balance_for(cast);
        let gains = white_balance_gains(temperature / 100.0, tint / 100.0);
        let fixed: Vec<f32> = cast.iter().zip(gains).map(|(c, g)| c * g).collect();
        assert!((fixed[0] - fixed[1]).abs() < 0.01 && (fixed[1] - fixed[2]).abs() < 0.01, "{fixed:?}");
        let dark = RgbaImage::from_fn(32, 32, |x, _| image::Rgba([(x * 2) as u8, (x * 2) as u8, (x * 2) as u8, 255]));
        let tone = auto_tone(&dark);
        assert!(tone[0].1 > 0.5, "a dark photo is brightened");
    }
}
