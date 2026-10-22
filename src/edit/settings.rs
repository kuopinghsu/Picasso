//! Every value an edit can change, and the history of steps that set them.
//!
//! Settings are named values: one number per slider, the four point curves,
//! and the masks. A step stores only the values it changed, so the history
//! replays from the untouched photo and old libraries keep loading.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// −100…100, shown with a sign.
    Signed,
    /// 0…100, shown as is.
    Plain,
    Ev,
    Degrees,
    Hue,
    Radius,
    Toggle,
    Choice,
    /// Crop edges, as fractions of the canvas.
    Fraction,
}

pub struct Def {
    pub key: String,
    pub label: &'static str,
    /// Group the label belongs to, such as a color band or a grading zone.
    pub prefix: Option<&'static str>,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Unit,
}

fn def(key: impl Into<String>, label: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> Def {
    Def {
        key: key.into(),
        label,
        prefix: None,
        min,
        max,
        default,
        unit,
    }
}

impl Def {
    fn prefixed(mut self, prefix: &'static str) -> Def {
        self.prefix = Some(prefix);
        self
    }
}

pub const BANDS: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];
const BAND_KEYS: [&str; 8] = ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"];
/// OkLCh hue of each band's center, in degrees.
pub const BAND_HUES: [f32; 8] = [29.0, 60.0, 105.0, 142.0, 195.0, 262.0, 300.0, 335.0];
pub const ZONES: [&str; 4] = ["Shadows", "Midtones", "Highlights", "Global"];
const ZONE_KEYS: [&str; 4] = ["shadows", "midtones", "highlights", "global"];
pub const PROFILES: [&str; 6] = ["Color", "Neutral", "Vivid", "Landscape", "Portrait", "Monochrome"];
pub const CURVE_CHANNELS: [&str; 4] = ["RGB", "Red", "Green", "Blue"];
const CURVE_KEYS: [&str; 4] = ["curve.points.rgb", "curve.points.red", "curve.points.green", "curve.points.blue"];
pub const MIX_PARTS: [&str; 3] = ["Hue", "Saturation", "Luminance"];

/// Adjustments a mask applies to the area it covers.
pub const LOCALS: [&str; 9] = [
    "Exposure",
    "Contrast",
    "Highlights",
    "Shadows",
    "Temperature",
    "Tint",
    "Saturation",
    "Clarity",
    "Dehaze",
];
const LOCAL_KEYS: [&str; 9] = [
    "exposure",
    "contrast",
    "highlights",
    "shadows",
    "temperature",
    "tint",
    "saturation",
    "clarity",
    "dehaze",
];
pub const LOCAL_EXPOSURE: usize = 0;
pub const LOCAL_CONTRAST: usize = 1;
pub const LOCAL_HIGHLIGHTS: usize = 2;
pub const LOCAL_SHADOWS: usize = 3;
pub const LOCAL_TEMPERATURE: usize = 4;
pub const LOCAL_TINT: usize = 5;
pub const LOCAL_SATURATION: usize = 6;
pub const LOCAL_CLARITY: usize = 7;
pub const LOCAL_DEHAZE: usize = 8;

const COUNT: usize = 89;
const MIX: u8 = 20;
const GRADE: u8 = 44;
const BW: u8 = 59;

/// One slider or switch, by its index in the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Control(pub u8);

