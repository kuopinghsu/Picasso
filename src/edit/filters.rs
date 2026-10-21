//! Image filters on single-channel float planes: box blur, the guided
//! filter that keeps edges for local tone mapping, a minimum filter for the
//! haze estimate, and resampling.

use rayon::prelude::*;

/// Mean over a `(2r + 1)²` window, shrinking at the borders.
pub fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let rows = blur_rows(src, w, r);
    let columns = transpose(&rows, w, h);
    let blurred = blur_rows(&columns, h, r);
    transpose(&blurred, h, w)
}

fn blur_rows(src: &[f32], w: usize, r: usize) -> Vec<f32> {
    let mut out = vec![0.0; src.len()];
    out.par_chunks_mut(w).zip(src.par_chunks(w)).for_each(|(out, line)| {
        let mut prefix = Vec::with_capacity(line.len() + 1);
        let mut sum = 0.0f64;
        prefix.push(0.0);
        for value in line {
            sum += f64::from(*value);
            prefix.push(sum);
        }
        let n = line.len();
        for (index, slot) in out.iter_mut().enumerate() {
            let start = index.saturating_sub(r);
            let end = (index + r + 1).min(n);
            *slot = ((prefix[end] - prefix[start]) / (end - start) as f64) as f32;
        }
    });
    out
}

fn transpose(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0; src.len()];
    out.par_chunks_mut(h).enumerate().for_each(|(x, column)| {
        for (y, slot) in column.iter_mut().enumerate() {
            *slot = src[y * w + x];
        }
    });
    out
}

/// Self-guided filter (He et al.): smooths `src` within regions while
/// keeping steps larger than about `sqrt(eps)`.
pub fn guided(src: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mean = box_blur(src, w, h, r);
    let squares: Vec<f32> = src.par_iter().map(|value| value * value).collect();
    let mean_sq = box_blur(&squares, w, h, r);
    let (a, b): (Vec<f32>, Vec<f32>) = mean
        .par_iter()
        .zip(mean_sq.par_iter())
        .map(|(mean, mean_sq)| {
            let variance = (mean_sq - mean * mean).max(0.0);
            let a = variance / (variance + eps);
            (a, mean - a * mean)
        })
        .unzip();
    let a = box_blur(&a, w, h, r);
    let b = box_blur(&b, w, h, r);
    src.par_iter()
        .zip(a.par_iter().zip(b.par_iter()))
        .map(|(value, (a, b))| a * value + b)
        .collect()
}

/// Minimum over a `(2r + 1)²` window.
pub fn min_filter(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let min_rows = |src: &[f32], w: usize| -> Vec<f32> {
        let mut out = vec![0.0; src.len()];
        out.par_chunks_mut(w).zip(src.par_chunks(w)).for_each(|(out, line)| {
            for (index, slot) in out.iter_mut().enumerate() {
                let start = index.saturating_sub(r);
                let end = (index + r + 1).min(line.len());
                *slot = line[start..end].iter().copied().fold(f32::INFINITY, f32::min);
            }
        });
        out
    };
    let rows = min_rows(src, w);
    let columns = min_rows(&transpose(&rows, w, h), h);
    transpose(&columns, h, w)
}

/// Area average down to at most `edge` pixels on the long side. Returns the
/// plane and its size.
pub fn shrink(src: &[f32], w: usize, h: usize, edge: usize) -> (Vec<f32>, usize, usize) {
    let factor = w.max(h).div_ceil(edge.max(1)).max(1);
    if factor == 1 {
        return (src.to_vec(), w, h);
    }
    let (sw, sh) = (w.div_ceil(factor), h.div_ceil(factor));
    let mut out = vec![0.0; sw * sh];
    out.par_chunks_mut(sw).enumerate().for_each(|(sy, row)| {
        for (sx, slot) in row.iter_mut().enumerate() {
            let (mut sum, mut count) = (0.0f32, 0usize);
            for y in sy * factor..((sy + 1) * factor).min(h) {
                for x in sx * factor..((sx + 1) * factor).min(w) {
                    sum += src[y * w + x];
                    count += 1;
                }
            }
            *slot = sum / count.max(1) as f32;
        }
    });
    (out, sw, sh)
}

/// A plane smaller than the image it describes, read with bilinear
/// interpolation at image fractions.
pub struct Low {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Low {
    pub fn sample(&self, fx: f32, fy: f32) -> f32 {
        if self.data.is_empty() {
            return 0.0;
        }
        let x = (fx * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (fy * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| self.data[y * self.w + x];
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
        top + (bottom - top) * ty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_and_guided_filter_keep_steps() {
        let (w, h) = (40, 10);
        let step: Vec<f32> = (0..w * h).map(|i| if i % w < 20 { 0.0 } else { 4.0 }).collect();
        let blurred = box_blur(&step, w, h, 3);
        assert!((blurred[5 * w + 2] - 0.0).abs() < 1e-6);
        assert!(blurred[5 * w + 19] > 0.5 && blurred[5 * w + 19] < 3.5);
        let kept = guided(&step, w, h, 3, 0.01);
        assert!(kept[5 * w + 19] < 0.2 && kept[5 * w + 20] > 3.8, "edges survive");
        let minimum = min_filter(&step, w, h, 2);
        assert_eq!(minimum[5 * w + 21], 0.0);
        let (small, sw, sh) = shrink(&step, w, h, 10);
        assert_eq!((sw, sh), (10, 3));
        assert_eq!(small.len(), 30);
    }
}
