//! The develop pipeline. A frame holds the straightened and cropped photo
//! in linear light with the analyses that don't depend on the sliders, so a
//! slider change only reruns the per-pixel pass.
//!
//! Order: noise reduction, dehaze, white balance and exposure, local
//! highlights and shadows, clarity, texture, and sharpening, the global
//! tone curve, mask contrast, the OkLab color stage (mixer, vibrance,
//! saturation, black & white, grading), vignette, encoding, point curves,
//! grain, and the clipping and mask overlays.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use image::RgbaImage;
use rayon::prelude::*;

use super::color::{self, decode_table, encode, from_oklab, hue_direction, luminance, smoothstep, to_oklab};
use super::curve;
use super::filters::{box_blur, guided, min_filter, shrink, Low};
use super::geometry::{warp, Geometry, FULL};
use super::presets;
use super::settings::*;

/// What a render shows besides the adjusted photo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Cut to the crop. Off in the crop tool, which shows the whole canvas.
    pub crop: bool,
    /// Off for the before view: only straighten, flips, and crop.
    pub adjust: bool,
    /// Paint clipped highlights red and clipped shadows blue.
    pub clipping: bool,
    /// Tint the area of this mask.
    pub overlay: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            crop: true,
            adjust: true,
            clipping: false,
            overlay: None,
        }
    }
}

type Key = (usize, u32, u32, [u32; 7]);

/// The frame of the last render, reused while the photo and its geometry
/// stay the same. Clear it when the photo changes.
#[derive(Default)]
pub struct Cache {
    key: Option<Key>,
    frame: Option<Arc<Frame>>,
}

impl Cache {
    pub fn clear(&mut self) {
        self.key = None;
        self.frame = None;
    }

    fn frame(&mut self, source: &RgbaImage, geometry: &Geometry) -> Arc<Frame> {
        let g = geometry;
        let key = (
            source.as_raw().as_ptr() as usize,
            source.width(),
            source.height(),
            [
                g.angle.to_bits(),
                u32::from(g.flip_h),
                u32::from(g.flip_v),
                g.crop[0].to_bits(),
                g.crop[1].to_bits(),
                g.crop[2].to_bits(),
                g.crop[3].to_bits(),
            ],
        );
        match &self.frame {
            Some(frame) if self.key == Some(key) => frame.clone(),
            _ => {
                let frame = Arc::new(Frame::new(source, geometry));
                self.key = Some(key);
                self.frame = Some(frame.clone());
                frame
            }
        }
    }
}

pub struct Frame {
    w: usize,
    h: usize,
    /// Size of the whole canvas, in pixels.
    canvas: [f32; 2],
    /// Part of the canvas the frame covers, in fractions.
    view: [f32; 4],
    rgb: Vec<[f32; 3]>,
    /// log2 of the luminance.
    log: Vec<f32>,
    base: OnceLock<Low>,
    fine: OnceLock<Vec<f32>>,
    haze: OnceLock<Haze>,
}

struct Haze {
    light: [f32; 3],
    transmission: Low,
}

const LOG_FLOOR: f32 = 1.0 / 4096.0;

impl Frame {
    fn new(source: &RgbaImage, geometry: &Geometry) -> Frame {
        let warped: Cow<RgbaImage> = if geometry.is_identity() {
            Cow::Borrowed(source)
        } else {
            Cow::Owned(warp(source, geometry))
        };
        let decode = decode_table();
        let rgb: Vec<[f32; 3]> = warped
            .as_raw()
            .par_chunks_exact(4)
            .map(|p| [decode[p[0] as usize], decode[p[1] as usize], decode[p[2] as usize]])
            .collect();
        let log = rgb.par_iter().map(|rgb| (luminance(*rgb) + LOG_FLOOR).log2()).collect();
        Frame {
            w: warped.width() as usize,
            h: warped.height() as usize,
            canvas: [source.width() as f32, source.height() as f32],
            view: geometry.crop,
            rgb,
            log,
            base: OnceLock::new(),
            fine: OnceLock::new(),
            haze: OnceLock::new(),
        }
    }

