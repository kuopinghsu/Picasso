//! A route's height along its length, read from its GPS track files and
//! drawn in the bottom-right corner of its map picture (`docs/TRAVEL.md`).

use std::path::Path;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

/// Distance along the route and height there, both in meters.
pub type Profile = Vec<[f32; 2]>;

/// At most this many points are kept; the chart is narrower than that.
const MOST: usize = 240;

/// The profile of the tracks in `paths`, one after the other, from the
/// points that have a height. A file whose heights are all 0 has none:
/// some apps write 0 where they recorded nothing. `None` when fewer than
/// two points have a height, or the route doesn't go anywhere.
pub fn profile_of(paths: &[&Path]) -> Option<Profile> {
    let mut out: Profile = Vec::new();
    let mut along = 0.0f64;
    for path in paths {
        let Ok(loaded) = crate::track_edit::load(path, crate::track_edit::Settings::default()) else { continue };
        let heights = loaded.points.iter().any(|point| point.ele.is_some_and(|ele| ele.is_finite() && ele != 0.0));
        let mut last = None;
        for point in &loaded.points {
            if let Some(previous) = last {
                along += crate::track_edit::distance_m(previous, point);
            }
            last = Some(point);
            if let Some(ele) = point.ele.filter(|ele| heights && ele.is_finite()) {
                out.push([along as f32, ele as f32]);
            }
        }
    }
    thin(out)
}

fn thin(points: Profile) -> Option<Profile> {
    let (first, last) = (*points.first()?, *points.last()?);
    if points.len() < 2 || last[0] <= first[0] {
        return None;
    }
    if points.len() <= MOST {
        return Some(points);
    }
    let step = (points.len() - 1) as f64 / (MOST - 1) as f64;
    Some((0..MOST).map(|n| points[((n as f64 * step).round() as usize).min(points.len() - 1)]).collect())
}

/// Size of the chart's panel and its gap from the picture's edges, before `scale`.
const PANEL: (f32, f32) = (230.0, 96.0);
const MARGIN: f32 = 12.0;
const PAD: f32 = 10.0;
/// Room for the height labels at the left of the chart.
const LABELS: f32 = 52.0;
const TEXT: [u8; 3] = [0x3a, 0x3a, 0x3a];
/// How much the panel veils the map under it, of 255: enough for the
/// labels to read, little enough for the map to show through.
pub const VEIL: u8 = 140;

/// Draws `profile` in the bottom-right corner of `pixmap`, on a light
/// see-through panel over the map, in `ink`, with the highest and lowest heights beside it.
pub fn draw(pixmap: &mut tiny_skia::Pixmap, profile: &[[f32; 2]], scale: f32, ink: [u8; 3]) {
    let (Some(first), Some(last)) = (profile.first(), profile.last()) else { return };
    let (width, height) = (pixmap.width() as f32, pixmap.height() as f32);
    let (panel_w, panel_h) = (PANEL.0 * scale, PANEL.1 * scale);
    let (left, top) = (width - panel_w - MARGIN * scale, height - panel_h - MARGIN * scale);
    if left < 0.0 || top < 0.0 {
        return;
    }
    let paint = |[r, g, b]: [u8; 3], alpha: u8| {
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(r, g, b, alpha);
        paint.anti_alias = true;
        paint
    };
    let none = tiny_skia::Transform::identity();
    if let Some(panel) = rounded(left, top, panel_w, panel_h, 6.0 * scale) {
        pixmap.fill_path(&panel, &paint([255, 255, 255], VEIL), tiny_skia::FillRule::Winding, none, None);
    }
    let low = profile.iter().map(|point| point[1]).fold(f32::INFINITY, f32::min);
    let high = profile.iter().map(|point| point[1]).fold(f32::NEG_INFINITY, f32::max);
    // A flat route still shows as a line a little above the bottom.
    let span = (high - low).max(20.0);
    let (x0, x1) = (left + (PAD + LABELS) * scale, left + panel_w - PAD * scale);
    let (y0, y1) = (top + PAD * scale, top + panel_h - PAD * scale);
    let length = last[0] - first[0];
    let at = |[along, ele]: [f32; 2]| (x0 + (along - first[0]) / length * (x1 - x0), y1 - (ele - low) / span * (y1 - y0));
    let mut area = tiny_skia::PathBuilder::new();
    let mut line = tiny_skia::PathBuilder::new();
    area.move_to(x0, y1);
    for (n, point) in profile.iter().enumerate() {
        let (x, y) = at(*point);
        area.line_to(x, y);
        if n == 0 {
            line.move_to(x, y);
        } else {
            line.line_to(x, y);
        }
    }
    area.line_to(x1, y1);
    area.close();
    if let Some(area) = area.finish() {
        pixmap.fill_path(&area, &paint(ink, 70), tiny_skia::FillRule::Winding, none, None);
    }
    if let Some(line) = line.finish() {
        let stroke = tiny_skia::Stroke { width: 2.0 * scale, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        pixmap.stroke_path(&line, &paint(ink, 255), &stroke, none, None);
    }
    let size = 12.0 * scale;
    let label_x = left + PAD * scale;
    text(pixmap, &meters(high), label_x, y0 + size * 0.8, size);
    text(pixmap, &meters(low), label_x, y1, size);
}

/// A height as the chart labels it.
fn meters(ele: f32) -> String {
    format!("{} m", ele.round() as i64)
}

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let mut path = tiny_skia::PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(x + w - r, y);
    path.quad_to(x + w, y, x + w, y + r);
    path.line_to(x + w, y + h - r);
    path.quad_to(x + w, y + h, x + w - r, y + h);
    path.line_to(x + r, y + h);
    path.quad_to(x, y + h, x, y + h - r);
    path.line_to(x, y + r);
    path.quad_to(x, y, x + r, y);
    path.close();
    path.finish()
}