impl Control {
    pub const EXPOSURE: Control = Control(0);
    pub const CONTRAST: Control = Control(1);
    pub const HIGHLIGHTS: Control = Control(2);
    pub const SHADOWS: Control = Control(3);
    pub const WHITES: Control = Control(4);
    pub const BLACKS: Control = Control(5);
    pub const TEMPERATURE: Control = Control(6);
    pub const TINT: Control = Control(7);
    pub const TEXTURE: Control = Control(8);
    pub const CLARITY: Control = Control(9);
    pub const DEHAZE: Control = Control(10);
    pub const VIBRANCE: Control = Control(11);
    pub const SATURATION: Control = Control(12);
    pub const CURVE_HIGHLIGHTS: Control = Control(13);
    pub const CURVE_LIGHTS: Control = Control(14);
    pub const CURVE_DARKS: Control = Control(15);
    pub const CURVE_SHADOWS: Control = Control(16);
    pub const SPLIT_SHADOWS: Control = Control(17);
    pub const SPLIT_MIDTONES: Control = Control(18);
    pub const SPLIT_HIGHLIGHTS: Control = Control(19);
    pub const GRADE_BLENDING: Control = Control(56);
    pub const GRADE_BALANCE: Control = Control(57);
    pub const MONOCHROME: Control = Control(58);
    pub const VIGNETTE_AMOUNT: Control = Control(67);
    pub const VIGNETTE_MIDPOINT: Control = Control(68);
    pub const VIGNETTE_ROUNDNESS: Control = Control(69);
    pub const VIGNETTE_FEATHER: Control = Control(70);
    pub const VIGNETTE_HIGHLIGHTS: Control = Control(71);
    pub const GRAIN_AMOUNT: Control = Control(72);
    pub const GRAIN_SIZE: Control = Control(73);
    pub const GRAIN_ROUGHNESS: Control = Control(74);
    pub const SHARPEN_AMOUNT: Control = Control(75);
    pub const SHARPEN_RADIUS: Control = Control(76);
    pub const SHARPEN_DETAIL: Control = Control(77);
    pub const SHARPEN_MASKING: Control = Control(78);
    pub const NOISE_LUMINANCE: Control = Control(79);
    pub const NOISE_COLOR: Control = Control(80);
    pub const STRAIGHTEN: Control = Control(81);
    pub const FLIP_H: Control = Control(82);
    pub const FLIP_V: Control = Control(83);
    pub const CROP_LEFT: Control = Control(84);
    pub const CROP_TOP: Control = Control(85);
    pub const CROP_RIGHT: Control = Control(86);
    pub const CROP_BOTTOM: Control = Control(87);
    pub const PROFILE: Control = Control(88);

    /// Color mixer slider: `part` 0 hue, 1 saturation, 2 luminance.
    pub const fn mix(part: usize, band: usize) -> Control {
        Control(MIX + (part * 8 + band) as u8)
    }

    /// Color grading slider: `part` 0 hue, 1 saturation, 2 luminance.
    pub const fn grade(zone: usize, part: usize) -> Control {
        Control(GRADE + (zone * 3 + part) as u8)
    }

    /// Black & white mix slider of a color band.
    pub const fn bw(band: usize) -> Control {
        Control(BW + band as u8)
    }

    pub const LIGHT: [Control; 6] = [
        Control::EXPOSURE,
        Control::CONTRAST,
        Control::HIGHLIGHTS,
        Control::SHADOWS,
        Control::WHITES,
        Control::BLACKS,
    ];

