//! Rectangle math for face boxes. Coordinates are fractions of the displayed image.

use crate::types::RelRect;

pub fn clamp_rect(mut rect: RelRect) -> RelRect {
    rect.w = rect.w.max(0.0);
    rect.h = rect.h.max(0.0);
    rect.x = rect.x.clamp(0.0, 1.0);
    rect.y = rect.y.clamp(0.0, 1.0);
    if rect.x + rect.w > 1.0 {
        rect.w = (1.0 - rect.x).max(0.0);
    }
    if rect.y + rect.h > 1.0 {
        rect.h = (1.0 - rect.y).max(0.0);
    }
    rect
}

/// 90 degrees clockwise in normalized image space.
pub fn rotate_cw(rect: RelRect) -> RelRect {
    clamp_rect(RelRect {
        x: 1.0 - (rect.y + rect.h),
        y: rect.x,
        w: rect.h,
        h: rect.w,
    })
}

pub fn rotate_ccw(rect: RelRect) -> RelRect {
    clamp_rect(RelRect {
        x: rect.y,
        y: 1.0 - (rect.x + rect.w),
        w: rect.h,
        h: rect.w,
    })
}

pub fn iou(a: RelRect, b: RelRect) -> f32 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a.w * a.h + b.w * b.h - inter;
    if union <= f32::EPSILON {
        0.0
    } else {
        inter / union
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_turns_restore_the_rectangle() {
        let start = RelRect {
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.25,
        };
        let mut rect = start;
        for _ in 0..4 {
            rect = rotate_cw(rect);
        }
        assert!((rect.x - start.x).abs() < 1e-5);
        assert!((rect.y - start.y).abs() < 1e-5);
        assert!((rect.w - start.w).abs() < 1e-5);
        assert!((rect.h - start.h).abs() < 1e-5);
    }

    #[test]
    fn identical_rects_have_iou_one() {
        let rect = RelRect {
            x: 0.2,
            y: 0.2,
            w: 0.4,
            h: 0.4,
        };
        assert!((iou(rect, rect) - 1.0).abs() < 1e-5);
    }
}