/// Writes `words` with its baseline at `baseline`, in the interface's own
/// font, laid on what is there.
fn text(pixmap: &mut tiny_skia::Pixmap, words: &str, x: f32, baseline: f32, size: f32) {
    let Ok(font) = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT) else { return };
    let scaled = font.as_scaled(PxScale::from(size));
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let data = pixmap.data_mut();
    let mut pen = x;
    for c in words.chars() {
        let glyph = scaled.scaled_glyph(c);
        let advance = scaled.h_advance(glyph.id);
        let mut glyph = glyph;
        glyph.position = ab_glyph::point(pen, baseline);
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let (px, py) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= width || py >= height {
                    return;
                }
                let at = (py as usize * width as usize + px as usize) * 4;
                let coverage = coverage.clamp(0.0, 1.0);
                // The map picture is opaque, so the colour is blended as is.
                for (channel, ink) in data[at..at + 3].iter_mut().zip(TEXT) {
                    *channel = (f32::from(*channel) * (1.0 - coverage) + f32::from(ink) * coverage).round() as u8;
                }
            });
        }
        pen += advance;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_profile_is_thinned_and_a_short_one_has_none() {
        let long: Profile = (0..1000).map(|n| [n as f32, (n % 50) as f32]).collect();
        let thinned = thin(long).unwrap();
        assert_eq!(thinned.len(), MOST);
        assert_eq!((thinned[0][0], thinned[MOST - 1][0]), (0.0, 999.0), "it keeps both ends");
        assert_eq!(thin(vec![[0.0, 10.0]]), None);
        assert_eq!(thin(vec![[5.0, 10.0], [5.0, 12.0]]), None, "a route that goes nowhere");
    }

    #[test]
    fn the_profile_is_read_from_the_track_and_drawn_in_the_corner() {
        let dir = std::env::temp_dir().join(format!("picasso-elevation-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("walk.gpx");
        let points: String = (0..5).map(|n| format!("<trkpt lat=\"35.{n}\" lon=\"135.0\"><ele>{}</ele><time>2024-05-01T08:0{n}:00Z</time></trkpt>", 100 + n * 50)).collect();
        std::fs::write(&path, format!("<gpx><trk><trkseg>{points}</trkseg></trk></gpx>")).unwrap();
        let profile = profile_of(&[path.as_path()]).unwrap();
        assert_eq!(profile.len(), 5);
        assert_eq!((profile[0][1], profile[4][1]), (100.0, 300.0));
        let mut pixmap = tiny_skia::Pixmap::new(720, 400).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
        draw(&mut pixmap, &profile, 1.0, [0x1f, 0x7a, 0x7a]);
        let pixel = |x: u32, y: u32| pixmap.pixel(x, y).unwrap();
        assert_eq!(pixel(10, 10).red(), 0, "the rest of the map is left alone");
        let veiled = pixel(720 - 20, 400 - 100).red();
        assert!(veiled > 100 && veiled < 200, "the panel veils the map in the bottom-right corner, {veiled}");
        // A file whose heights are all 0 recorded none.
        let flat = dir.join("flat.gpx");
        std::fs::write(&flat, "<gpx><trk><trkseg><trkpt lat=\"35.0\" lon=\"135.0\"><ele>0</ele></trkpt><trkpt lat=\"35.1\" lon=\"135.0\"><ele>0</ele></trkpt></trkseg></trk></gpx>").unwrap();
        assert_eq!(profile_of(&[flat.as_path()]), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