    pub fn def(self) -> &'static Def {
        let defs = defs();
        &defs[(self.0 as usize).min(defs.len() - 1)]
    }

    pub fn key(self) -> &'static str {
        &self.def().key
    }

    /// Full translated name, such as "Red Hue", for the history list.
    pub fn label(self) -> String {
        let def = self.def();
        match def.prefix {
            Some(prefix) => trf("{} {}", &[&tr(prefix), &tr(def.label)]),
            None => tr(def.label).to_string(),
        }
    }

    #[cfg(test)]
    pub fn range(self) -> (f32, f32) {
        let def = self.def();
        (def.min, def.max)
    }

    pub fn neutral(self) -> f32 {
        self.def().default
    }

    pub fn format(self, value: f32) -> String {
        match self.def().unit {
            Unit::Ev => format!("{value:+.2} EV"),
            Unit::Degrees => format!("{value:+.1}°"),
            Unit::Hue => format!("{value:.0}°"),
            Unit::Radius => format!("{value:.1}"),
            Unit::Toggle => tr(if value >= 0.5 { "On" } else { "Off" }).to_string(),
            Unit::Choice => tr(PROFILES[(value.round() as usize).min(PROFILES.len() - 1)]).to_string(),
            Unit::Fraction => format!("{:.0}%", value * 100.0),
            Unit::Plain => format!("{value:.0}"),
            Unit::Signed if value.round() == 0.0 => "0".into(),
            Unit::Signed => format!("{value:+.0}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Control> {
        static KEYS: OnceLock<HashMap<&'static str, Control>> = OnceLock::new();
        KEYS.get_or_init(|| {
            defs()
                .iter()
                .enumerate()
                .map(|(index, def)| (def.key.as_str(), Control(index as u8)))
                .collect()
        })
        .get(key)
        .copied()
    }

    pub fn all() -> impl Iterator<Item = Control> {
        (0..COUNT as u8).map(Control)
    }

    fn is_geometry(self) -> bool {
        (Control::STRAIGHTEN.0..=Control::CROP_BOTTOM.0).contains(&self.0)
    }
}

fn defs() -> &'static [Def] {
    static DEFS: OnceLock<Vec<Def>> = OnceLock::new();
    DEFS.get_or_init(|| {
        let signed = |key: String, label| def(key, label, -100.0, 100.0, 0.0, Unit::Signed);
        let plain = |key: String, label, default| def(key, label, 0.0, 100.0, default, Unit::Plain);
        let mut list = vec![
            def("light.exposure", "Exposure", -5.0, 5.0, 0.0, Unit::Ev),
            signed("light.contrast".into(), "Contrast"),
            signed("light.highlights".into(), "Highlights"),
            signed("light.shadows".into(), "Shadows"),
            signed("light.whites".into(), "Whites"),
            signed("light.blacks".into(), "Blacks"),
            signed("wb.temperature".into(), "Temperature"),
            signed("wb.tint".into(), "Tint"),
            signed("presence.texture".into(), "Texture"),
            signed("presence.clarity".into(), "Clarity"),
            signed("presence.dehaze".into(), "Dehaze"),
            signed("presence.vibrance".into(), "Vibrance"),
            signed("presence.saturation".into(), "Saturation"),
            signed("curve.highlights".into(), "Highlights").prefixed("Tone curve"),
            signed("curve.lights".into(), "Lights").prefixed("Tone curve"),
            signed("curve.darks".into(), "Darks").prefixed("Tone curve"),
            signed("curve.shadows".into(), "Shadows").prefixed("Tone curve"),
            def("curve.split.shadows", "Shadow split", 10.0, 40.0, 25.0, Unit::Plain),
            def("curve.split.midtones", "Midtone split", 35.0, 65.0, 50.0, Unit::Plain),
            def("curve.split.highlights", "Highlight split", 60.0, 90.0, 75.0, Unit::Plain),
        ];
        for (part, label) in ["hue", "saturation", "luminance"].into_iter().zip(MIX_PARTS) {
            for (band, key) in BAND_KEYS.iter().enumerate() {
                list.push(signed(format!("mix.{key}.{part}"), label).prefixed(BANDS[band]));
            }
        }
        for (zone, key) in ZONE_KEYS.iter().enumerate() {
            let prefix = ZONES[zone];
            list.push(def(format!("grade.{key}.hue"), "Hue", 0.0, 360.0, 0.0, Unit::Hue).prefixed(prefix));
            list.push(plain(format!("grade.{key}.saturation"), "Saturation", 0.0).prefixed(prefix));
            list.push(signed(format!("grade.{key}.luminance"), "Luminance").prefixed(prefix));
        }
        list.push(plain("grade.blending".into(), "Blending", 50.0).prefixed("Color grading"));
        list.push(signed("grade.balance".into(), "Balance").prefixed("Color grading"));
        list.push(def("bw.enabled", "Black & White", 0.0, 1.0, 0.0, Unit::Toggle));
        for (band, key) in BAND_KEYS.iter().enumerate() {
            list.push(signed(format!("bw.{key}"), BANDS[band]).prefixed("B&W mix"));
        }
        list.extend([
            signed("vignette.amount".into(), "Amount").prefixed("Vignette"),
            plain("vignette.midpoint".into(), "Midpoint", 50.0).prefixed("Vignette"),
            signed("vignette.roundness".into(), "Roundness").prefixed("Vignette"),
            plain("vignette.feather".into(), "Feather", 50.0).prefixed("Vignette"),
            plain("vignette.highlights".into(), "Highlights", 0.0).prefixed("Vignette"),
            plain("grain.amount".into(), "Amount", 0.0).prefixed("Grain"),
            plain("grain.size".into(), "Size", 25.0).prefixed("Grain"),
            plain("grain.roughness".into(), "Roughness", 50.0).prefixed("Grain"),
            def("sharpen.amount", "Amount", 0.0, 150.0, 0.0, Unit::Plain).prefixed("Sharpening"),
            def("sharpen.radius", "Radius", 0.5, 3.0, 1.0, Unit::Radius).prefixed("Sharpening"),
            plain("sharpen.detail".into(), "Detail", 25.0).prefixed("Sharpening"),
            plain("sharpen.masking".into(), "Masking", 0.0).prefixed("Sharpening"),
            plain("noise.luminance".into(), "Luminance", 0.0).prefixed("Noise reduction"),
            plain("noise.color".into(), "Color", 0.0).prefixed("Noise reduction"),
            def("geometry.angle", "Straighten", -45.0, 45.0, 0.0, Unit::Degrees),
            def("geometry.flip_h", "Flip horizontal", 0.0, 1.0, 0.0, Unit::Toggle),
            def("geometry.flip_v", "Flip vertical", 0.0, 1.0, 0.0, Unit::Toggle),
            def("crop.left", "Crop left", 0.0, 1.0, 0.0, Unit::Fraction),
            def("crop.top", "Crop top", 0.0, 1.0, 0.0, Unit::Fraction),
            def("crop.right", "Crop right", 0.0, 1.0, 1.0, Unit::Fraction),
            def("crop.bottom", "Crop bottom", 0.0, 1.0, 1.0, Unit::Fraction),
            def("profile", "Profile", 0.0, (PROFILES.len() - 1) as f32, 0.0, Unit::Choice),
        ]);
        debug_assert_eq!(list.len(), COUNT);
        list
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Curve(pub Vec<[f32; 2]>);

impl Default for Curve {
    fn default() -> Self {
        Curve(vec![[0.0, 0.0], [1.0, 1.0]])
    }
}

impl Curve {
    pub fn is_identity(&self) -> bool {
        self.0.iter().all(|[x, y]| (x - y).abs() < 1e-4)
    }

    fn to_json(&self) -> Value {
        json!(self.0)
    }

    fn from_json(value: &Value) -> Option<Curve> {
        let points: Vec<[f32; 2]> = value
            .as_array()?
            .iter()
            .filter_map(|point| {
                let pair = point.as_array()?;
                Some([pair.first()?.as_f64()? as f32, pair.get(1)?.as_f64()? as f32])
            })
            .collect();
        (points.len() >= 2).then_some(Curve(points))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Full effect at `from`, fading to none at `to`.
    Linear { from: [f32; 2], to: [f32; 2] },
    /// Ellipse around `center`; `radius` is in canvas fractions and
    /// `feather` (0…1) softens its edge.
    Radial { center: [f32; 2], radius: [f32; 2], feather: f32 },
}

/// A local adjustment. Positions are fractions of the straightened canvas,
/// so a crop doesn't move the mask.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub name: String,
    pub shape: Shape,
    pub invert: bool,
    pub amounts: [f32; 9],
}

impl Mask {
    pub fn local_range(local: usize) -> (f32, f32) {
        if local == LOCAL_EXPOSURE {
            (-4.0, 4.0)
        } else {
            (-100.0, 100.0)
        }
    }

    fn to_json(&self) -> Value {
        let mut object = match self.shape {
            Shape::Linear { from, to } => json!({ "kind": "linear", "from": from, "to": to }),
            Shape::Radial { center, radius, feather } => {
                json!({ "kind": "radial", "center": center, "radius": radius, "feather": feather })
            }
        };
        let amounts: Map<String, Value> = LOCAL_KEYS
            .iter()
            .zip(self.amounts)
            .filter(|(_, amount)| *amount != 0.0)
            .map(|(key, amount)| ((*key).to_string(), json!(amount)))
            .collect();
        object["name"] = json!(self.name);
        object["invert"] = json!(self.invert);
        object["amounts"] = Value::Object(amounts);
        object
    }

    fn from_json(value: &Value) -> Option<Mask> {
        let pair = |key: &str| -> Option<[f32; 2]> {
            let array = value[key].as_array()?;
            Some([array.first()?.as_f64()? as f32, array.get(1)?.as_f64()? as f32])
        };
        let shape = match value["kind"].as_str()? {
            "linear" => Shape::Linear {
                from: pair("from")?,
                to: pair("to")?,
            },
            "radial" => Shape::Radial {
                center: pair("center")?,
                radius: pair("radius")?,
                feather: value["feather"].as_f64().unwrap_or(0.5) as f32,
            },
            _ => return None,
        };
        let mut amounts = [0.0; 9];
        for (slot, key) in amounts.iter_mut().zip(LOCAL_KEYS) {
            *slot = value["amounts"][key].as_f64().unwrap_or(0.0) as f32;
        }
        Some(Mask {
            name: value["name"].as_str().unwrap_or_default().to_string(),
            shape,
            invert: value["invert"].as_bool().unwrap_or(false),
            amounts,
        })
    }
}

/// Every value of one edit. The default is the untouched photo.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    values: Vec<f32>,
    /// Point curves for RGB, red, green, and blue.
    pub curves: [Curve; 4],
    pub masks: Vec<Mask>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            values: defs().iter().map(|def| def.default).collect(),
            curves: Default::default(),
            masks: Vec::new(),
        }
    }
}