    /// Edge-aware large-scale luminance, the base local tone works on.
    fn base(&self) -> &Low {
        self.base.get_or_init(|| {
            let (small, w, h) = shrink(&self.log, self.w, self.h, 1024);
            let radius = ((w.max(h) as f32 * 0.03).round() as usize).max(2);
            Low {
                data: guided(&small, w, h, radius, 0.35),
                w,
                h,
            }
        })
    }

    /// Small-scale blur of the luminance, the base texture works on.
    fn fine(&self) -> &[f32] {
        self.fine.get_or_init(|| {
            let radius = ((self.w.max(self.h) as f32 * 0.0025).round() as usize).max(1);
            box_blur(&box_blur(&self.log, self.w, self.h, radius), self.w, self.h, radius)
        })
    }

    /// Dark channel prior: the haze color and how much of the scene shows
    /// through it.
    fn haze(&self) -> &Haze {
        self.haze.get_or_init(|| {
            let plane = |channel: usize| -> Vec<f32> { self.rgb.par_iter().map(|rgb| rgb[channel]).collect() };
            let (r, w, h) = shrink(&plane(0), self.w, self.h, 512);
            let (g, _, _) = shrink(&plane(1), self.w, self.h, 512);
            let (b, _, _) = shrink(&plane(2), self.w, self.h, 512);
            let radius = (w.max(h) / 70).max(1);
            let dark: Vec<f32> = (0..r.len()).map(|i| r[i].min(g[i]).min(b[i])).collect();
            let dark = min_filter(&dark, w, h, radius);
            let mut order: Vec<usize> = (0..dark.len()).collect();
            order.sort_unstable_by(|a, b| dark[*b].total_cmp(&dark[*a]));
            let brightest = &order[..(order.len() / 1000).max(1)];
            let mut light = [0.0f32; 3];
            for &i in brightest {
                light[0] += r[i];
                light[1] += g[i];
                light[2] += b[i];
            }
            let light = light.map(|sum| (sum / brightest.len() as f32).clamp(0.05, 1.0));
            let normalized: Vec<f32> = (0..r.len())
                .map(|i| (r[i] / light[0]).min(g[i] / light[1]).min(b[i] / light[2]))
                .collect();
            let transmission: Vec<f32> = min_filter(&normalized, w, h, radius)
                .into_iter()
                .map(|dark| 1.0 - 0.95 * dark.min(1.0))
                .collect();
            let transmission = guided(&transmission, w, h, radius * 2, 0.01)
                .into_iter()
                .map(|t| t.clamp(0.05, 1.0))
                .collect();
            Haze {
                light,
                transmission: Low { data: transmission, w, h },
            }
        })
    }
}

/// `settings` with the profile's offsets added.
pub fn effective(settings: &Settings) -> Settings {
    let profile = settings.get(Control::PROFILE).round() as usize;
    if profile == 0 {
        return settings.clone();
    }
    let mut out = settings.clone();
    for (control, offset) in presets::profile_offsets(profile) {
        out.set(control, out.get(control) + offset);
    }
    out
}

/// Part of the canvas a render with `options` shows, in fractions.
pub fn view(settings: &Settings, options: &Options) -> [f32; 4] {
    if options.crop {
        settings.crop()
    } else {
        FULL
    }
}

/// The photo with every edit in `settings` applied. Overlays are left to
/// [`decorate`].
pub fn render(source: &RgbaImage, settings: &Settings, options: &Options, cache: Option<&mut Cache>) -> RgbaImage {
    let mut geometry = Geometry::from_settings(settings);
    geometry.crop = view(settings, options);
    if !options.adjust {
        return if geometry.is_identity() { source.clone() } else { warp(source, &geometry) };
    }
    let frame = match cache {
        Some(cache) => cache.frame(source, &geometry),
        None => Arc::new(Frame::new(source, &geometry)),
    };
    process(&frame, &effective(settings))
}

