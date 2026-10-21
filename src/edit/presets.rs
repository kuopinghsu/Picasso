//! Built-in presets, profiles, white balance presets, and the presets
//! users save.

use serde_json::{json, Map, Value};

use super::settings::{Control, Settings, Unit};
use crate::i18n::tr;

pub struct Preset {
    pub name: &'static str,
    pub group: &'static str,
    values: &'static [(&'static str, f32)],
    /// Points of the RGB curve, when the preset has one.
    curve: Option<&'static [[f32; 2]]>,
}

impl Preset {
    pub fn patch(&self) -> Map<String, Value> {
        let mut patch: Map<String, Value> = self.values.iter().map(|(key, value)| ((*key).to_string(), json!(value))).collect();
        if let Some(points) = self.curve {
            patch.insert("curve.points.rgb".into(), json!(points));
        }
        patch
    }
}

pub const GROUPS: [&str; 6] = ["Color", "B&W", "Film", "Landscape", "Portrait", "Style"];

const FADE: &[[f32; 2]] = &[[0.0, 0.1], [0.25, 0.27], [0.75, 0.77], [1.0, 0.95]];

const fn preset(name: &'static str, group: &'static str, values: &'static [(&'static str, f32)]) -> Preset {
    Preset {
        name,
        group,
        values,
        curve: None,
    }
}

const fn faded(name: &'static str, group: &'static str, values: &'static [(&'static str, f32)]) -> Preset {
    Preset {
        name,
        group,
        values,
        curve: Some(FADE),
    }
}