impl Settings {
    pub fn get(&self, control: Control) -> f32 {
        self.values.get(control.0 as usize).copied().unwrap_or(0.0)
    }

    pub fn set(&mut self, control: Control, value: f32) {
        let def = control.def();
        let mut value = if value.is_finite() { value.clamp(def.min, def.max) } else { def.default };
        if matches!(def.unit, Unit::Toggle | Unit::Choice) {
            value = value.round();
        }
        if let Some(slot) = self.values.get_mut(control.0 as usize) {
            *slot = value;
        }
    }

    pub fn is_identity(&self) -> bool {
        Control::all().all(|control| self.get(control) == control.neutral())
            && self.curves.iter().all(Curve::is_identity)
            && self.masks.is_empty()
    }

    /// Left, top, right, and bottom edges of the crop, as canvas fractions.
    pub fn crop(&self) -> [f32; 4] {
        [
            self.get(Control::CROP_LEFT),
            self.get(Control::CROP_TOP),
            self.get(Control::CROP_RIGHT),
            self.get(Control::CROP_BOTTOM),
        ]
    }

    pub fn set_crop(&mut self, [left, top, right, bottom]: [f32; 4]) {
        self.set(Control::CROP_LEFT, left);
        self.set(Control::CROP_TOP, top);
        self.set(Control::CROP_RIGHT, right);
        self.set(Control::CROP_BOTTOM, bottom);
    }

