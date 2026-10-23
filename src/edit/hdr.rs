//! HDR tone mapping of a single photo. Every method works on the
//! edge-aware base layer (the large-scale brightness, in log2 stops) and
//! puts the fine detail back on top, scaled by Detail, so strong
//! compression leaves no halos around edges and texture stays crisp.
//!
//! - Local tone mapping compresses the base around mid-gray in the log
//!   domain, in the manner of Durand and Dorsey's bilateral method.
//! - Exposure fusion blends three virtual exposures two stops apart,
//!   weighting each by how well exposed it is, as Mertens et al. do.
//! - Reinhard, Drago, and Filmic are global curves (Reinhard's photographic
//!   operator, Drago's adaptive logarithm, and the ACES filmic fit) applied
//!   to the photo seen two stops brighter, with its white kept at white.

use super::color::{linear_to_srgb, srgb_to_linear};

/// log2 of 18% gray.
const MID_GRAY: f32 = -2.474;
/// Stops of extra light the global curves see, so they have range to
/// compress in an ordinary photo.
const SCENE_STOPS: f32 = 2.0;
const FLOOR: f32 = 1.0 / 4096.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Local,
    Fusion,
    Reinhard,
    Drago,
    Filmic,
}

impl Method {
    pub fn from_index(index: usize) -> Method {
        match index {
            1 => Method::Fusion,
            2 => Method::Reinhard,
            3 => Method::Drago,
            4 => Method::Filmic,
            _ => Method::Local,
        }
    }
}

/// Brightness of a photo in log2 stops: the mean and the 1st and 99.5th
/// percentiles.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub mean: f32,
    pub low: f32,
    pub high: f32,
}

impl Stats {
    pub fn of(log: &[f32]) -> Stats {
        if log.is_empty() {
            return Stats { mean: MID_GRAY, low: -8.0, high: 0.0 };
        }
        let mean = log.iter().sum::<f32>() / log.len() as f32;
        let mut sorted = log.to_vec();
        sorted.sort_unstable_by(f32::total_cmp);
        let at = |fraction: f32| sorted[((sorted.len() - 1) as f32 * fraction) as usize];
        Stats {
            mean,
            low: at(0.01),
            high: at(0.995),
        }
    }
}

pub struct Hdr {
    method: Method,
    /// 0…1, how far toward the tone-mapped result.
    pub strength: f32,
    /// Gain on the fine detail, 1 keeps it as it is.
    detail: f32,
    /// Added to the base so the photo's average moves halfway to mid-gray.
    shift: f32,
    low: f32,
    high: f32,
    /// What the global curve makes of the scene's white, and of mid-gray,
    /// so white stays white and mid-gray moves only halfway.
    white: f32,
    offset: f32,
}

impl Hdr {
    /// `detail` is the slider value 0…100 and `exposure` the stops the
    /// photo was brightened by, which the photo's white point follows.
    pub fn new(method: Method, strength: f32, detail: f32, stats: Stats, exposure: f32) -> Hdr {
        let shift = 0.5 * (MID_GRAY - stats.mean);
        let high = (stats.high + exposure + shift).max(MID_GRAY + 1.0);
        let low = (stats.low + exposure + shift).min(MID_GRAY - 1.0);
        let mut hdr = Hdr {
            method,
            strength: strength.clamp(0.0, 1.0),
            detail: detail / 50.0,
            shift,
            low,
            high,
            white: 1.0,
            offset: 0.0,
        };
        if matches!(method, Method::Reinhard | Method::Drago | Method::Filmic) {
            let scene_white = (high + SCENE_STOPS).exp2();
            hdr.white = hdr.curve(scene_white, scene_white).max(1e-6);
            let mid = hdr.curve((MID_GRAY + SCENE_STOPS).exp2(), scene_white) / hdr.white;
            hdr.offset = -0.5 * ((mid + FLOOR).log2() - MID_GRAY);
        }
        hdr
    }

    /// Display brightness in log2 stops for a pixel with the base `base`
    /// and the detail `detail` above it, both in log2 stops.
    pub fn map(&self, base: f32, detail: f32) -> f32 {
        let b = base + self.shift;
        let detail = detail * self.detail;
        let out = match self.method {
            Method::Local => self.compress(b) + detail,
            Method::Fusion => fuse(b, detail),
            Method::Reinhard | Method::Drago | Method::Filmic => {
                let scene_white = (self.high + SCENE_STOPS).exp2();
                let x = (b + SCENE_STOPS).exp2();
                (self.curve(x, scene_white) / self.white + FLOOR).log2() + self.offset + detail
            }
        };
        out.min(0.5)
    }

