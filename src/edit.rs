//! Non-destructive editing. A photo's edit is a list of steps stored in the
//! library; the original file is never written. The applied steps are
//! replayed onto the preview, the thumbnails, and an exported copy.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use crossbeam_channel::{Receiver, Sender};
use image::RgbaImage;
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::i18n::tr;
use crate::worker::UiEvent;

/// Long edge of the image the editor previews on.
pub const PREVIEW_EDGE: u32 = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Temperature,
    Tint,
    Vibrance,
    Saturation,
    Monochrome,
    Vignette,
    Grain,
}

const COUNT: usize = 13;

/// Panel sections and the sliders in each, in display order.
pub const GROUPS: [(&str, &[Control]); 3] = [
    (
        "Light",
        &[Control::Exposure, Control::Contrast, Control::Highlights, Control::Shadows, Control::Whites, Control::Blacks],
    ),
    ("Color", &[Control::Temperature, Control::Tint, Control::Vibrance, Control::Saturation]),
    ("Effects", &[Control::Vignette, Control::Grain]),
];

impl Control {
    pub const ALL: [Control; COUNT] = [
        Control::Exposure,
        Control::Contrast,
        Control::Highlights,
        Control::Shadows,
        Control::Whites,
        Control::Blacks,
        Control::Temperature,
        Control::Tint,
        Control::Vibrance,
        Control::Saturation,
        Control::Monochrome,
        Control::Vignette,
        Control::Grain,
    ];

    /// Name stored in the library.
    pub fn key(self) -> &'static str {
        match self {
            Control::Exposure => "exposure",
            Control::Contrast => "contrast",
            Control::Highlights => "highlights",
            Control::Shadows => "shadows",
            Control::Whites => "whites",
            Control::Blacks => "blacks",
            Control::Temperature => "temperature",
            Control::Tint => "tint",
            Control::Vibrance => "vibrance",
            Control::Saturation => "saturation",
            Control::Monochrome => "monochrome",
            Control::Vignette => "vignette",
            Control::Grain => "grain",
        }
    }

    /// English label, translated with `tr` for display.
    pub fn label(self) -> &'static str {
        match self {
            Control::Exposure => "Exposure",
            Control::Contrast => "Contrast",
            Control::Highlights => "Highlights",
            Control::Shadows => "Shadows",
            Control::Whites => "Whites",
            Control::Blacks => "Blacks",
            Control::Temperature => "Temperature",
            Control::Tint => "Tint",
            Control::Vibrance => "Vibrance",
            Control::Saturation => "Saturation",
            Control::Monochrome => "Black & White",
            Control::Vignette => "Vignette",
            Control::Grain => "Grain",
        }
    }

    pub fn range(self) -> (f32, f32) {
        match self {
            Control::Exposure => (-4.0, 4.0),
            Control::Grain => (0.0, 100.0),
            Control::Monochrome => (0.0, 1.0),
            _ => (-100.0, 100.0),
        }
    }

    pub fn format(self, value: f32) -> String {
        match self {
            Control::Exposure => format!("{value:+.2} EV"),
            Control::Monochrome => tr(if value >= 0.5 { "On" } else { "Off" }).to_string(),
            _ if value.round() == 0.0 => "0".into(),
            Control::Grain => format!("{value:.0}"),
            _ => format!("{value:+.0}"),
        }
    }

    fn from_key(key: &str) -> Option<Control> {
        Control::ALL.into_iter().find(|control| control.key() == key)
    }
}

/// Value of every control; all zero is the untouched photo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Settings([f32; COUNT]);

impl Settings {
    pub fn get(&self, control: Control) -> f32 {
        self.0[control as usize]
    }

    pub fn set(&mut self, control: Control, value: f32) {
        let (min, max) = control.range();
        self.0[control as usize] = value.clamp(min, max);
    }