    /// The settings with straighten, flips, and crop back to none, as copied
    /// to other photos.
    pub fn without_geometry(&self) -> Settings {
        let mut copy = self.clone();
        for control in Control::all().filter(|control| control.is_geometry()) {
            copy.set(control, control.neutral());
        }
        copy
    }

    /// `self` with the straighten, flips, and crop of `other`, as pasted.
    pub fn with_geometry_of(&self, other: &Settings) -> Settings {
        let mut copy = self.clone();
        for control in Control::all().filter(|control| control.is_geometry()) {
            copy.set(control, other.get(control));
        }
        copy
    }

    /// Values of `self` that differ from `base`.
    pub fn changed_from(&self, base: &Settings) -> Map<String, Value> {
        let mut patch = Map::new();
        for control in Control::all() {
            if self.get(control) != base.get(control) {
                patch.insert(control.key().to_string(), json!(self.get(control)));
            }
        }
        for (index, key) in CURVE_KEYS.iter().enumerate() {
            if self.curves[index] != base.curves[index] {
                patch.insert((*key).to_string(), self.curves[index].to_json());
            }
        }
        if self.masks != base.masks {
            patch.insert("masks".into(), Value::Array(self.masks.iter().map(Mask::to_json).collect()));
        }
        patch
    }

    pub fn apply_patch(&mut self, patch: &Map<String, Value>) {
        for (key, value) in patch {
            if let Some(control) = Control::from_key(key) {
                if let Some(number) = value.as_f64() {
                    self.set(control, number as f32);
                }
            } else if let Some(index) = CURVE_KEYS.iter().position(|curve| curve == key) {
                if let Some(curve) = Curve::from_json(value) {
                    self.curves[index] = curve;
                }
            } else if key == "masks" {
                self.masks = value.as_array().map(|masks| masks.iter().filter_map(Mask::from_json).collect()).unwrap_or_default();
            }
        }
    }
}

/// What a step did, for its line in the history.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Adjust,
    /// Every value back to the untouched photo.
    Reset,
    Preset(String),
    Profile,
    WhiteBalance(String),
    Paste,
    AutoTone,
    AutoWhiteBalance,
    Picker,
    Curve,
    Crop,
    Mask,
}

impl Kind {
    fn code(&self) -> String {
        match self {
            Kind::Adjust => "adjust".into(),
            Kind::Reset => "reset".into(),
            Kind::Preset(name) => format!("preset:{name}"),
            Kind::Profile => "profile".into(),
            Kind::WhiteBalance(name) => format!("wb:{name}"),
            Kind::Paste => "paste".into(),
            Kind::AutoTone => "auto-tone".into(),
            Kind::AutoWhiteBalance => "auto-wb".into(),
            Kind::Picker => "picker".into(),
            Kind::Curve => "curve".into(),
            Kind::Crop => "crop".into(),
            Kind::Mask => "mask".into(),
        }
    }