    /// Log-domain compression around mid-gray: the brightest base level
    /// lands just under white and the darkest six stops under mid-gray at
    /// most, each side scaled on its own.
    fn compress(&self, b: f32) -> f32 {
        if b >= MID_GRAY {
            let scale = ((-0.4 - MID_GRAY) / (self.high - MID_GRAY)).clamp(0.2, 1.0);
            MID_GRAY + (b - MID_GRAY) * scale
        } else {
            let scale = ((MID_GRAY + 6.0) / (MID_GRAY - self.low)).clamp(0.2, 1.0);
            MID_GRAY - (MID_GRAY - b) * scale
        }
    }

    /// The global curve at linear `x`, for a scene whose white is `white`.
    fn curve(&self, x: f32, white: f32) -> f32 {
        match self.method {
            Method::Reinhard => x * (1.0 + x / (white * white)) / (1.0 + x),
            Method::Drago => {
                const BIAS: f32 = 0.85;
                let power = BIAS.ln() / 0.5f32.ln();
                let log_part = (x + 1.0).ln() / (white + 1.0).ln();
                log_part * 10f32.ln() / (2.0 + 8.0 * (x / white).powf(power)).ln()
            }
            Method::Filmic => {
                let x = x * 0.6;
                x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14)
            }
            Method::Local | Method::Fusion => x,
        }
    }
}

/// Exposure fusion of one pixel: three exposures two stops apart, each
/// weighted by how close its base is to middle gray, blended in display
/// values. Weighting by the smooth base rather than the pixel keeps the
/// blend from flattening edges.
fn fuse(base: f32, detail: f32) -> f32 {
    const SIGMA: f32 = 0.25;
    let (mut sum, mut weights) = (0.0, 0.0);
    for stops in [-2.0, -1.0, 0.0, 1.0, 2.0] {
        let shown = linear_to_srgb((base + stops).exp2().min(1.0));
        let weight = (-(shown - 0.5).powi(2) / (2.0 * SIGMA * SIGMA)).exp() + 1e-4;
        let pixel = linear_to_srgb((base + detail + stops).exp2().min(1.0));
        sum += pixel * weight;
        weights += weight;
    }
    (srgb_to_linear(sum / weights) + FLOOR).log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDINARY: Stats = Stats { mean: MID_GRAY, low: -9.0, high: 0.0 };

    #[test]
    fn methods_keep_order_and_white() {
        for index in 0..5 {
            let method = Method::from_index(index);
            let hdr = Hdr::new(method, 1.0, 50.0, ORDINARY, 0.0);
            let dark = hdr.map(-8.0, 0.0);
            // Filmic deepens the darkest shadows on purpose, as film does.
            assert!(method == Method::Filmic || dark > -8.0, "{index}: shadows lifted, got {dark}");
            let mut last = f32::MIN;
            for step in 0..=40 {
                let b = -10.0 + step as f32 * 0.25;
                let out = hdr.map(b, 0.0);
                assert!(out.is_finite() && out >= last - 1e-4, "{index}: monotonic at {b}");
                last = out;
            }
            assert!(hdr.map(0.0, 0.0) <= 0.01, "{index}: white stays at most white");
        }
    }

    #[test]
    fn detail_follows_its_slider() {
        let flat = Hdr::new(Method::Local, 1.0, 0.0, ORDINARY, 0.0);
        let strong = Hdr::new(Method::Local, 1.0, 100.0, ORDINARY, 0.0);
        let (base, detail) = (-3.0, 0.5);
        assert!((flat.map(base, detail) - flat.map(base, 0.0)).abs() < 1e-5);
        assert!((strong.map(base, detail) - strong.map(base, 0.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn exposure_past_white_is_brought_back() {
        let hdr = Hdr::new(Method::Reinhard, 1.0, 50.0, ORDINARY, 2.0);
        assert!(hdr.map(2.0, 0.0) <= 0.01);
        let local = Hdr::new(Method::Local, 1.0, 50.0, ORDINARY, 2.0);
        assert!(local.map(2.0, 0.0) < 0.0);
    }
}