    pub fn is_identity(&self) -> bool {
        self.0.iter().all(|value| *value == 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Set(Control, f32),
    /// Every control back to zero.
    Reset,
}

impl Step {
    /// Line shown in the history list.
    pub fn label(&self) -> String {
        match self {
            Step::Set(control, value) => format!("{}  {}", tr(control.label()), control.format(*value)),
            Step::Reset => tr("Reset all").to_string(),
        }
    }

    fn apply(&self, settings: &mut Settings) {
        match self {
            Step::Set(control, value) => settings.set(*control, *value),
            Step::Reset => *settings = Settings::default(),
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
            step.apply(&mut settings);
        }
        settings
    }

    /// Adds `step` after the applied ones, dropping any undone steps. A step
    /// that changes nothing is not recorded.
    pub fn push(&mut self, step: Step) -> bool {
        let before = self.settings();
        let mut after = before;
        step.apply(&mut after);
        if after == before {
            return false;
        }
        self.steps.truncate(self.applied);
        self.steps.push(step);
        self.applied = self.steps.len();
        true
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

    /// The steps as JSON, for example
    /// `[{"control":"exposure","value":0.5},{"reset":true}]`.
    pub fn steps_json(&self) -> String {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Set(control, value) => json!({ "control": control.key(), "value": value }),
                Step::Reset => json!({ "reset": true }),
            })
            .collect();
        Value::Array(steps).to_string()
    }

    /// Reads steps written by [`History::steps_json`]. Unknown entries are skipped.
    pub fn from_json(text: &str, applied: usize) -> History {
        let steps: Vec<Step> = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|entry| {
                if entry["reset"].as_bool() == Some(true) {
                    return Some(Step::Reset);
                }
                let control = Control::from_key(entry["control"].as_str()?)?;
                Some(Step::Set(control, entry["value"].as_f64()? as f32))
            })
            .collect();
        let applied = applied.min(steps.len());
        History { steps, applied }
    }
}

/// Renders `settings` onto `image` in place. Light and color work on linear
/// light, so exposure behaves like more or less light reaching the sensor.
pub fn apply(image: &mut RgbaImage, settings: &Settings) {
    if settings.is_identity() {
        return;
    }
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return;
    }
    let plan = Plan::new(settings, width, height);
    image
        .as_mut()
        .par_chunks_mut(width as usize * 4)
        .enumerate()
        .for_each(|(y, row)| plan.row(y as u32, row));
}

const TONE_STEPS: usize = 4096;
/// Brightest linear luminance the tone table covers (+4 EV over white).
const TONE_MAX: f32 = 16.0;
const ENCODE_STEPS: usize = 16384;

struct Plan {
    /// White balance and exposure as one gain per channel.
    gain: [f32; 3],
    /// Luminance gain from the tone sliders, indexed by `sqrt(L / TONE_MAX)`.
    tone: Vec<f32>,
    saturation: f32,
    vibrance: f32,
    monochrome: bool,
    vignette: f32,
    grain: f32,
    grain_cell: f32,
    width: f32,
    height: f32,
}

impl Plan {
    fn new(settings: &Settings, width: u32, height: u32) -> Plan {
        let value = |control: Control| settings.get(control) / 100.0;
        let temperature = value(Control::Temperature);
        let tint = value(Control::Tint);
        let mut gain = [
            (1.0 + 0.30 * temperature) * (1.0 + 0.12 * tint),
            1.0 - 0.25 * tint,
            (1.0 - 0.30 * temperature) * (1.0 + 0.12 * tint),
        ];
        let luma = luminance(gain[0], gain[1], gain[2]).max(1e-3);
        let exposure = settings.get(Control::Exposure);
        for channel in &mut gain {
            *channel *= exposure.exp2() / luma;
        }
        Plan {
            gain,
            tone: tone_table(settings),
            saturation: 1.0 + value(Control::Saturation),
            vibrance: value(Control::Vibrance),
            monochrome: settings.get(Control::Monochrome) >= 0.5,
            vignette: value(Control::Vignette),
            grain: value(Control::Grain),
            grain_cell: (width.max(height) as f32 / 1600.0).max(1.0),
            width: width as f32,
            height: height as f32,
        }
    }