    fn parse(code: &str) -> Kind {
        if let Some(name) = code.strip_prefix("preset:") {
            return Kind::Preset(name.into());
        }
        if let Some(name) = code.strip_prefix("wb:") {
            return Kind::WhiteBalance(name.into());
        }
        match code {
            "reset" => Kind::Reset,
            "profile" => Kind::Profile,
            "paste" => Kind::Paste,
            "auto-tone" => Kind::AutoTone,
            "auto-wb" => Kind::AutoWhiteBalance,
            "picker" => Kind::Picker,
            "curve" => Kind::Curve,
            "crop" => Kind::Crop,
            "mask" => Kind::Mask,
            _ => Kind::Adjust,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub kind: Kind,
    /// Values the step set. A reset starts from the untouched photo.
    pub patch: Map<String, Value>,
}

impl Step {
    /// Line shown in the history list.
    pub fn label(&self) -> String {
        match &self.kind {
            Kind::Adjust => {
                if self.patch.len() == 1 {
                    if let Some((control, value)) = self
                        .patch
                        .iter()
                        .next()
                        .and_then(|(key, value)| Some((Control::from_key(key)?, value.as_f64()? as f32)))
                    {
                        return format!("{}  {}", control.label(), control.format(value));
                    }
                }
                trf("{} adjustments", &[&self.patch.len()])
            }
            Kind::Reset => tr("Reset all").into(),
            Kind::Preset(name) => trf("Preset: {}", &[&super::presets::display_name(name)]),
            Kind::Profile => {
                let index = self.patch.get(Control::PROFILE.key()).and_then(Value::as_f64).unwrap_or(0.0);
                trf("Profile: {}", &[&Control::PROFILE.format(index as f32)])
            }
            Kind::WhiteBalance(name) => trf("White balance: {}", &[&super::presets::white_balance_name(name)]),
            Kind::Paste => tr("Paste settings").into(),
            Kind::AutoTone => tr("Auto tone").into(),
            Kind::AutoWhiteBalance => tr("Auto white balance").into(),
            Kind::Picker => tr("White balance picker").into(),
            Kind::Curve => tr("Tone curve").into(),
            Kind::Crop => tr("Crop & straighten").into(),
            Kind::Mask => tr("Masks").into(),
        }
    }
}

/// Edit steps of one photo and how many of them are applied. Steps past
/// `applied` were undone and can be redone until a new step replaces them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    steps: Vec<Step>,
    applied: usize,
}

impl History {
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn applied(&self) -> usize {
        self.applied
    }

    pub fn settings(&self) -> Settings {
        self.settings_at(self.applied)
    }

    /// Settings after the first `count` steps.
    pub fn settings_at(&self, count: usize) -> Settings {
        let mut settings = Settings::default();
        for step in self.steps.iter().take(count) {
            if step.kind == Kind::Reset {
                settings = Settings::default();
            }
            settings.apply_patch(&step.patch);
        }
        settings
    }

    /// Records the change to `after` as a new step after the applied ones,
    /// dropping any undone steps. Nothing is recorded when nothing changes.
    pub fn commit(&mut self, kind: Kind, after: &Settings) -> bool {
        let before = self.settings();
        if *after == before {
            return false;
        }
        let base = if kind == Kind::Reset { Settings::default() } else { before };
        let patch = after.changed_from(&base);
        self.steps.truncate(self.applied);
        self.steps.push(Step { kind, patch });
        self.applied = self.steps.len();
        true
    }

    /// Like [`History::commit`], but a change of the same kind as the newest
    /// step replaces it, as a preset's amount is tuned.
    pub fn amend(&mut self, kind: Kind, after: &Settings) -> bool {
        let last = self.applied.checked_sub(1).filter(|_| self.applied == self.steps.len());
        let Some(last) = last.filter(|last| self.steps[*last].kind == kind) else {
            return self.commit(kind, after);
        };
        let base = self.settings_at(last);
        self.steps.truncate(last);
        self.applied = last;
        if *after != base {
            self.steps.push(Step {
                patch: after.changed_from(&base),
                kind,
            });
            self.applied = self.steps.len();
        }
        true
    }

    /// Values set by Auto tone or Auto white balance that no later applied
    /// step changed.
    pub fn auto_values(&self) -> Vec<(Control, f32)> {
        let mut auto: Vec<(Control, f32)> = Vec::new();
        for step in self.steps.iter().take(self.applied) {
            if step.kind == Kind::Reset {
                auto.clear();
            }
            let by_auto = matches!(step.kind, Kind::AutoTone | Kind::AutoWhiteBalance);
            for (key, value) in &step.patch {
                let Some(control) = Control::from_key(key) else { continue };
                auto.retain(|(other, _)| *other != control);
                if let Some(value) = value.as_f64().filter(|_| by_auto) {
                    auto.push((control, value as f32));
                }
            }
        }
        auto
    }