pub const BUILTIN: &[Preset] = &[
    preset(
        "Vivid Pop",
        "Color",
        &[("presence.vibrance", 35.0), ("presence.saturation", 10.0), ("light.contrast", 15.0), ("presence.clarity", 10.0)],
    ),
    preset(
        "Golden Hour",
        "Color",
        &[
            ("wb.temperature", 25.0),
            ("wb.tint", 5.0),
            ("light.highlights", -20.0),
            ("light.shadows", 15.0),
            ("presence.vibrance", 15.0),
            ("grade.highlights.hue", 45.0),
            ("grade.highlights.saturation", 25.0),
            ("grade.shadows.hue", 25.0),
            ("grade.shadows.saturation", 10.0),
        ],
    ),
    preset(
        "Cool Breeze",
        "Color",
        &[
            ("wb.temperature", -20.0),
            ("presence.vibrance", 10.0),
            ("light.highlights", -10.0),
            ("grade.shadows.hue", 210.0),
            ("grade.shadows.saturation", 15.0),
        ],
    ),
    preset(
        "Teal & Orange",
        "Color",
        &[
            ("light.contrast", 15.0),
            ("grade.shadows.hue", 195.0),
            ("grade.shadows.saturation", 30.0),
            ("grade.highlights.hue", 35.0),
            ("grade.highlights.saturation", 30.0),
            ("mix.orange.saturation", 15.0),
            ("mix.aqua.hue", 20.0),
            ("mix.blue.hue", -15.0),
        ],
    ),
    preset(
        "Soft Pastel",
        "Color",
        &[
            ("light.contrast", -25.0),
            ("light.highlights", -15.0),
            ("light.shadows", 25.0),
            ("light.blacks", 15.0),
            ("presence.saturation", -15.0),
            ("presence.vibrance", 10.0),
        ],
    ),
    preset("Classic B&W", "B&W", &[("bw.enabled", 1.0), ("light.contrast", 20.0), ("presence.clarity", 10.0)]),
    preset(
        "High Contrast B&W",
        "B&W",
        &[
            ("bw.enabled", 1.0),
            ("light.contrast", 50.0),
            ("light.whites", 20.0),
            ("light.blacks", -25.0),
            ("presence.clarity", 25.0),
            ("bw.red", 20.0),
            ("bw.blue", -30.0),
        ],
    ),
    preset(
        "Selenium Tone",
        "B&W",
        &[
            ("bw.enabled", 1.0),
            ("light.contrast", 15.0),
            ("grade.shadows.hue", 280.0),
            ("grade.shadows.saturation", 20.0),
            ("grade.highlights.hue", 40.0),
            ("grade.highlights.saturation", 8.0),
        ],
    ),
    preset(
        "Sepia",
        "B&W",
        &[("bw.enabled", 1.0), ("light.contrast", -5.0), ("grade.global.hue", 38.0), ("grade.global.saturation", 30.0)],
    ),
    preset(
        "Infrared",
        "B&W",
        &[
            ("bw.enabled", 1.0),
            ("bw.yellow", 50.0),
            ("bw.green", 80.0),
            ("bw.aqua", -30.0),
            ("bw.blue", -60.0),
            ("light.contrast", 25.0),
            ("light.highlights", -20.0),
        ],
    ),
    faded(
        "Faded Matte",
        "Film",
        &[("light.contrast", -10.0), ("presence.saturation", -10.0), ("grain.amount", 15.0)],
    ),
    preset(
        "Portra Film",
        "Film",
        &[
            ("wb.temperature", 8.0),
            ("presence.saturation", -8.0),
            ("presence.vibrance", 10.0),
            ("mix.orange.saturation", -10.0),
            ("mix.orange.luminance", 10.0),
            ("grade.shadows.hue", 200.0),
            ("grade.shadows.saturation", 8.0),
            ("grade.highlights.hue", 50.0),
            ("grade.highlights.saturation", 10.0),
            ("grain.amount", 20.0),
            ("grain.size", 30.0),
        ],
    ),
    preset(
        "Kodachrome",
        "Film",
        &[
            ("light.contrast", 25.0),
            ("light.blacks", -10.0),
            ("presence.saturation", 15.0),
            ("mix.red.saturation", 20.0),
            ("mix.yellow.hue", -10.0),
            ("mix.blue.saturation", 15.0),
            ("grain.amount", 10.0),
        ],
    ),
    faded(
        "Vintage",
        "Film",
        &[
            ("wb.temperature", 15.0),
            ("presence.saturation", -25.0),
            ("light.contrast", -10.0),
            ("grade.shadows.hue", 60.0),
            ("grade.shadows.saturation", 15.0),
            ("vignette.amount", -25.0),
            ("grain.amount", 30.0),
        ],
    ),
    preset(
        "Cross Process",
        "Film",
        &[
            ("light.contrast", 25.0),
            ("presence.saturation", 15.0),
            ("grade.shadows.hue", 170.0),
            ("grade.shadows.saturation", 30.0),
            ("grade.highlights.hue", 60.0),
            ("grade.highlights.saturation", 35.0),
        ],
    ),
    preset(
        "Crisp Landscape",
        "Landscape",
        &[
            ("light.highlights", -30.0),
            ("light.shadows", 25.0),
            ("presence.clarity", 25.0),
            ("presence.dehaze", 15.0),
            ("presence.vibrance", 25.0),
            ("mix.green.saturation", 10.0),
            ("mix.blue.saturation", 15.0),
            ("sharpen.amount", 40.0),
        ],
    ),
    preset(
        "Moody Forest",
        "Landscape",
        &[
            ("wb.temperature", -5.0),
            ("light.contrast", 15.0),
            ("light.highlights", -25.0),
            ("presence.clarity", 15.0),
            ("mix.green.hue", -20.0),
            ("mix.green.saturation", -25.0),
            ("mix.green.luminance", -20.0),
            ("grade.shadows.hue", 160.0),
            ("grade.shadows.saturation", 10.0),
            ("vignette.amount", -20.0),
        ],
    ),
    preset(
        "Blue Hour",
        "Landscape",
        &[
            ("wb.temperature", -25.0),
            ("wb.tint", 8.0),
            ("light.highlights", -15.0),
            ("presence.vibrance", 20.0),
            ("grade.shadows.hue", 230.0),
            ("grade.shadows.saturation", 25.0),
        ],
    ),
    preset(
        "Soft Portrait",
        "Portrait",
        &[
            ("wb.temperature", 5.0),
            ("light.highlights", -10.0),
            ("light.shadows", 15.0),
            ("presence.texture", -25.0),
            ("presence.clarity", -10.0),
            ("mix.orange.saturation", -5.0),
            ("mix.orange.luminance", 10.0),
            ("vignette.amount", -10.0),
        ],
    ),
    preset(
        "Warm Skin",
        "Portrait",
        &[
            ("wb.temperature", 12.0),
            ("presence.vibrance", 10.0),
            ("presence.texture", -10.0),
            ("mix.red.saturation", -5.0),
            ("mix.orange.saturation", 10.0),
            ("mix.orange.luminance", 8.0),
        ],
    ),
    preset(
        "Moody",
        "Style",
        &[
            ("light.exposure", -0.3),
            ("light.contrast", 20.0),
            ("light.highlights", -40.0),
            ("light.shadows", -10.0),
            ("presence.saturation", -20.0),
            ("presence.clarity", 15.0),
            ("grade.shadows.hue", 210.0),
            ("grade.shadows.saturation", 15.0),
            ("vignette.amount", -30.0),
        ],
    ),
    preset(
        "Cinematic",
        "Style",
        &[
            ("light.contrast", 15.0),
            ("presence.saturation", -10.0),
            ("mix.green.saturation", -20.0),
            ("grade.shadows.hue", 190.0),
            ("grade.shadows.saturation", 25.0),
            ("grade.highlights.hue", 40.0),
            ("grade.highlights.saturation", 20.0),
            ("grade.balance", -10.0),
            ("vignette.amount", -15.0),
        ],
    ),
    preset(
        "Bright & Airy",
        "Style",
        &[
            ("light.exposure", 0.4),
            ("light.contrast", -10.0),
            ("light.highlights", -30.0),
            ("light.shadows", 35.0),
            ("wb.temperature", -5.0),
            ("presence.vibrance", 15.0),
            ("presence.clarity", -5.0),
        ],
    ),
];

