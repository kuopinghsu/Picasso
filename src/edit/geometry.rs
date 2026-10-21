//! Straighten, flips, and crop. The canvas is the photo flipped and then
//! turned by the straighten angle about its center, at the photo's size;
//! the crop is a rectangle on that canvas in fractions.

use image::RgbaImage;
use rayon::prelude::*;

use super::settings::{Control, Settings};

pub const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// Clockwise, in degrees.
    pub angle: f32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub crop: [f32; 4],
}

impl Geometry {
    pub fn from_settings(settings: &Settings) -> Geometry {
        Geometry {
            angle: settings.get(Control::STRAIGHTEN),
            flip_h: settings.get(Control::FLIP_H) >= 0.5,
            flip_v: settings.get(Control::FLIP_V) >= 0.5,
            crop: settings.crop(),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.angle == 0.0 && !self.flip_h && !self.flip_v && self.crop == FULL
    }

    /// Size of the result for a `w`×`h` photo.
    pub fn output_size(&self, w: u32, h: u32) -> (u32, u32) {
        let [left, top, right, bottom] = self.crop;
        (
            (((right - left) * w as f32).round() as u32).max(1),
            (((bottom - top) * h as f32).round() as u32).max(1),
        )
    }

    /// Photo position, in pixels, under a canvas position in pixels.
    pub fn canvas_to_source(&self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32) {
        let (cx, cy) = (w * 0.5, h * 0.5);
        let (dx, dy) = (x - cx, y - cy);
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let mut rx = dx * cos + dy * sin;
        let mut ry = -dx * sin + dy * cos;
        if self.flip_h {
            rx = -rx;
        }
        if self.flip_v {
            ry = -ry;
        }
        (rx + cx, ry + cy)
    }
}

/// Whether every corner of `crop` covers the photo after turning by `angle`.
pub fn inside(crop: [f32; 4], angle: f32, w: f32, h: f32) -> bool {
    let geometry = Geometry {
        angle,
        flip_h: false,
        flip_v: false,
        crop,
    };
    let [left, top, right, bottom] = crop;
    [(left, top), (right, top), (left, bottom), (right, bottom)].into_iter().all(|(fx, fy)| {
        let (x, y) = geometry.canvas_to_source(fx * w, fy * h, w, h);
        x >= -0.5 && y >= -0.5 && x <= w + 0.5 && y <= h + 0.5
    })
}

/// `crop` shrunk about its center, keeping its shape, until no corner shows
/// the empty area around a straightened photo.
pub fn constrain(crop: [f32; 4], angle: f32, w: f32, h: f32) -> [f32; 4] {
    if inside(crop, angle, w, h) {
        return crop;
    }
    let [left, top, right, bottom] = crop;
    let scaled = |center: [f32; 2], scale: f32| {
        let (half_w, half_h) = ((right - left) * 0.5 * scale, (bottom - top) * 0.5 * scale);
        [center[0] - half_w, center[1] - half_h, center[0] + half_w, center[1] + half_h]
    };
    let mut center = [(left + right) * 0.5, (top + bottom) * 0.5];
    if !inside(scaled(center, 0.01), angle, w, h) {
        center = [0.5, 0.5];
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let middle = (low + high) * 0.5;
        if inside(scaled(center, middle), angle, w, h) {
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
    let (step_x, step_y) = ((right - left) * fw / out_w as f32, (bottom - top) * fh / out_h as f32);
    let src = source.as_raw();
    let stride = w as usize * 4;
    let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
    out.par_chunks_mut(out_w as usize * 4).enumerate().for_each(|(y, row)| {
        let cy = top * fh + (y as f32 + 0.5) * step_y;
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let cx = left * fw + (x as f32 + 0.5) * step_x;
            let (sx, sy) = geometry.canvas_to_source(cx, cy, fw, fh);
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

    #[test]
    fn crops_flip_and_stay_on_the_photo() {
        let mut source = RgbaImage::new(4, 2);
        source.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let flipped = warp(
            &source,
            &Geometry {
                angle: 0.0,
                flip_h: true,
                flip_v: false,
                crop: FULL,
            },
        );
        assert_eq!(flipped.get_pixel(3, 0)[0], 255);
        let half = Geometry {
            angle: 0.0,
            flip_h: false,
            flip_v: false,
            crop: [0.0, 0.0, 0.5, 1.0],
        };
        assert_eq!(warp(&source, &half).dimensions(), (2, 2));
        assert!(!inside(FULL, 10.0, 400.0, 300.0));
        let kept = constrain(FULL, 10.0, 400.0, 300.0);
        assert!(inside(kept, 10.0, 400.0, 300.0));
        assert!(kept[2] - kept[0] > 0.6);
        let square = with_aspect(FULL, 1.0, 400.0, 300.0);
        assert!(((square[2] - square[0]) * 400.0 - 300.0).abs() < 0.5);
    }
}