/// White balance gains for temperature and tint in −1…1, keeping
/// luminance. Positive temperature warms, positive tint goes magenta.
pub fn white_balance_gains(temperature: f32, tint: f32) -> [f32; 3] {
    let gains = [
        (TEMPERATURE_STOPS * temperature).exp2(),
        (-TINT_STOPS * tint).exp2(),
        (-TEMPERATURE_STOPS * temperature).exp2(),
    ];
    let y = luminance(gains);
    gains.map(|gain| gain / y)
}

pub const TEMPERATURE_STOPS: f32 = 0.6;
pub const TINT_STOPS: f32 = 0.4;

struct Area {
    shape: Shape,
    invert: bool,
    amounts: [f32; 9],
}

impl Area {
    /// Strength at a canvas position in fractions; `canvas` is its size in
    /// pixels.
    fn weight(&self, fx: f32, fy: f32, canvas: [f32; 2]) -> f32 {
        let weight = match self.shape {
            Shape::Linear { from, to } => {
                let (dx, dy) = ((to[0] - from[0]) * canvas[0], (to[1] - from[1]) * canvas[1]);
                let length = (dx * dx + dy * dy).max(1e-6);
                let t = ((fx - from[0]) * canvas[0] * dx + (fy - from[1]) * canvas[1] * dy) / length;
                1.0 - smoothstep(0.0, 1.0, t)
            }
            Shape::Radial { center, radius, feather } => {
                let x = (fx - center[0]) / radius[0].max(1e-4);
                let y = (fy - center[1]) / radius[1].max(1e-4);
                let d = (x * x + y * y).sqrt();
                1.0 - smoothstep(1.0 - feather.clamp(0.01, 1.0), 1.0, d)
            }
        };
        if self.invert {
            1.0 - weight
        } else {
            weight
        }
    }
}

const TONE_STEPS: usize = 4096;

/// Global tone in linear light, indexed by `sqrt(linear)`.
fn tone_table(contrast: f32, whites: f32, blacks: f32) -> Vec<f32> {
    (0..TONE_STEPS)
        .map(|index| {
            let t = index as f32 / (TONE_STEPS - 1) as f32;
            let mut p = color::linear_to_srgb(t * t);
            p += blacks * 0.12 * (1.0 - p).powi(4);
            p += whites * 0.12 * p.powi(4);
            p = p.clamp(0.0, 1.0);
            if contrast != 0.0 {
                let k = contrast * 0.7;
                let pivot = 0.46;
                p = if p < pivot {
                    pivot * (p / pivot).powf(1.0 + k)
                } else {
                    1.0 - (1.0 - pivot) * ((1.0 - p) / (1.0 - pivot)).powf(1.0 + k)
                };
            }
            color::srgb_to_linear(p.clamp(0.0, 1.0))
        })
        .collect()
}

fn tone(table: &[f32], linear: f32) -> f32 {
    curve::lookup(table, linear.clamp(0.0, 1.0).sqrt())
}

/// The two color bands around an OkLCh hue and how far toward the second.
fn bands(hue: f32) -> (usize, usize, f32) {
    let hue = hue.rem_euclid(360.0);
    let n = BAND_HUES.len();
    for k in 0..n {
        let low = BAND_HUES[k];
        let high = if k + 1 < n { BAND_HUES[k + 1] } else { BAND_HUES[0] + 360.0 };
        let h = if hue < low { hue + 360.0 } else { hue };
        if h >= low && h < high {
            return (k, (k + 1) % n, smoothstep(0.0, 1.0, (h - low) / (high - low)));
        }
    }
    (0, 1, 0.0)
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 32767.5 - 1.0
}