    fn row(&self, y: u32, row: &mut [u8]) {
        let decode = decode_table();
        let encode = encode_table();
        let ny = (y as f32 + 0.5) / self.height * 2.0 - 1.0;
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let mut rgb = [
                decode[pixel[0] as usize] * self.gain[0],
                decode[pixel[1] as usize] * self.gain[1],
                decode[pixel[2] as usize] * self.gain[2],
            ];
            let lum = luminance(rgb[0], rgb[1], rgb[2]);
            let tone = self.tone_gain(lum);
            let lum = lum * tone;
            let max = rgb[0].max(rgb[1]).max(rgb[2]) * tone;
            let min = rgb[0].min(rgb[1]).min(rgb[2]) * tone;
            let chroma = if max > 1e-6 { (max - min) / max } else { 0.0 };
            let colorful = if self.monochrome {
                0.0
            } else {
                self.saturation * (1.0 + self.vibrance * (1.0 - chroma))
            };
            let nx = (x as f32 + 0.5) / self.width * 2.0 - 1.0;
            let shade = self.vignette_gain(nx, ny);
            for channel in &mut rgb {
                *channel = (lum + (*channel * tone - lum) * colorful).max(0.0) * shade;
            }
            let noise = if self.grain > 0.0 { self.noise(x as f32, y as f32) } else { 0.0 };
            for (out, channel) in pixel.iter_mut().zip(rgb) {
                let index = (channel.min(1.0).sqrt() * (ENCODE_STEPS - 1) as f32 + 0.5) as usize;
                let coded = encode[index.min(ENCODE_STEPS - 1)] as f32 + noise;
                *out = coded.round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    fn tone_gain(&self, lum: f32) -> f32 {
        let position = (lum.max(0.0) / TONE_MAX).sqrt().min(1.0) * (TONE_STEPS - 1) as f32;
        let index = position as usize;
        let next = (index + 1).min(TONE_STEPS - 1);
        let fraction = position - index as f32;
        self.tone[index] + (self.tone[next] - self.tone[index]) * fraction
    }

    fn vignette_gain(&self, nx: f32, ny: f32) -> f32 {
        if self.vignette == 0.0 {
            return 1.0;
        }
        let radius = ((nx * nx + ny * ny) / 2.0).sqrt();
        let weight = smoothstep(0.4, 1.0, radius);
        let strength = if self.vignette < 0.0 { 0.85 } else { 0.6 };
        (1.0 + self.vignette * strength * weight).max(0.0)
    }

    /// Film grain in 8-bit steps. The grain cell grows with the image, so a
    /// preview and a full-size export look alike.
    fn noise(&self, x: f32, y: f32) -> f32 {
        let cx = (x / self.grain_cell) as u32;
        let cy = (y / self.grain_cell) as u32;
        let mut hash = cx.wrapping_mul(0x8DA6_B343) ^ cy.wrapping_mul(0xD816_3841) ^ 0x9E37_79B9;
        hash ^= hash >> 13;
        hash = hash.wrapping_mul(0x5BD1_E995);
        hash ^= hash >> 15;
        let unit = (hash & 0xFFFF) as f32 / 65535.0 * 2.0 - 1.0;
        unit * self.grain * 26.0
    }
}

/// Luminance gain for the tone sliders. They act on a perceptual scale
/// (`L^(1/2.2)`) so that each slider moves the part of the range its name
/// promises: shadows peak at a third, highlights at two thirds.
fn tone_table(settings: &Settings) -> Vec<f32> {
    let value = |control: Control| settings.get(control) / 100.0;
    let contrast = value(Control::Contrast);
    let highlights = value(Control::Highlights);
    let shadows = value(Control::Shadows);
    let whites = value(Control::Whites);
    let blacks = value(Control::Blacks);
    // Brightened highlights roll off like film instead of clipping, blended
    // in with positive exposure so that untouched photos stay exact.
    let rolloff = settings.get(Control::Exposure).clamp(0.0, 1.0);
    let mut table: Vec<f32> = (0..TONE_STEPS)
        .map(|index| {
            let t = index as f32 / (TONE_STEPS - 1) as f32;
            let lum = t * t * TONE_MAX;
            let x = lum.powf(1.0 / 2.2);
            let soft = 0.8 + 0.2 * ((x - 0.8).max(0.0) / 0.2).tanh();
            let soft = if x < 0.8 { x } else { soft };
            let mut x = x.min(1.0) + (soft - x.min(1.0)) * rolloff;
            x += blacks * 0.18 * (1.0 - x).powi(3);
            x += shadows * 0.32 * 6.75 * x * (1.0 - x).powi(2);
            x += highlights * 0.32 * 6.75 * x * x * (1.0 - x);
            x += whites * 0.22 * x.powi(3);
            x = x.clamp(0.0, 1.0);
            if contrast > 0.0 {
                x += contrast * (smoothstep(0.0, 1.0, x) - x);
            } else {
                x += -contrast * 0.5 * (0.5 - x);
            }
            let out = x.clamp(0.0, 1.0).powf(2.2);
            if lum > 1e-7 {
                out / lum
            } else {
                f32::NAN
            }
        })
        .collect();
    table[0] = if table.len() > 1 { table[1] } else { 1.0 };
    table
}

fn luminance(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn decode_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|index| srgb_to_linear(index as f32 / 255.0)))
}

