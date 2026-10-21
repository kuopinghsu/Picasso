//! Face detection and a small on-device descriptor used to group the same person.
//!
//! Detection is SeetaFace (bundled model, BSD). Grouping compares a HOG plus a
//! normalized pixel patch. It is meant for frontal faces in the same library,
//! and a wrong group can be split from the viewer.

use std::io::Cursor;

use image::{DynamicImage, GenericImageView, GrayImage, Luma};

use crate::geom::{clamp_rect, iou};
use crate::image_io::face_crop;
use crate::types::RelRect;

const MODEL: &[u8] = include_bytes!("../assets/seeta_fd_frontal_v1.0.bin");
pub const EMBEDDING_LEN: usize = 128 + 256;
const ASSIGN_HINT: f32 = 0.93;

pub fn open_detector() -> Result<Box<dyn rustface::Detector>, String> {
    let model = rustface::read_model(Cursor::new(MODEL)).map_err(|err| format!("face model: {err}"))?;
    let mut detector = rustface::create_detector_with_model(model);
    detector.set_min_face_size(20);
    detector.set_score_thresh(2.0);
    detector.set_pyramid_scale_factor(0.8);
    detector.set_slide_window_step(4, 4);
    Ok(detector)
}

#[derive(Clone, Debug)]
pub struct Detection {
    pub rect: RelRect,
    pub score: f32,
    pub embedding: Vec<f32>,
}

pub fn detect(detector: &mut dyn rustface::Detector, image: &DynamicImage) -> Vec<Detection> {
    let (orig_w, orig_h) = image.dimensions();
    if orig_w < 24 || orig_h < 24 {
        return Vec::new();
    }
    let max_edge = 900u32;
    let long = orig_w.max(orig_h);
    let gray = if long > max_edge {
        let scale = max_edge as f32 / long as f32;
        let nw = ((orig_w as f32) * scale).round().max(1.0) as u32;
        let nh = ((orig_h as f32) * scale).round().max(1.0) as u32;
        image
            .resize(nw, nh, image::imageops::FilterType::Triangle)
            .to_luma8()
    } else {
        image.to_luma8()
    };
    if gray.width() < 20 || gray.height() < 20 {
        return Vec::new();
    }
    let data = rustface::ImageData::new(gray.as_raw(), gray.width(), gray.height());
    let found = detector.detect(&data);
    let sx = orig_w as f32 / gray.width() as f32;
    let sy = orig_h as f32 / gray.height() as f32;
    let mut boxes = Vec::new();
    for face in found {
        let bbox = face.bbox();
        let rect = clamp_rect(RelRect {
            x: bbox.x() as f32 * sx / orig_w as f32,
            y: bbox.y() as f32 * sy / orig_h as f32,
            w: bbox.width() as f32 * sx / orig_w as f32,
            h: bbox.height() as f32 * sy / orig_h as f32,
        });
        if rect.w < 0.01 || rect.h < 0.01 {
            continue;
        }
        boxes.push((rect, face.score() as f32));
    }
    boxes.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut kept: Vec<(RelRect, f32)> = Vec::new();
    for (rect, score) in boxes {
        if kept.iter().any(|(prev, _)| iou(*prev, rect) > 0.4) {
            continue;
        }
        kept.push((rect, score));
    }
    kept.into_iter()
        .map(|(rect, score)| Detection {
            embedding: embedding(image, rect),
            rect,
            score,
        })
        .collect()
}

pub fn embedding(image: &DynamicImage, rect: RelRect) -> Vec<f32> {
    let patch = DynamicImage::ImageRgba8(face_crop(image, rect, 32)).into_luma8();
    let mut hog = hog32(&patch);
    let mut pixels = pixel_patch(&patch);
    l2_normalize(&mut hog);
    l2_normalize(&mut pixels);
    let mut all = hog;
    all.append(&mut pixels);
    l2_normalize(&mut all);
    if all.len() != EMBEDDING_LEN {
        all.resize(EMBEDDING_LEN, 0.0);
    }
    all
}

fn hog32(image: &GrayImage) -> Vec<f32> {
    let mut hist = vec![0f32; 128];
    if image.width() < 3 || image.height() < 3 {
        return hist;
    }
    for y in 1..image.height() - 1 {
        for x in 1..image.width() - 1 {
            let gx = image.get_pixel(x + 1, y)[0] as f32 - image.get_pixel(x - 1, y)[0] as f32;
            let gy = image.get_pixel(x, y + 1)[0] as f32 - image.get_pixel(x, y - 1)[0] as f32;
            let mag = (gx * gx + gy * gy).sqrt();
            if mag < 1.0 {
                continue;
            }
            let mut angle = gy.atan2(gx);
            if angle < 0.0 {
                angle += std::f32::consts::PI;
            }
            let bin_f = (angle / std::f32::consts::PI * 8.0).clamp(0.0, 7.999);
            let bin0 = bin_f.floor() as usize;
            let bin1 = (bin0 + 1) % 8;
            let frac = bin_f - bin0 as f32;
            let cell_x = ((x - 1) / 8).min(3) as usize;
            let cell_y = ((y - 1) / 8).min(3) as usize;
            let base = (cell_y * 4 + cell_x) * 8;
            hist[base + bin0] += mag * (1.0 - frac);
            hist[base + bin1] += mag * frac;
        }
    }
    hist
}

fn pixel_patch(image: &GrayImage) -> Vec<f32> {
    let small = image::imageops::resize(image, 16, 16, image::imageops::FilterType::Triangle);
    let mut values: Vec<f32> = small.pixels().map(|Luma([v])| *v as f32 / 255.0).collect();
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    for value in &mut values {
        *value -= mean;
    }
    values
}

pub fn l2_normalize(values: &mut [f32]) {
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for value in values {
            *value /= norm;
        }
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>().clamp(-1.0, 1.0)
}

pub fn blobs_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

pub fn vec_to_blob(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn average_embeddings(rows: &[Vec<f32>]) -> Vec<f32> {
    if rows.is_empty() {
        return vec![0.0; EMBEDDING_LEN];
    }
    let mut acc = vec![0f32; rows[0].len()];
    for row in rows {
        for (slot, value) in acc.iter_mut().zip(row) {
            *slot += *value;
        }
    }
    l2_normalize(&mut acc);
    acc
}

/// Minimum cosine similarity for automatically attaching a new face to a person.
pub fn assign_threshold() -> f32 {
    ASSIGN_HINT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_patches_score_one() {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(48, 48, |x, y| {
            image::Rgb([(x * 3) as u8, (y * 5) as u8, 40])
        }));
        let rect = RelRect {
            x: 0.1,
            y: 0.1,
            w: 0.8,
            h: 0.8,
        };
        let emb = embedding(&image, rect);
        assert_eq!(emb.len(), EMBEDDING_LEN);
        assert!((cosine(&emb, &emb) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn detector_opens_and_scans_a_blank_image() {
        let mut detector = open_detector().expect("model");
        let image = DynamicImage::ImageLuma8(GrayImage::from_pixel(80, 80, Luma([180])));
        let _ = detect(&mut *detector, &image);
    }
}