pub fn builtin(name: &str) -> Option<&'static Preset> {
    BUILTIN.iter().find(|preset| preset.name == name)
}

/// Translated name of a built-in preset; user presets keep their name.
pub fn display_name(name: &str) -> String {
    builtin(name).map_or_else(|| name.to_string(), |preset| tr(preset.name).to_string())
}

/// `base` with the preset `patch` applied at `amount` (0…2): each value is
/// moved from neutral toward the preset's by that much. Switches, the
/// profile, and hues are set when the amount isn't zero.
pub fn apply(base: &Settings, patch: &Map<String, Value>, amount: f32) -> Settings {
    let mut target = Settings::default();
    target.apply_patch(patch);
    let mut out = base.clone();
    for control in Control::all().filter(|control| patch.contains_key(control.key())) {
        let value = target.get(control);
        match control.def().unit {
            Unit::Toggle | Unit::Choice | Unit::Hue => {
                if amount > 0.0 {
                    out.set(control, value);
                }
            }
            _ => {
                let neutral = control.neutral();
                out.set(control, neutral + (value - neutral) * amount);
            }
        }
    }
    for (index, curve) in target.curves.iter().enumerate() {
        if *curve != super::settings::Curve::default() {
            out.curves[index] = super::settings::Curve(curve.0.iter().map(|[x, y]| [*x, x + (y - x) * amount.min(1.5)]).collect());
        }
    }
    if patch.contains_key("masks") && amount > 0.0 {
        out.masks = target.masks;
    }
    out
}

/// Slider offsets a profile adds under the user's values.
pub fn profile_offsets(profile: usize) -> Vec<(Control, f32)> {
    match profile {
        1 => vec![(Control::CONTRAST, -10.0), (Control::VIBRANCE, -10.0)],
        2 => vec![(Control::CONTRAST, 10.0), (Control::VIBRANCE, 25.0), (Control::SATURATION, 5.0)],
        3 => vec![
            (Control::VIBRANCE, 15.0),
            (Control::CLARITY, 10.0),
            (Control::DEHAZE, 5.0),
            (Control::mix(1, 3), 10.0),
            (Control::mix(1, 5), 10.0),
        ],
        4 => vec![
            (Control::CONTRAST, -5.0),
            (Control::TEXTURE, -10.0),
            (Control::mix(1, 1), -5.0),
            (Control::mix(2, 1), 5.0),
        ],
        5 => vec![(Control::MONOCHROME, 1.0), (Control::CONTRAST, 10.0)],
        _ => Vec::new(),
    }
}

/// White balance presets: name, temperature, and tint.
pub const WHITE_BALANCE: [(&str, f32, f32); 7] = [
    ("As shot", 0.0, 0.0),
    ("Daylight", 5.0, 0.0),
    ("Cloudy", 15.0, 3.0),
    ("Shade", 25.0, 5.0),
    ("Tungsten", -45.0, 5.0),
    ("Fluorescent", -25.0, 20.0),
    ("Flash", 10.0, 0.0),
];

pub fn white_balance_name(name: &str) -> String {
    WHITE_BALANCE
        .iter()
        .find(|(preset, _, _)| *preset == name)
        .map_or_else(|| name.to_string(), |(preset, _, _)| tr(preset).to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserPreset {
    pub name: String,
    pub patch: Map<String, Value>,
}

impl UserPreset {
    /// Everything in `settings` except straighten, flips, and crop.
    pub fn new(name: String, settings: &Settings) -> UserPreset {
        UserPreset {
            name,
            patch: settings.without_geometry().changed_from(&Settings::default()),
        }
    }
}

pub fn user_presets_from_json(text: &str) -> Vec<UserPreset> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            Some(UserPreset {
                name: entry["name"].as_str()?.to_string(),
                patch: entry["patch"].as_object()?.clone(),
            })
        })
        .collect()
}

pub fn user_presets_to_json(presets: &[UserPreset]) -> String {
    Value::Array(presets.iter().map(|preset| json!({ "name": preset.name, "patch": preset.patch })).collect()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_use_known_keys_and_scale_by_amount() {
        for preset in BUILTIN {
            assert!(GROUPS.contains(&preset.group), "{}", preset.name);
            for (key, _) in preset.values {
                assert!(Control::from_key(key).is_some(), "{}: {key}", preset.name);
            }
        }
        let golden = builtin("Golden Hour").unwrap().patch();
        let half = apply(&Settings::default(), &golden, 0.5);
        assert_eq!(half.get(Control::TEMPERATURE), 12.5);
        assert_eq!(half.get(Control::grade(2, 0)), 45.0);
        let none = apply(&Settings::default(), &golden, 0.0);
        assert!(none.is_identity());
        let saved = vec![UserPreset::new("Mine".into(), &half)];
        assert_eq!(user_presets_from_json(&user_presets_to_json(&saved)), saved);
    }
}
