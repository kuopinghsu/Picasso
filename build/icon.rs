//! The Picasso mark: a navy tile crossed by a yellow arc, an orange sweep,
//! and a blue stroke, each painted as a brush stroke with ragged ends.
//! Shapes are analytic so every icon size is drawn sharp from scratch.

use std::f32::consts::TAU;

type Rgb = [f32; 3];

const TILE_MARGIN: f32 = 0.035;
const TILE_RADIUS: f32 = 0.15;
const TILE_TOP: Rgb = [0.12, 0.29, 0.54];
const TILE_BOTTOM: Rgb = [0.06, 0.15, 0.33];
const TILE_RIM: Rgb = [0.36, 0.55, 0.86];
const SHADOW_OFFSET: (f32, f32) = (0.010, 0.014);
const SHADOW_ALPHA: f32 = 0.32;
const LANES: f32 = 13.0;

enum Path {
    Arc { center: (f32, f32), radius: f32, from: f32, sweep: f32 },
    Line { from: (f32, f32), to: (f32, f32) },
}

struct Brush {
    path: Path,
    width: f32,
    /// Width factor at the start and the end of the stroke.
    taper: (f32, f32),
    /// Share of the stroke over which the bristles fray at each end.
    rag: (f32, f32),
    top: Rgb,
    bottom: Rgb,
    seed: u32,
}

impl Brush {
    /// Position along the stroke (0 to 1) and signed distance across it.
    fn locate(&self, x: f32, y: f32) -> (f32, f32) {
        match self.path {
            Path::Arc { center, radius, from, sweep } => {
                let (dx, dy) = (x - center.0, y - center.1);
                let angle = dy.atan2(dx);
                let along = if sweep >= 0.0 { angle - from } else { from - angle };
                (along.rem_euclid(TAU) / sweep.abs(), (dx * dx + dy * dy).sqrt() - radius)
            }
            Path::Line { from, to } => {
                let (dx, dy) = (to.0 - from.0, to.1 - from.1);
                let length = (dx * dx + dy * dy).sqrt();
                let (px, py) = (x - from.0, y - from.1);
                ((px * dx + py * dy) / (length * length), (dx * py - dy * px) / length)
            }
        }
    }

    /// Bristle shade when the stroke covers the point.
    fn cover(&self, x: f32, y: f32) -> Option<f32> {
        let (t, s) = self.locate(x, y);
        if !(0.0..=1.0).contains(&t) {
            return None;
        }
        let width = self.width
            * mix(self.taper.0, 1.0, smoothstep(0.0, 0.4, t))
            * mix(self.taper.1, 1.0, smoothstep(0.0, 0.4, 1.0 - t));
        let across = s / width;
        if across.abs() > 0.5 {
            return None;
        }
        let lane = ((across + 0.5) * LANES).floor() as i32;
        let edge = across.abs() * 2.0;
        let start = self.rag.0 * (0.15 + 0.85 * hash(lane, self.seed).max(edge * 0.8));
        let end = self.rag.1 * (0.15 + 0.85 * hash(lane, self.seed + 1).max(edge * 0.8));
        if t < start || 1.0 - t < end {
            return None;
        }
        Some(0.9 + 0.1 * hash(lane, self.seed + 2))
    }

    fn color(&self, y: f32, shade: f32) -> Rgb {
        let base = mix3(self.top, self.bottom, y);
        base.map(|channel| channel * shade)
    }
}

fn brushes() -> [Brush; 3] {
    let yellow = Brush {
        path: Path::Arc {
            center: (0.54, 0.45),
            radius: 0.27,
            from: 197f32.to_radians(),
            sweep: 218f32.to_radians(),
        },
        width: 0.095,
        taper: (0.7, 0.8),
        rag: (0.2, 0.12),
        top: [1.0, 0.80, 0.28],
        bottom: [0.96, 0.67, 0.16],
        seed: 11,
    };
    let orange = Brush {
        path: arc_through((0.11, 0.535), (0.45, 0.65), (0.89, 0.34)),
        width: 0.115,
        taper: (0.6, 0.35),
        rag: (0.14, 0.22),
        top: [0.97, 0.47, 0.21],
        bottom: [0.90, 0.36, 0.15],
        seed: 23,
    };
    let blue = Brush {
        path: Path::Line {
            from: (0.48, 0.12),
            to: (0.27, 0.93),
        },
        width: 0.105,
        taper: (0.85, 0.55),
        rag: (0.06, 0.22),
        top: [0.22, 0.46, 0.82],
        bottom: [0.16, 0.36, 0.70],
        seed: 37,
    };
    [yellow, orange, blue]
}

