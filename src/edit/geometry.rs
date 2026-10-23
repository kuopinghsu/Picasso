//! Straighten, perspective, corners, flips, and crop. The canvas is the
//! photo flipped, tilted in perspective, turned by the straighten angle
//! about its center, and then distorted so its corners land where they were
//! dragged. It has the photo's size, or its width and height swapped when
//! the angle is nearer a quarter turn than upright or upside down; the crop
//! is a rectangle on that canvas in fractions.

use image::RgbaImage;
use rayon::prelude::*;

use super::settings::{Control, Settings};

pub const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// Corners of the canvas in fractions, clockwise from the top left.
pub const SQUARE: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// Where a canvas position has no photo behind it.
const NOWHERE: (f32, f32) = (-1e9, -1e9);

/// A plane-to-plane perspective map, as a 3×3 matrix by rows.
#[derive(Clone, Copy, Debug)]
struct Projective([f32; 9]);

impl Projective {
    /// The map taking the corners of the unit square, clockwise from the top
    /// left, to `quad`. `None` when the quad has collapsed.
    fn square_to(quad: [[f32; 2]; 4]) -> Option<Projective> {
        let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = quad;
        let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
        let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-9 {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        Some(Projective([
            x1 - x0 + g * x1,
            x3 - x0 + h * x3,
            x0,
            y1 - y0 + g * y1,
            y3 - y0 + h * y3,
            y0,
            g,
            h,
            1.0,
        ]))
    }

    /// The inverse map, up to scale.
    fn inverse(&self) -> Projective {
        let [a, b, c, d, e, f, g, h, i] = self.0;
        Projective([
            e * i - f * h,
            c * h - b * i,
            b * f - c * e,
            f * g - d * i,
            a * i - c * g,
            c * d - a * f,
            d * h - e * g,
            b * g - a * h,
            a * e - b * d,
        ])
    }

    fn depth(&self, x: f32, y: f32) -> f32 {
        self.0[6] * x + self.0[7] * y + self.0[8]
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let m = &self.0;
        let depth = self.depth(x, y);
        ((m[0] * x + m[1] * y + m[2]) / depth, (m[3] * x + m[4] * y + m[5]) / depth)
    }
}

/// The photo as a plane seen from one focal length away, about a normal
/// lens, and tilted about its center.
#[derive(Clone, Copy, Debug)]
struct Tilt {
    focal: f32,
    vertical: (f32, f32),
    horizontal: (f32, f32),
    center: (f32, f32),
}

impl Tilt {
    fn new(vertical: f32, horizontal: f32, w: f32, h: f32) -> Option<Tilt> {
        if vertical == 0.0 && horizontal == 0.0 {
            return None;
        }
        let mut tilt = Tilt {
            focal: w.max(h),
            vertical: vertical.to_radians().sin_cos(),
            horizontal: horizontal.to_radians().sin_cos(),
            center: (0.0, 0.0),
        };
        let (x, y, z) = tilt.ray(0.0, 0.0);
        tilt.center = (x / z, y / z);
        Some(tilt)
    }

    fn ray(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let ((sin_v, cos_v), (sin_h, cos_h)) = (self.vertical, self.horizontal);
        let (y1, z1) = (y * cos_v + self.focal * sin_v, -y * sin_v + self.focal * cos_v);
        (x * cos_h - z1 * sin_h, y1, x * sin_h + z1 * cos_h)
    }