/// Linear light to sRGB codes, indexed by `sqrt(linear)` for precision in
/// the shadows.
fn encode_table() -> &'static [u8] {
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

pub enum EditJob {
    /// Decode the photo at preview size.
    Load { id: i64, path: String },
    Render {
        id: i64,
        seq: u64,
        base: Arc<RgbaImage>,
        settings: Settings,
    },
}

/// Editor thread: decodes preview images and renders edits onto them. While
/// a slider moves, only the newest render request is drawn.
pub fn edit_loop(jobs: Receiver<EditJob>, events: Sender<UiEvent>, ctx: eframe::egui::Context) {
    while let Ok(first) = jobs.recv() {
        let mut pending = vec![first];
        pending.extend(jobs.try_iter());
        let newest_render = pending.iter().rposition(|job| matches!(job, EditJob::Render { .. }));
        for (index, job) in pending.into_iter().enumerate() {
            if matches!(job, EditJob::Render { .. }) && Some(index) != newest_render {
                continue;
            }
            let event = match job {
                EditJob::Load { id, path } => load_preview(id, &path),
                EditJob::Render { id, seq, base, settings } => {
                    let mut image = (*base).clone();
                    apply(&mut image, &settings);
                    let (w, h) = (image.width() as usize, image.height() as usize);
                    UiEvent::EditPreview {
                        id,
                        seq,
                        w,
                        h,
                        rgba: image.into_raw(),
                    }
                }
            };
            if events.send(event).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    }
}

fn load_preview(id: i64, path: &str) -> UiEvent {
    let path = Path::new(path);
    let decoded = std::panic::catch_unwind(|| crate::image_io::load_oriented(path))
        .unwrap_or_else(|_| Err("decoder fault".into()));
    match decoded {
        Ok(image) => UiEvent::EditBase {
            id,
            image: Arc::new(crate::image_io::limit_rgba(image, PREVIEW_EDGE)),
        },
        Err(error) => UiEvent::EditFailed { id, error },
    }
}

/// Writes the edited photo at full size to `target`: JPEG, PNG, or TIFF by
/// extension. `rotation` is the clockwise turn stored in the catalog.
pub fn export(source: &Path, target: &Path, settings: &Settings, rotation: i32) -> Result<(), String> {
    if same_file(source, target) {
        return Err(tr("Choose a different name than the original").into());
    }
    let image = std::panic::catch_unwind(|| crate::image_io::load_oriented(source))
        .unwrap_or_else(|_| Err("decoder fault".into()))?;
    let mut rgba = image.to_rgba8();
    apply(&mut rgba, settings);
    let rgba = match rotation.rem_euclid(360) {
        90 => image::imageops::rotate90(&rgba),
        180 => image::imageops::rotate180(&rgba),
        270 => image::imageops::rotate270(&rgba),
        _ => rgba,
    };
    let extension = target
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => rgba.save_with_format(target, image::ImageFormat::Png).map_err(|err| err.to_string()),
        "tif" | "tiff" => rgba.save_with_format(target, image::ImageFormat::Tiff).map_err(|err| err.to_string()),
        _ => crate::image_io::save_jpeg(target, &rgba, 92),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// `photo.jpg` → `photo-edited.jpg` beside the original; a RAW or HEIC
/// source exports as JPEG.
pub fn export_name(source: &Path) -> String {
    let stem = source.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
    let extension = source
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .filter(|ext| matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "tif" | "tiff"))
        .unwrap_or_else(|| "jpg".into());
    format!("{stem}-edited.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_and_new_steps_after_undo() {
        let mut history = History::default();
        assert!(history.push(Step::Set(Control::Exposure, 1.0)));
        assert!(history.push(Step::Set(Control::Contrast, 20.0)));
        assert!(!history.push(Step::Set(Control::Contrast, 20.0)));
        history.undo();
        assert_eq!(history.settings().get(Control::Contrast), 0.0);
        history.redo_all();
        assert_eq!(history.settings().get(Control::Contrast), 20.0);
        history.undo_all();
        assert!(history.settings().is_identity());
        history.redo();
        assert!(history.push(Step::Set(Control::Shadows, 30.0)));
        assert_eq!(history.steps().len(), 2, "the undone contrast step is replaced");
        assert!(!history.can_redo());
        assert!(history.push(Step::Reset));
        assert!(history.settings().is_identity());
        assert_eq!(history.settings_at(2).get(Control::Shadows), 30.0);
    }

    #[test]
    fn history_survives_json() {
        let mut history = History::default();
        history.push(Step::Set(Control::Exposure, 0.5));
        history.push(Step::Set(Control::Monochrome, 1.0));
        history.push(Step::Reset);
        history.undo();
        let back = History::from_json(&history.steps_json(), history.applied());
        assert_eq!(back, history);
        assert_eq!(History::from_json("not json", 3), History::default());
    }

    #[test]
    fn sliders_change_pixels_as_named() {
        let gray = || RgbaImage::from_pixel(8, 8, image::Rgba([118, 118, 118, 255]));
        let render = |control: Control, value: f32| {
            let mut settings = Settings::default();
            settings.set(control, value);
            let mut image = gray();
            apply(&mut image, &settings);
            image.get_pixel(4, 4).0
        };
        assert!(render(Control::Exposure, 1.0)[0] > 150);
        assert!(render(Control::Exposure, -1.0)[0] < 100);
        let warm = render(Control::Temperature, 60.0);
        assert!(warm[0] > warm[2]);
        let toned = {
            let mut settings = Settings::default();
            settings.set(Control::Monochrome, 1.0);
            settings.set(Control::Temperature, 50.0);
            let mut image = RgbaImage::from_pixel(2, 2, image::Rgba([200, 60, 30, 255]));
            apply(&mut image, &settings);
            image.get_pixel(0, 0).0
        };
        assert!(toned[0] == toned[1] && toned[1] == toned[2]);
    }

    #[test]
    fn untouched_light_settings_keep_codes() {
        let decode = decode_table();
        let encode = encode_table();
        for code in 0..=255usize {
            let index = (decode[code].sqrt() * (ENCODE_STEPS - 1) as f32 + 0.5) as usize;
            assert_eq!(encode[index] as usize, code, "code {code}");
        }
        let mut settings = Settings::default();
        settings.set(Control::Vignette, 0.0);
        settings.set(Control::Grain, 0.0);
        settings.set(Control::Saturation, 0.0001);
        let mut image = RgbaImage::from_fn(16, 16, |x, y| image::Rgba([(x * 16) as u8, (y * 16) as u8, 77, 255]));
        let original = image.clone();
        apply(&mut image, &settings);
        assert_eq!(image, original);
    }
}