/// Straight-alpha RGBA pixels of the icon at `size` × `size`.
pub fn render(size: u32) -> Vec<u8> {
    let brushes = brushes();
    let samples = if size <= 64 { 8 } else { 4 };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut sum = [0.0f32; 3];
            let mut alpha = 0.0;
            for sy in 0..samples {
                for sx in 0..samples {
                    let x = (px as f32 + (sx as f32 + 0.5) / samples as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / samples as f32) / size as f32;
                    if let Some(color) = sample(&brushes, x, y) {
                        for (total, channel) in sum.iter_mut().zip(color) {
                            *total += channel;
                        }
                        alpha += 1.0;
                    }
                }
            }
            let count = (samples * samples) as f32;
            let rgb = if alpha > 0.0 { sum.map(|total| total / alpha) } else { [0.0; 3] };
            out.extend(rgb.map(byte));
            out.push(byte(alpha / count));
        }
    }
    out
}

/// Opaque color at a point inside the tile, or `None` outside it.
fn sample(brushes: &[Brush], x: f32, y: f32) -> Option<Rgb> {
    let edge = tile_distance(x, y);
    if edge > 0.0 {
        return None;
    }
    let glow = 0.07 * (-((x - 0.5).powi(2) + (y - 0.12).powi(2)) / 0.16).exp();
    let mut color = mix3(TILE_TOP, TILE_BOTTOM, y).map(|channel| channel + glow);
    if edge > -0.014 {
        color = mix3(color, TILE_RIM, 0.35 * (1.0 - 0.6 * y));
    }
    for brush in brushes {
        if brush.cover(x - SHADOW_OFFSET.0, y - SHADOW_OFFSET.1).is_some() {
            color = color.map(|channel| channel * (1.0 - SHADOW_ALPHA));
        }
        if let Some(shade) = brush.cover(x, y) {
            color = brush.color(y, shade);
        }
    }
    Some(color)
}

/// Signed distance to the rounded tile; negative inside.
fn tile_distance(x: f32, y: f32) -> f32 {
    let half = 0.5 - TILE_MARGIN - TILE_RADIUS;
    let qx = (x - 0.5).abs() - half;
    let qy = (y - 0.5).abs() - half;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - TILE_RADIUS
}

/// Circular arc from `a` to `c` that passes through `b`.
fn arc_through(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> Path {
    let d = 2.0 * (a.0 * (b.1 - c.1) + b.0 * (c.1 - a.1) + c.0 * (a.1 - b.1));
    let norm = |p: (f32, f32)| p.0 * p.0 + p.1 * p.1;
    let cx = (norm(a) * (b.1 - c.1) + norm(b) * (c.1 - a.1) + norm(c) * (a.1 - b.1)) / d;
    let cy = (norm(a) * (c.0 - b.0) + norm(b) * (a.0 - c.0) + norm(c) * (b.0 - a.0)) / d;
    let angle = |p: (f32, f32)| (p.1 - cy).atan2(p.0 - cx);
    let from = angle(a);
    let to_b = (angle(b) - from).rem_euclid(TAU);
    let to_c = (angle(c) - from).rem_euclid(TAU);
    let sweep = if to_b < to_c { to_c } else { to_c - TAU };
    Path::Arc {
        center: (cx, cy),
        radius: ((a.0 - cx).powi(2) + (a.1 - cy).powi(2)).sqrt(),
        from,
        sweep,
    }
}

fn hash(lane: i32, seed: u32) -> f32 {
    let mut x = (lane as u32).wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA77);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

fn smoothstep(from: f32, to: f32, value: f32) -> f32 {
    let t = ((value - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix3(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