    /// Name of the newest applied step when it is a preset nothing followed.
    pub fn last_preset(&self) -> Option<&str> {
        let step = self.steps.last().filter(|_| self.applied == self.steps.len())?;
        match &step.kind {
            Kind::Preset(name) => Some(name),
            _ => None,
        }
    }

    pub fn can_undo(&self) -> bool {
        self.applied > 0
    }

    pub fn can_redo(&self) -> bool {
        self.applied < self.steps.len()
    }

    pub fn undo(&mut self) {
        self.applied = self.applied.saturating_sub(1);
    }

    pub fn redo(&mut self) {
        self.applied = (self.applied + 1).min(self.steps.len());
    }

    pub fn undo_all(&mut self) {
        self.applied = 0;
    }

    pub fn redo_all(&mut self) {
        self.applied = self.steps.len();
    }

    /// Applies exactly the first `count` steps.
    pub fn jump(&mut self, count: usize) {
        self.applied = count.min(self.steps.len());
    }

    /// The steps as JSON: `[{"kind":"adjust","patch":{"light.exposure":0.5}}]`.
    pub fn steps_json(&self) -> String {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|step| json!({ "kind": step.kind.code(), "patch": Value::Object(step.patch.clone()) }))
            .collect();
        Value::Array(steps).to_string()
    }

    /// Reads [`History::steps_json`], and the single-slider steps of earlier
    /// versions. Unknown entries are skipped.
    pub fn from_json(text: &str, applied: usize) -> History {
        let steps: Vec<Step> = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|entry| {
                if let Some(kind) = entry["kind"].as_str() {
                    let patch = entry["patch"].as_object().cloned().unwrap_or_default();
                    return Some(Step {
                        kind: Kind::parse(kind),
                        patch,
                    });
                }
                if entry["reset"].as_bool() == Some(true) {
                    return Some(Step {
                        kind: Kind::Reset,
                        patch: Map::new(),
                    });
                }
                let key = legacy_key(entry["control"].as_str()?)?;
                let mut patch = Map::new();
                patch.insert(key.into(), json!(entry["value"].as_f64()?));
                Some(Step {
                    kind: Kind::Adjust,
                    patch,
                })
            })
            .collect();
        let applied = applied.min(steps.len());
        History { steps, applied }
    }
}