/// Smooth value noise in −1…1 with cells `cell` pixels wide.
fn value_noise(x: f32, y: f32, cell: f32, seed: u32) -> f32 {
    let (x, y) = (x / cell, y / cell);
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (smoothstep(0.0, 1.0, x - x0), smoothstep(0.0, 1.0, y - y0));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let top = hash(ix, iy, seed) + (hash(ix + 1, iy, seed) - hash(ix, iy, seed)) * tx;
    let bottom = hash(ix, iy + 1, seed) + (hash(ix + 1, iy + 1, seed) - hash(ix, iy + 1, seed)) * tx;
    top + (bottom - top) * ty
}

fn process(frame: &Frame, s: &Settings) -> RgbaImage {
    let (w, h) = (frame.w, frame.h);
    let n = |control: Control| s.get(control) / 100.0;

    let exposure = s.get(Control::EXPOSURE);
    let (contrast, highlights, shadows) = (n(Control::CONTRAST), n(Control::HIGHLIGHTS), n(Control::SHADOWS));
    let (whites, blacks) = (n(Control::WHITES), n(Control::BLACKS));
    let (temperature, tint) = (n(Control::TEMPERATURE), n(Control::TINT));
    let (texture, clarity, dehaze) = (n(Control::TEXTURE), n(Control::CLARITY), n(Control::DEHAZE));
    let (vibrance, saturation) = (n(Control::VIBRANCE), n(Control::SATURATION));
    let monochrome = s.get(Control::MONOCHROME) >= 0.5;

    let areas: Vec<Area> = s
        .masks
        .iter()
        .map(|mask| {
            let mut amounts = mask.amounts;
            for (local, amount) in amounts.iter_mut().enumerate() {
                if local != LOCAL_EXPOSURE {
                    *amount /= 100.0;
                }
            }
            Area {
                shape: mask.shape,
                invert: mask.invert,
                amounts,
            }
        })
        .collect();
    let local_any = |local: usize| areas.iter().any(|area| area.amounts[local] != 0.0);

    // Noise reduction replaces the frame's pixels for this render.
    let (luma_nr, chroma_nr) = (n(Control::NOISE_LUMINANCE), n(Control::NOISE_COLOR));
    let mut log: Cow<[f32]> = Cow::Borrowed(&frame.log);
    let mut rgb_in: Cow<[[f32; 3]]> = Cow::Borrowed(&frame.rgb);
    if chroma_nr > 0.0 {
        let radius = ((w.max(h) as f32 * 0.01 * chroma_nr).round() as usize).max(1);
        let planes: Vec<Vec<f32>> = (0..3)
            .map(|channel| box_blur(&frame.rgb.iter().map(|rgb| rgb[channel]).collect::<Vec<_>>(), w, h, radius))
            .collect();
        let y: Vec<f32> = frame.rgb.iter().map(|rgb| luminance(*rgb)).collect();
        let y_blur = box_blur(&y, w, h, radius);
        rgb_in = Cow::Owned(
            (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let scale = y[i] / y_blur[i].max(1e-6);
                    let smooth = [planes[0][i] * scale, planes[1][i] * scale, planes[2][i] * scale];
                    let original = frame.rgb[i];
                    std::array::from_fn(|c| original[c] + (smooth[c] - original[c]) * chroma_nr)
                })
                .collect(),
        );
    }
    if luma_nr > 0.0 {
        let radius = 1 + (luma_nr * 3.0).round() as usize;
        let eps = (0.05 + 0.4 * luma_nr).powi(2);
        let smooth = guided(&frame.log, w, h, radius, eps);
        log = Cow::Owned(frame.log.iter().zip(smooth).map(|(l, s)| l + (s - l) * luma_nr).collect());
    }

    let sharpen_amount = s.get(Control::SHARPEN_AMOUNT) / 100.0;
    let sharpen: Option<Vec<f32>> = (sharpen_amount > 0.0).then(|| {
        let radius = s.get(Control::SHARPEN_RADIUS);
        let box_radius = radius.round().max(1.0);
        let strength = (radius / box_radius).min(1.0) * sharpen_amount * 1.2;
        let blurred = box_blur(&log, w, h, box_radius as usize);
        let soften = (1.0 - n(Control::SHARPEN_DETAIL)) * 0.08 + 0.001;
        let masking = n(Control::SHARPEN_MASKING) * 0.6;
        (0..w * h)
            .into_par_iter()
            .map(|i| {
                let delta = log[i] - blurred[i];
                let mut delta = delta * delta.abs() / (delta.abs() + soften);
                if masking > 0.0 {
                    let (x, y) = (i % w, i / w);
                    let gx = log[y * w + (x + 1).min(w - 1)] - log[y * w + x.saturating_sub(1)];
                    let gy = log[(y + 1).min(h - 1) * w + x] - log[y.saturating_sub(1) * w + x];
                    delta *= smoothstep(masking * 0.5, masking, gx.abs() + gy.abs());
                }
                delta * strength
            })
            .collect()
    });

    let needs_base = highlights != 0.0
        || shadows != 0.0
        || clarity != 0.0
        || local_any(LOCAL_HIGHLIGHTS)
        || local_any(LOCAL_SHADOWS)
        || local_any(LOCAL_CLARITY);
    let base = needs_base.then(|| frame.base());
    let fine = (texture != 0.0).then(|| frame.fine());
    let haze = (dehaze != 0.0 || local_any(LOCAL_DEHAZE)).then(|| frame.haze());
    let tone_table = (contrast != 0.0 || whites != 0.0 || blacks != 0.0).then(|| tone_table(contrast, whites, blacks));

    let mix_parts: [[f32; 8]; 3] = std::array::from_fn(|part| std::array::from_fn(|band| n(Control::mix(part, band))));
    let mixing = mix_parts.iter().flatten().any(|v| *v != 0.0);
    let bw_mix: [f32; 8] = std::array::from_fn(|band| n(Control::bw(band)));
    let grades: [([f32; 2], f32, f32); 4] = std::array::from_fn(|zone| {
        let direction = hue_direction(s.get(Control::grade(zone, 0)));
        let strength = n(Control::grade(zone, 1)) * 0.06;
        (direction, strength, n(Control::grade(zone, 2)) * 0.15)
    });
    let grading = grades.iter().any(|(_, strength, lum)| *strength != 0.0 || *lum != 0.0);
    let blending = 0.05 + 0.25 * n(Control::GRADE_BLENDING);
    let center = 0.5 - 0.2 * n(Control::GRADE_BALANCE);
    let color_stage = mixing || grading || monochrome || vibrance != 0.0 || saturation != 0.0 || local_any(LOCAL_SATURATION);

    let vignette = n(Control::VIGNETTE_AMOUNT);
    let vignette_mid = 0.4 + 0.8 * n(Control::VIGNETTE_MIDPOINT);
    let vignette_half = 0.02 + 0.6 * n(Control::VIGNETTE_FEATHER);
    let roundness = n(Control::VIGNETTE_ROUNDNESS);
    let vignette_protect = n(Control::VIGNETTE_HIGHLIGHTS);
    let aspect = w as f32 / h as f32;

    let split = [
        n(Control::SPLIT_SHADOWS),
        n(Control::SPLIT_MIDTONES),
        n(Control::SPLIT_HIGHLIGHTS),
    ];
    let parametric = [
        n(Control::CURVE_SHADOWS),
        n(Control::CURVE_DARKS),
        n(Control::CURVE_LIGHTS),
        n(Control::CURVE_HIGHLIGHTS),
    ];
    let curves: Option<[[u8; 256]; 3]> = (parametric.iter().any(|v| *v != 0.0) || s.curves.iter().any(|c| !c.is_identity())).then(|| {
        let param = curve::sample(&curve::parametric_points(parametric, split), 256);
        let rgb = curve::sample(&s.curves[0].0, 1024);
        std::array::from_fn(|channel| {
            let table = curve::sample(&s.curves[channel + 1].0, 1024);
            std::array::from_fn(|v| {
                let x = curve::lookup(&rgb, param[v]);
                (curve::lookup(&table, x) * 255.0).round().clamp(0.0, 255.0) as u8
            })
        })
    });

    let grain = n(Control::GRAIN_AMOUNT) * 46.0;
    let grain_cell = (0.8 + n(Control::GRAIN_SIZE) * 4.0) * (w.max(h) as f32 / 2000.0).max(0.35);
    let roughness = n(Control::GRAIN_ROUGHNESS);

    let view = frame.view;
    let canvas = frame.canvas;
    let mut out = vec![0u8; w * h * 4];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let fy = (y as f32 + 0.5) / h as f32;
        let cy = view[1] + fy * (view[3] - view[1]);
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let i = y * w + x;
            let fx = (x as f32 + 0.5) / w as f32;
            let mut local = [0.0f32; 9];
            if !areas.is_empty() {
                let cx = view[0] + fx * (view[2] - view[0]);
                for area in &areas {
                    let weight = area.weight(cx, cy, canvas);
                    if weight > 0.0 {
                        for (slot, amount) in local.iter_mut().zip(area.amounts) {
                            *slot += amount * weight;
                        }
                    }
                }
            }

            let l0 = log[i];
            let mut rgb = rgb_in[i];
            if luma_nr > 0.0 {
                let scale = (l0 - frame.log[i]).exp2();
                rgb = rgb.map(|c| c * scale);
            }

            if let Some(haze) = haze {
                let amount = (dehaze + local[LOCAL_DEHAZE]).clamp(-1.0, 1.0);
                let light = haze.light;
                if amount > 0.0 {
                    let t = haze.transmission.sample(fx, fy);
                    let t = (1.0 - amount * (1.0 - t)).max(0.1);
                    rgb = std::array::from_fn(|c| ((rgb[c] - light[c]) / t + light[c]).max(0.0));
                } else if amount < 0.0 {
                    rgb = std::array::from_fn(|c| rgb[c] + (light[c] - rgb[c]) * -amount * 0.5);
                }
            }

            let gains = white_balance_gains(temperature + local[LOCAL_TEMPERATURE], tint + local[LOCAL_TINT]);
            let gain = (exposure + local[LOCAL_EXPOSURE]).exp2();
            rgb = std::array::from_fn(|c| rgb[c] * gains[c] * gain);

            let mut delta = 0.0;
            if let Some(base) = base {
                let current = (luminance(rgb) + LOG_FLOOR).log2();
                let detail = l0 - base.sample(fx, fy);
                let b = current - detail;
                let lift = (shadows + local[LOCAL_SHADOWS]) * 2.2 * (1.0 - smoothstep(-6.5, -1.5, b));
                let recover = (highlights + local[LOCAL_HIGHLIGHTS]) * 1.4 * smoothstep(-2.5, 0.0, b);
                let mid = (1.0 - ((b + 2.5) / 5.0).powi(2)).clamp(0.0, 1.0);
                delta += lift + recover + (clarity + local[LOCAL_CLARITY]) * 0.6 * detail * mid;
            }
            if let Some(fine) = fine {
                delta += texture * 1.2 * (l0 - fine[i]).clamp(-2.0, 2.0);
            }
            if let Some(sharpen) = &sharpen {
                delta += sharpen[i];
            }
            if delta != 0.0 {
                let scale = delta.clamp(-6.0, 6.0).exp2();
                rgb = rgb.map(|c| c * scale);
            }

            if let Some(table) = &tone_table {
                rgb = rgb.map(|c| tone(table, c));
            } else {
                rgb = rgb.map(|c| c.clamp(0.0, 1.0));
            }

            let local_contrast = local[LOCAL_CONTRAST];
            if local_contrast != 0.0 {
                rgb = rgb.map(|c| {
                    let p = color::linear_to_srgb(c);
                    let p = 0.46 + (p - 0.46) * (1.0 + local_contrast * 0.6);
                    color::srgb_to_linear(p.clamp(0.0, 1.0))
                });
            }

            if color_stage {
                let [mut l, a, b] = to_oklab(rgb);
                let mut chroma = (a * a + b * b).sqrt();
                let mut hue = b.atan2(a).to_degrees();
                let colorful = smoothstep(0.0, 0.08, chroma);
                let (low, high, t) = bands(hue);
                let band = |values: &[f32; 8]| values[low] * (1.0 - t) + values[high] * t;
                if mixing {
                    hue += band(&mix_parts[0]) * 30.0;
                    chroma *= (1.0 + band(&mix_parts[1])).max(0.0);
                    l += band(&mix_parts[2]) * 0.2 * colorful;
                }
                if vibrance != 0.0 {
                    let skin = (-((hue.rem_euclid(360.0) - 60.0) / 22.0).powi(2)).exp();
                    let room = if vibrance > 0.0 { (1.0 - chroma / 0.25).clamp(0.0, 1.0) * (1.0 - skin * 0.6) } else { 1.0 };
                    chroma *= (1.0 + vibrance * room).max(0.0);
                }
                chroma *= (1.0 + saturation + local[LOCAL_SATURATION]).max(0.0);
                if monochrome {
                    l += band(&bw_mix) * 0.3 * colorful;
                    chroma = 0.0;
                }
                let (sin, cos) = hue.to_radians().sin_cos();
                let (mut a, mut b) = (chroma * cos, chroma * sin);
                if grading {
                    let shadows_w = 1.0 - smoothstep(center - 0.25 - blending, center - 0.25 + blending, l);
                    let highlights_w = smoothstep(center + 0.25 - blending, center + 0.25 + blending, l);
                    let weights = [shadows_w, (1.0 - shadows_w - highlights_w).max(0.0), highlights_w, 1.0];
                    for ((direction, strength, lum), weight) in grades.iter().zip(weights) {
                        a += direction[0] * strength * weight;
                        b += direction[1] * strength * weight;
                        l += lum * weight;
                    }
                }
                rgb = from_oklab([l.max(0.0), a, b]).map(|c| c.clamp(0.0, 1.0));
            }

            if vignette != 0.0 {
                let mut dx = (fx - 0.5) * 2.0;
                let mut dy = (fy - 0.5) * 2.0;
                if roundness > 0.0 {
                    if aspect > 1.0 {
                        dx *= aspect.powf(roundness);
                    } else {
                        dy *= (1.0 / aspect).powf(roundness);
                    }
                }
                let power = 2.0 + (-roundness).max(0.0) * 6.0;
                let d = (dx.abs().powf(power) + dy.abs().powf(power)).powf(1.0 / power);
                let weight = smoothstep(vignette_mid - vignette_half, vignette_mid + vignette_half, d);
                if vignette < 0.0 {
                    let protect = 1.0 - vignette_protect * smoothstep(0.4, 1.0, luminance(rgb));
                    let scale = 1.0 + vignette * weight * protect;
                    rgb = rgb.map(|c| c * scale);
                } else {
                    rgb = rgb.map(|c| c + (1.0 - c) * vignette * weight * 0.8);
                }
            }

            let mut out = rgb.map(encode);
            if let Some(curves) = &curves {
                out = std::array::from_fn(|c| curves[c][out[c] as usize]);
            }
            if grain > 0.0 {
                let (px, py) = (x as f32, y as f32);
                let coarse = value_noise(px, py, grain_cell, 17);
                let finer = value_noise(px, py, grain_cell * 0.5, 91);
                let noise = coarse * (1.0 - roughness * 0.5) + finer * roughness * 0.7;
                let p = luminance(out.map(|c| c as f32 / 255.0));
                let amount = grain * noise * (0.25 + 3.0 * p * (1.0 - p));
                out = out.map(|c| (c as f32 + amount).round().clamp(0.0, 255.0) as u8);
            }
            pixel.copy_from_slice(&[out[0], out[1], out[2], 255]);
        }
    });
    RgbaImage::from_raw(w as u32, h as u32, out).expect("buffer matches the frame size")
}

