//! Color math: sRGB transfer tables and the OkLab space the color stage
//! works in.

use std::sync::OnceLock;

/// Steps of the linear-to-sRGB table, indexed by `sqrt(linear)`.
pub const ENCODE_STEPS: usize = 16384;

pub fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

pub fn decode_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|index| srgb_to_linear(index as f32 / 255.0)))
}

/// Linear light to sRGB codes, indexed by `sqrt(linear)` for precision in
/// the shadows.
pub fn encode_table() -> &'static [u8] {
    static TABLE: OnceLock<Vec<u8>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..ENCODE_STEPS)
            .map(|index| {
                let t = index as f32 / (ENCODE_STEPS - 1) as f32;
                (linear_to_srgb(t * t) * 255.0).round().clamp(0.0, 255.0) as u8
            })
            .collect()
    })
}

pub fn encode(linear: f32) -> u8 {
    let index = (linear.clamp(0.0, 1.0).sqrt() * (ENCODE_STEPS - 1) as f32 + 0.5) as usize;
    encode_table()[index.min(ENCODE_STEPS - 1)]
}

/// Linear sRGB to OkLab (L, a, b).
pub fn to_oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// OkLab (L, a, b) to linear sRGB.
pub fn from_oklab([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

/// Fully saturated sRGB color of an HSL hue in degrees, as display values.
pub fn hue_rgb(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as u32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

/// Unit OkLab (a, b) direction of an HSL hue, so a grading wheel's hue tints
/// toward the color shown on the wheel.
pub fn hue_direction(hue: f32) -> [f32; 2] {
    let [_, a, b] = to_oklab(hue_rgb(hue).map(srgb_to_linear));
    let length = (a * a + b * b).sqrt().max(1e-6);
    [a / length, b / length]
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_round_trips() {
        for rgb in [[0.2, 0.5, 0.8], [1.0, 1.0, 1.0], [0.01, 0.0, 0.3]] {
            let back = from_oklab(to_oklab(rgb));
            for (a, b) in rgb.iter().zip(back) {
                assert!((a - b).abs() < 1e-4, "{rgb:?} → {back:?}");
            }
        }
        let [l, a, b] = to_oklab([0.18, 0.18, 0.18]);
        assert!(l > 0.5 && a.abs() < 1e-4 && b.abs() < 1e-4);
    }
}