/// Keys of the first editor, which stored one slider per step.
fn legacy_key(key: &str) -> Option<&'static str> {
    Some(match key {
        "exposure" => "light.exposure",
        "contrast" => "light.contrast",
        "highlights" => "light.highlights",
        "shadows" => "light.shadows",
        "whites" => "light.whites",
        "blacks" => "light.blacks",
        "temperature" => "wb.temperature",
        "tint" => "wb.tint",
        "vibrance" => "presence.vibrance",
        "saturation" => "presence.saturation",
        "monochrome" => "bw.enabled",
        "vignette" => "vignette.amount",
        "grain" => "grain.amount",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(control: Control, value: f32, base: &Settings) -> Settings {
        let mut settings = base.clone();
        settings.set(control, value);
        settings
    }

    #[test]
    fn controls_line_up_with_their_keys() {
        assert_eq!(defs().len(), COUNT);
        assert_eq!(Control::EXPOSURE.key(), "light.exposure");
        assert_eq!(Control::mix(0, 0).key(), "mix.red.hue");
        assert_eq!(Control::mix(2, 7).key(), "mix.magenta.luminance");
        assert_eq!(Control::grade(3, 2).key(), "grade.global.luminance");
        assert_eq!(Control::GRADE_BLENDING.key(), "grade.blending");
        assert_eq!(Control::MONOCHROME.key(), "bw.enabled");
        assert_eq!(Control::bw(7).key(), "bw.magenta");
        assert_eq!(Control::VIGNETTE_AMOUNT.key(), "vignette.amount");
        assert_eq!(Control::NOISE_COLOR.key(), "noise.color");
        assert_eq!(Control::CROP_BOTTOM.key(), "crop.bottom");
        assert_eq!(Control::PROFILE.key(), "profile");
        assert!(Settings::default().is_identity());
    }

    #[test]
    fn undo_redo_and_new_steps_after_undo() {
        let mut history = History::default();
        let start = Settings::default();
        assert!(history.commit(Kind::Adjust, &with(Control::EXPOSURE, 1.0, &start)));
        let both = with(Control::CONTRAST, 20.0, &history.settings());
        assert!(history.commit(Kind::Adjust, &both));
        assert!(!history.commit(Kind::Adjust, &both));
        history.undo();
        assert_eq!(history.settings().get(Control::CONTRAST), 0.0);
        history.redo_all();
        assert_eq!(history.settings().get(Control::CONTRAST), 20.0);
        history.undo_all();
        assert!(history.settings().is_identity());
        history.redo();
        assert!(history.commit(Kind::Adjust, &with(Control::SHADOWS, 30.0, &history.settings())));
        assert_eq!(history.steps().len(), 2, "the undone contrast step is replaced");
        assert!(!history.can_redo());
        assert!(history.commit(Kind::Reset, &Settings::default()));
        assert!(history.settings().is_identity());
        assert_eq!(history.settings_at(2).get(Control::SHADOWS), 30.0);
    }

    #[test]
    fn auto_values_last_until_changed_by_hand() {
        let mut history = History::default();
        let mut tone = with(Control::EXPOSURE, 0.4, &Settings::default());
        tone.set(Control::CONTRAST, 12.0);
        history.commit(Kind::AutoTone, &tone);
        let balanced = with(Control::TEMPERATURE, -8.0, &tone);
        history.commit(Kind::AutoWhiteBalance, &balanced);
        let auto = |history: &History| {
            let mut values = history.auto_values();
            values.sort_by_key(|(control, _)| control.0);
            values
        };
        let mut expected = vec![(Control::EXPOSURE, 0.4), (Control::CONTRAST, 12.0), (Control::TEMPERATURE, -8.0)];
        expected.sort_by_key(|(control, _)| control.0);
        assert_eq!(auto(&history), expected);
        history.commit(Kind::Adjust, &with(Control::CONTRAST, 20.0, &balanced));
        assert!(!auto(&history).iter().any(|(control, _)| *control == Control::CONTRAST));
        assert_eq!(auto(&history).len(), 2);
        history.undo();
        assert_eq!(auto(&history), expected);
        history.commit(Kind::Reset, &Settings::default());
        assert!(history.auto_values().is_empty());
    }

    #[test]
    fn presets_amend_their_own_step() {
        let mut history = History::default();
        let warm = with(Control::TEMPERATURE, 30.0, &Settings::default());
        assert!(history.commit(Kind::Preset("Golden Hour".into()), &warm));
        let warmer = with(Control::TEMPERATURE, 60.0, &Settings::default());
        assert!(history.amend(Kind::Preset("Golden Hour".into()), &warmer));
        assert_eq!(history.steps().len(), 1);
        assert_eq!(history.settings().get(Control::TEMPERATURE), 60.0);
        assert_eq!(history.last_preset(), Some("Golden Hour"));
    }

    #[test]
    fn history_survives_json_with_curves_and_masks() {
        let mut history = History::default();
        let mut settings = with(Control::EXPOSURE, 0.5, &Settings::default());
        history.commit(Kind::Adjust, &settings);
        settings.curves[0] = Curve(vec![[0.0, 0.1], [0.5, 0.6], [1.0, 1.0]]);
        history.commit(Kind::Curve, &settings);
        settings.masks.push(Mask {
            name: "Sky".into(),
            shape: Shape::Linear {
                from: [0.5, 0.0],
                to: [0.5, 0.5],
            },
            invert: false,
            amounts: [-0.5, 0.0, -30.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0],
        });
        history.commit(Kind::Mask, &settings);
        history.commit(Kind::Reset, &Settings::default());
        history.undo();
        let back = History::from_json(&history.steps_json(), history.applied());
        assert_eq!(back, history);
        assert_eq!(back.settings(), settings);
        assert_eq!(History::from_json("not json", 3), History::default());
    }

    #[test]
    fn reads_steps_of_the_first_editor() {
        let old = r#"[{"control":"exposure","value":0.5},{"control":"monochrome","value":1.0},{"reset":true}]"#;
        let history = History::from_json(old, 2);
        assert_eq!(history.steps().len(), 3);
        assert_eq!(history.settings().get(Control::EXPOSURE), 0.5);
        assert_eq!(history.settings().get(Control::MONOCHROME), 1.0);
        assert!(history.settings_at(3).is_identity());
    }
}