    /// Photo offset from its center under an offset from the center of the
    /// corrected photo, both in the photo's own axes and pixels.
    fn untilt(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (rx, ry, rz) = self.ray(x, y);
        if rz <= self.focal * 1e-3 {
            return None;
        }
        Some((self.focal * (rx / rz - self.center.0), self.focal * (ry / rz - self.center.1)))
    }
}

/// Whether `quad`, in canvas fractions clockwise from the top left, still
/// outlines a photo: convex, turning the same way as the canvas, and not
/// squeezed to a sliver.
pub fn is_convex(quad: [[f32; 2]; 4]) -> bool {
    let area = (0..4)
        .map(|i| {
            let [a, b] = [quad[i], quad[(i + 1) % 4]];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        * 0.5;
    area > 0.02
        && (0..4).all(|i| {
            let [a, b, c] = [quad[i], quad[(i + 1) % 4], quad[(i + 2) % 4]];
            (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]) > 1e-4
        })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// Clockwise, in degrees.
    pub angle: f32,
    /// Tilt of the photo's plane about its horizontal axis, in degrees.
    /// Positive brings the top nearer, so it widens.
    pub vertical: f32,
    /// Tilt about the vertical axis, in degrees. Positive widens the right.
    pub horizontal: f32,
    /// How far each canvas corner is moved, clockwise from the top left,
    /// in canvas fractions. Only what was on the canvas comes along, so the
    /// photo ends at the outline of the moved corners.
    pub corners: [[f32; 2]; 4],
    pub flip_h: bool,
    pub flip_v: bool,
    pub crop: [f32; 4],
}

impl Geometry {
    pub fn from_settings(settings: &Settings) -> Geometry {
        Geometry {
            angle: settings.get(Control::STRAIGHTEN),
            vertical: settings.get(Control::PERSPECTIVE_V),
            horizontal: settings.get(Control::PERSPECTIVE_H),
            corners: std::array::from_fn(|corner| [settings.get(Control::corner(corner, 0)), settings.get(Control::corner(corner, 1))]),
            flip_h: settings.get(Control::FLIP_H) >= 0.5,
            flip_v: settings.get(Control::FLIP_V) >= 0.5,
            crop: settings.crop(),
        }
    }

    /// Only the turn, tilt, and corners of `settings`, which decide where
    /// the photo covers the canvas.
    pub fn turn_of(settings: &Settings) -> Geometry {
        Geometry {
            flip_h: false,
            flip_v: false,
            crop: FULL,
            ..Geometry::from_settings(settings)
        }
    }

    pub fn is_identity(&self) -> bool {
        self.angle == 0.0
            && self.vertical == 0.0
            && self.horizontal == 0.0
            && self.corners == [[0.0; 2]; 4]
            && !self.flip_h
            && !self.flip_v
            && self.crop == FULL
    }

    /// Where the canvas corners land, clockwise from the top left, in
    /// canvas fractions.
    pub fn quad(&self) -> [[f32; 2]; 4] {
        std::array::from_fn(|i| [SQUARE[i][0] + self.corners[i][0], SQUARE[i][1] + self.corners[i][1]])
    }

    /// Size of the result for a `w`×`h` photo.
    pub fn output_size(&self, w: u32, h: u32) -> (u32, u32) {
        let [left, top, right, bottom] = self.crop;
        let [cw, ch] = canvas_size(self.angle, w as f32, h as f32);
        (
            (((right - left) * cw).round() as u32).max(1),
            (((bottom - top) * ch).round() as u32).max(1),
        )
    }

    /// Photo position, in pixels, under a position on a `cw`×`ch` canvas.
    pub fn canvas_to_source(&self, x: f32, y: f32, cw: f32, ch: f32) -> (f32, f32) {
        self.mapper(cw, ch)(x, y)
    }

    /// [`Geometry::canvas_to_source`] with the work that is the same for
    /// every position done once.
    fn mapper(&self, cw: f32, ch: f32) -> impl Fn(f32, f32) -> (f32, f32) + Sync {
        let [w, h] = canvas_size(self.angle, cw, ch);
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let tilt = Tilt::new(self.vertical, self.horizontal, w, h);
        let quad = self.quad();
        let distort = (self.corners != [[0.0; 2]; 4] && is_convex(quad))
            .then(|| Projective::square_to(quad))
            .flatten()
            .map(|forward| {
                let inverse = forward.inverse();
                let (cx, cy) = forward.apply(0.5, 0.5);
                (inverse, inverse.depth(cx, cy).signum())
            });
        let (flip_h, flip_v) = (self.flip_h, self.flip_v);
        move |x, y| {
            let (x, y) = match distort {
                Some((inverse, side)) => {
                    let (fx, fy) = (x / cw, y / ch);
                    if inverse.depth(fx, fy) * side <= 0.0 {
                        return NOWHERE;
                    }
                    let (u, v) = inverse.apply(fx, fy);
                    if !(-1e-4..=1.0 + 1e-4).contains(&u) || !(-1e-4..=1.0 + 1e-4).contains(&v) {
                        return NOWHERE;
                    }
                    (u * cw, v * ch)
                }
                None => (x, y),
            };
            let (dx, dy) = (x - cw * 0.5, y - ch * 0.5);
            let (turned_x, turned_y) = (dx * cos + dy * sin, -dx * sin + dy * cos);
            let Some((mut rx, mut ry)) = (match tilt {
                Some(tilt) => tilt.untilt(turned_x, turned_y),
                None => Some((turned_x, turned_y)),
            }) else {
                return NOWHERE;
            };
            if flip_h {
                rx = -rx;
            }
            if flip_v {
                ry = -ry;
            }
            (rx + w * 0.5, ry + h * 0.5)
        }
    }
}

/// Width and height of the canvas for a `w`×`h` photo turned by `angle`.
/// Given the canvas size instead, it gives back the photo's.
pub fn canvas_size(angle: f32, w: f32, h: f32) -> [f32; 2] {
    let half_turn = angle.rem_euclid(180.0);
    if half_turn > 45.0 && half_turn < 135.0 {
        [h, w]
    } else {
        [w, h]
    }
}

/// Whether every corner of `crop` on a `cw`×`ch` canvas covers the photo
/// after the turn, tilt, and corners of `turn`. The photo covers a convex area, so
/// the whole crop does then too.
pub fn inside(crop: [f32; 4], turn: &Geometry, cw: f32, ch: f32) -> bool {
    let [w, h] = canvas_size(turn.angle, cw, ch);
    let [left, top, right, bottom] = crop;
    let to_source = turn.mapper(cw, ch);
    [(left, top), (right, top), (left, bottom), (right, bottom)].into_iter().all(|(fx, fy)| {
        let (x, y) = to_source(fx * cw, fy * ch);
        x >= -0.5 && y >= -0.5 && x <= w + 0.5 && y <= h + 0.5
    })
}

/// `crop` shrunk about its center, keeping its shape, until no corner shows
/// the empty area around a turned, tilted, or distorted photo. `w`×`h` is
/// the canvas.
pub fn constrain(crop: [f32; 4], turn: &Geometry, w: f32, h: f32) -> [f32; 4] {
    if inside(crop, turn, w, h) {
        return crop;
    }
    let [left, top, right, bottom] = crop;
    let scaled = |center: [f32; 2], scale: f32| {
        let (half_w, half_h) = ((right - left) * 0.5 * scale, (bottom - top) * 0.5 * scale);
        [center[0] - half_w, center[1] - half_h, center[0] + half_w, center[1] + half_h]
    };
    let mut center = [(left + right) * 0.5, (top + bottom) * 0.5];
    if !inside(scaled(center, 0.01), turn, w, h) {
        center = [0.5, 0.5];
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let middle = (low + high) * 0.5;
        if inside(scaled(center, middle), turn, w, h) {
            low = middle;
        } else {
            high = middle;
        }
    }
    scaled(center, low)
}

/// The largest crop of `ratio` (width over height, in pixels) centered in
/// `crop`, for a canvas of `w`×`h` pixels.
pub fn with_aspect(crop: [f32; 4], ratio: f32, w: f32, h: f32) -> [f32; 4] {
    let [left, top, right, bottom] = crop;
    let (width, height) = ((right - left) * w, (bottom - top) * h);
    let (new_w, new_h) = if width / height > ratio { (height * ratio, height) } else { (width, width / ratio) };
    let (cx, cy) = ((left + right) * 0.5, (top + bottom) * 0.5);
    let (half_w, half_h) = (new_w / w * 0.5, new_h / h * 0.5);
    [cx - half_w, cy - half_h, cx + half_w, cy + half_h]
}

/// The cropped canvas, sampled bilinearly. Areas off the photo are black.
pub fn warp(source: &RgbaImage, geometry: &Geometry) -> RgbaImage {
    let (w, h) = source.dimensions();
    let (out_w, out_h) = geometry.output_size(w, h);
    let [left, top, right, bottom] = geometry.crop;
    let (fw, fh) = (w as f32, h as f32);
    let [cw, ch] = canvas_size(geometry.angle, fw, fh);
    let (step_x, step_y) = ((right - left) * cw / out_w as f32, (bottom - top) * ch / out_h as f32);
    let to_source = geometry.mapper(cw, ch);
    let src = source.as_raw();
    let stride = w as usize * 4;
    let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
    out.par_chunks_mut(out_w as usize * 4).enumerate().for_each(|(y, row)| {
        let cy = top * ch + (y as f32 + 0.5) * step_y;
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let cx = left * cw + (x as f32 + 0.5) * step_x;
            let (sx, sy) = to_source(cx, cy);
            let (sx, sy) = (sx - 0.5, sy - 0.5);
            if sx < -0.5 || sy < -0.5 || sx > fw - 0.5 || sy > fh - 0.5 {
                pixel.copy_from_slice(&[0, 0, 0, 255]);
                continue;
            }
            let (sx, sy) = (sx.clamp(0.0, fw - 1.0), sy.clamp(0.0, fh - 1.0));
            let (x0, y0) = (sx as usize, sy as usize);
            let (x1, y1) = ((x0 + 1).min(w as usize - 1), (y0 + 1).min(h as usize - 1));
            let (tx, ty) = (sx - x0 as f32, sy - y0 as f32);
            for channel in 0..4 {
                let at = |x: usize, y: usize| f32::from(src[y * stride + x * 4 + channel]);
                let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
                let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
                pixel[channel] = (top + (bottom - top) * ty).round() as u8;
            }
        }
    });
    RgbaImage::from_raw(out_w, out_h, out).expect("buffer matches the output size")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(angle: f32) -> Geometry {
        Geometry {
            angle,
            vertical: 0.0,
            horizontal: 0.0,
            corners: [[0.0; 2]; 4],
            flip_h: false,
            flip_v: false,
            crop: FULL,
        }
    }

    #[test]
    fn corners_land_where_they_are_dragged() {
        let (w, h) = (400.0, 300.0);
        let sheared = Geometry {
            corners: [[0.1, 0.0], [0.1, 0.0], [0.0, 0.0], [0.0, 0.0]],
            ..turn(0.0)
        };
        let keystone = Geometry {
            corners: [[0.15, 0.05], [-0.1, 0.0], [0.05, -0.1], [0.0, 0.1]],
            ..turn(0.0)
        };
        for geometry in [sheared, keystone, Geometry { angle: 20.0, ..keystone }] {
            let quad = geometry.quad();
            assert!(is_convex(quad));
            let unturned = Geometry { corners: [[0.0; 2]; 4], ..geometry };
            for (i, corner) in quad.iter().enumerate() {
                let (x, y) = geometry.canvas_to_source(corner[0] * w, corner[1] * h, w, h);
                let (ex, ey) = unturned.canvas_to_source(SQUARE[i][0] * w, SQUARE[i][1] * h, w, h);
                assert!((x - ex).abs() < 0.05 && (y - ey).abs() < 0.05, "corner {i}: {x},{y} not {ex},{ey}");
            }
            assert!(!inside(FULL, &geometry, w, h));
            let kept = constrain(FULL, &geometry, w, h);
            assert!(inside(kept, &geometry, w, h));
            assert!(kept[2] - kept[0] > 0.4);
        }
        let (x, _) = sheared.canvas_to_source(0.05 * w, 0.0, w, h);
        assert!(x < 0.0, "left of the sheared top edge is off the photo");
        let (x, _) = sheared.canvas_to_source(0.05 * w, h, w, h);
        assert!(x > 0.0, "but the bottom edge did not move");

        let inward = Geometry {
            vertical: 15.0,
            corners: [[0.1, 0.1], [-0.1, 0.1], [-0.1, -0.1], [0.1, -0.1]],
            ..turn(0.0)
        };
        let (x, y) = inward.canvas_to_source(0.05 * w, 0.05 * h, w, h);
        assert!(x < 0.0 || y < 0.0, "the widened top stays cut at the outline");

        assert!(!is_convex([[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]]), "crossed");
        assert!(!is_convex([[0.0, 0.0], [1.0, 0.0], [0.4, 0.4], [0.0, 1.0]]), "dented");
        assert!(!is_convex([[0.0, 0.0], [1.0, 0.0], [1.0, 0.01], [0.0, 0.01]]), "a sliver");
    }

    #[test]
    fn crops_flip_and_stay_on_the_photo() {
        let mut source = RgbaImage::new(4, 2);
        source.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let flipped = warp(&source, &Geometry { flip_h: true, ..turn(0.0) });
        assert_eq!(flipped.get_pixel(3, 0)[0], 255);
        let half = Geometry {
            crop: [0.0, 0.0, 0.5, 1.0],
            ..turn(0.0)
        };
        assert_eq!(warp(&source, &half).dimensions(), (2, 2));
        assert!(!inside(FULL, &turn(10.0), 400.0, 300.0));
        let kept = constrain(FULL, &turn(10.0), 400.0, 300.0);
        assert!(inside(kept, &turn(10.0), 400.0, 300.0));
        assert!(kept[2] - kept[0] > 0.6);
        let square = with_aspect(FULL, 1.0, 400.0, 300.0);
        assert!(((square[2] - square[0]) * 400.0 - 300.0).abs() < 0.5);
    }

    #[test]
    fn perspective_widens_one_side_and_keeps_the_center() {
        let (w, h) = (400.0, 300.0);
        let up = Geometry { vertical: 12.0, ..turn(0.0) };
        let (cx, cy) = up.canvas_to_source(w / 2.0, h / 2.0, w, h);
        assert!((cx - w / 2.0).abs() < 0.01 && (cy - h / 2.0).abs() < 0.01);
        let (top_left, _) = up.canvas_to_source(0.0, 0.0, w, h);
        let (bottom_left, _) = up.canvas_to_source(0.0, h, w, h);
        assert!(top_left > 0.0, "the top shows less of the photo, so it is widened");
        assert!(bottom_left < 0.0, "the bottom shows more, past the photo's edge");

        let right = Geometry { horizontal: 12.0, ..turn(0.0) };
        let (_, right_top) = right.canvas_to_source(w, 0.0, w, h);
        let (_, left_top) = right.canvas_to_source(0.0, 0.0, w, h);
        assert!(right_top > 0.0 && left_top < 0.0);

        let both = Geometry { horizontal: -20.0, ..up };
        assert!(!inside(FULL, &both, w, h));
        let kept = constrain(FULL, &both, w, h);
        assert!(inside(kept, &both, w, h));
        assert!(kept[2] - kept[0] > 0.4);
        assert_eq!(warp(&RgbaImage::new(40, 30), &both).dimensions(), (40, 30));

        let steep = Geometry { vertical: 45.0, horizontal: 45.0, ..turn(0.0) };
        let kept = constrain(FULL, &steep, w, h);
        assert!(inside(kept, &steep, w, h), "even the steepest tilt leaves a crop");
    }

    #[test]
    fn turns_any_angle_and_stands_the_canvas_on_its_side_past_45() {
        let mut source = RgbaImage::new(4, 2);
        source.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let turned = |angle: f32| warp(&source, &turn(angle));
        let quarter = turned(90.0);
        assert_eq!(quarter.dimensions(), (2, 4));
        assert_eq!(quarter.get_pixel(1, 0)[0], 255, "the top-left corner turns to the top right");
        let left = turned(-90.0);
        assert_eq!(left.get_pixel(0, 3)[0], 255, "and to the bottom left the other way");
        let half = turned(180.0);
        assert_eq!(half.dimensions(), (4, 2));
        assert_eq!(half.get_pixel(3, 1)[0], 255);

        assert_eq!(canvas_size(45.0, 400.0, 300.0), [400.0, 300.0], "edits at the old ±45° limit keep their canvas");
        assert_eq!(canvas_size(-45.0, 400.0, 300.0), [400.0, 300.0]);
        assert_eq!(canvas_size(46.0, 400.0, 300.0), [300.0, 400.0]);
        assert_eq!(canvas_size(-120.0, 400.0, 300.0), [300.0, 400.0]);
        assert_eq!(canvas_size(170.0, 400.0, 300.0), [400.0, 300.0]);
        assert!(inside(FULL, &turn(90.0), 300.0, 400.0));
        assert!(inside(FULL, &turn(180.0), 400.0, 300.0));
        for angle in [60.0, 100.0, 150.0, -135.0] {
            let [cw, ch] = canvas_size(angle, 400.0, 300.0);
            let kept = constrain(FULL, &turn(angle), cw, ch);
            assert!(inside(kept, &turn(angle), cw, ch), "{angle}");
            assert!(kept[2] - kept[0] > 0.3, "{angle}");
        }
    }
}