/// Paints the clipping warning and the selected mask over a render.
/// `canvas` is the photo's size and `view` the part of it the render shows.
pub fn decorate(image: &mut RgbaImage, settings: &Settings, options: &Options, canvas: [f32; 2], view: [f32; 4]) {
    let area = options.overlay.and_then(|index| settings.masks.get(index)).map(|mask| Area {
        shape: mask.shape,
        invert: mask.invert,
        amounts: mask.amounts,
    });
    if !options.clipping && area.is_none() {
        return;
    }
    let (w, h) = (image.width() as usize, image.height() as usize);
    image.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let cy = view[1] + (y as f32 + 0.5) / h as f32 * (view[3] - view[1]);
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            if options.clipping {
                let max = pixel[0].max(pixel[1]).max(pixel[2]);
                if max == 255 {
                    pixel[..3].copy_from_slice(&[255, 40, 40]);
                } else if max <= 1 {
                    pixel[..3].copy_from_slice(&[40, 90, 255]);
                }
            }
            if let Some(area) = &area {
                let cx = view[0] + (x as f32 + 0.5) / w as f32 * (view[2] - view[0]);
                let shown = area.weight(cx, cy, canvas);
                let tint = [255.0, 64.0, 64.0];
                for c in 0..3 {
                    pixel[c] = (pixel[c] as f32 + (tint[c] - pixel[c] as f32) * shown * 0.5).round() as u8;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient() -> RgbaImage {
        RgbaImage::from_fn(64, 48, |x, y| image::Rgba([(x * 4) as u8, (y * 5) as u8, 128, 255]))
    }

    #[test]
    fn neutral_settings_keep_the_photo() {
        let source = gradient();
        let mut settings = Settings::default();
        settings.set(Control::SATURATION, 0.0);
        let out = render(&source, &settings, &Options::default(), None);
        for (a, b) in out.as_raw().iter().zip(source.as_raw()) {
            assert!(a.abs_diff(*b) <= 1);
        }
    }

    #[test]
    fn every_control_renders_at_its_extremes() {
        let source = gradient();
        let mut cache = Cache::default();
        for control in Control::all() {
            for value in [control.range().0, control.range().1] {
                let mut settings = Settings::default();
                settings.set(control, value);
                let out = render(&source, &settings, &Options::default(), Some(&mut cache));
                assert!(out.width() > 0 && out.height() > 0, "{}", control.key());
            }
        }
    }

    #[test]
    fn exposure_brightens_and_monochrome_is_gray() {
        let source = gradient();
        let mut settings = Settings::default();
        settings.set(Control::EXPOSURE, 1.0);
        let bright = render(&source, &settings, &Options::default(), None);
        let sum = |image: &RgbaImage| image.as_raw().iter().map(|v| *v as u64).sum::<u64>();
        assert!(sum(&bright) > sum(&source));
        let mut settings = Settings::default();
        settings.set(Control::MONOCHROME, 1.0);
        let gray = render(&source, &settings, &Options::default(), None);
        let pixel = gray.get_pixel(40, 20);
        assert!(pixel[0].abs_diff(pixel[1]) <= 2 && pixel[1].abs_diff(pixel[2]) <= 2);
        let mut settings = Settings::default();
        settings.masks.push(Mask {
            name: String::new(),
            shape: Shape::Radial {
                center: [0.5, 0.5],
                radius: [0.3, 0.3],
                feather: 0.5,
            },
            invert: false,
            amounts: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        });
        let masked = render(&source, &settings, &Options::default(), None);
        assert!(masked.get_pixel(32, 24)[1] > source.get_pixel(32, 24)[1]);
        assert_eq!(masked.get_pixel(0, 0), source.get_pixel(0, 0));
    }
}
