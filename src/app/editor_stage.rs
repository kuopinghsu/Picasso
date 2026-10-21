//! The stage while editing: before and after views, the crop frame, mask
//! handles, and the white balance eyedropper.

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::edit::{self, Kind, Settings, Shape};
use crate::i18n::tr;
use crate::types::Photo;

use super::editor::{Compare, EditSession, Live, Tool};
use super::theme::{self, ACCENT, WHITE};
use super::PicassoApp;

#[derive(Clone, Copy, Debug)]
pub(super) enum StageDrag {
    /// A crop handle, by the crop edges it moves along x and y.
    Edge { x: Option<usize>, y: Option<usize> },
    MoveCrop { start: [f32; 4], from: [f32; 2] },
    Mask { handle: MaskHandle, start: Shape, from: [f32; 2] },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum MaskHandle {
    From,
    To,
    Center,
    RadiusX,
    RadiusY,
}

const CROP_HANDLES: [(Option<usize>, Option<usize>); 8] = [
    (Some(0), Some(1)),
    (Some(2), Some(1)),
    (Some(0), Some(3)),
    (Some(2), Some(3)),
    (Some(0), None),
    (Some(2), None),
    (None, Some(1)),
    (None, Some(3)),
];

/// Canvas fractions to screen positions and back, for a photo drawn in
/// `rect` after `turns` clockwise quarter turns and showing the `view` part
/// of the canvas.
struct Mapping {
    rect: Rect,
    turns: usize,
    view: [f32; 4],
}

impl Mapping {
    fn to_screen(&self, at: [f32; 2]) -> Pos2 {
        let u = (at[0] - self.view[0]) / (self.view[2] - self.view[0]).max(1e-6);
        let v = (at[1] - self.view[1]) / (self.view[3] - self.view[1]).max(1e-6);
        let (s, t) = match self.turns {
            1 => (1.0 - v, u),
            2 => (1.0 - u, 1.0 - v),
            3 => (v, 1.0 - u),
            _ => (u, v),
        };
        self.rect.min + Vec2::new(s * self.rect.width(), t * self.rect.height())
    }

    fn to_canvas(&self, pos: Pos2) -> [f32; 2] {
        let s = (pos.x - self.rect.left()) / self.rect.width().max(1.0);
        let t = (pos.y - self.rect.top()) / self.rect.height().max(1.0);
        let (u, v) = match self.turns {
            1 => (t, 1.0 - s),
            2 => (1.0 - s, 1.0 - t),
            3 => (1.0 - t, s),
            _ => (s, t),
        };
        [
            self.view[0] + u * (self.view[2] - self.view[0]),
            self.view[1] + v * (self.view[3] - self.view[1]),
        ]
    }
}

fn turns(photo: &Photo) -> usize {
    photo.rotation.div_euclid(90).rem_euclid(4) as usize
}

impl PicassoApp {
    /// Paints the editor's rendering of `photo`. Returns false while no
    /// preview is ready, so the stage shows the regular view instead.
    pub(super) fn paint_edit_stage(&self, ui: &mut egui::Ui, stage: Rect, image_rect: Rect, photo: &Photo) -> bool {
        let Some(session) = self.edit.as_ref().filter(|session| session.id == photo.id) else { return false };
        let Some(preview) = session.preview() else { return false };
        let before = session.before().unwrap_or(preview);
        let painter = ui.painter_at(stage);
        let rotation = photo.rotation;
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        match session.compare() {
            Compare::Off => theme::paint_image(&painter, preview, image_rect, full, rotation),
            Compare::Before => {
                theme::paint_image(&painter, before, image_rect, full, rotation);
                pill(&painter, Pos2::new(stage.center().x, stage.top() + 22.0), tr("Before"));
            }
            Compare::SideBySide => {
                let gap = 8.0;
                let cell_size = Vec2::new((stage.width() - gap) * 0.5, stage.height());
                let size = image_rect.size();
                for (index, texture, label) in [(0, before, "Before"), (1, preview, "After")] {
                    let cell = Rect::from_min_size(stage.min + Vec2::new(index as f32 * (cell_size.x + gap), 0.0), cell_size);
                    let scale = (cell.width() / size.x).min(cell.height() / size.y);
                    let rect = Rect::from_center_size(cell.center(), size * scale);
                    theme::paint_image(&painter.with_clip_rect(cell), texture, rect, full, rotation);
                    pill(&painter, Pos2::new(cell.center().x, cell.top() + 22.0), tr(label));
                }
            }
            Compare::Split(fraction) => {
                theme::paint_image(&painter, preview, image_rect, full, rotation);
                let x = image_rect.left() + image_rect.width() * fraction;
                let left = Rect::from_min_max(image_rect.min, Pos2::new(x, image_rect.bottom()));
                theme::paint_image(&painter.with_clip_rect(left.intersect(stage)), before, image_rect, full, rotation);
                let top = image_rect.top().max(stage.top());
                let bottom = image_rect.bottom().min(stage.bottom());
                painter.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], Stroke::new(2.0, WHITE));
                let knob = Pos2::new(x, (top + bottom) * 0.5);
                painter.circle_filled(knob, 11.0, Color32::from_black_alpha(170));
                painter.circle_stroke(knob, 11.0, Stroke::new(1.5, WHITE));
                painter.text(knob, Align2::CENTER_CENTER, "⇔", FontId::proportional(13.0), WHITE);
                pill(&painter, Pos2::new(x - 44.0, top + 22.0), tr("Before"));
                pill(&painter, Pos2::new(x + 44.0, top + 22.0), tr("After"));
            }
        }
        true
    }

    /// Crop frame, mask handles, eyedropper, and split divider. Takes the
    /// pointer from the stage while a tool needs it.
    pub(super) fn edit_overlay(&mut self, ui: &mut egui::Ui, stage: Rect, image_rect: Rect, photo: &Photo) {
        let turns = turns(photo);
        let Some(session) = self.edit.as_mut().filter(|session| session.id == photo.id) else { return };
        if session.base.is_none() || session.preview().is_none() {
            return;
        }
        let canvas = session.canvas(photo);
        let settings = session.current();
        if session.tool == Tool::Crop {
            crop_overlay(ui, session, stage, image_rect, turns, canvas, &settings);
        } else if session.tool == Tool::Mask {
            mask_overlay(ui, session, stage, image_rect, turns, &settings);
        } else if session.picker {
            let response = ui
                .interact(image_rect.intersect(stage), egui::Id::unique("edit-picker"), Sense::click())
                .on_hover_cursor(CursorIcon::Crosshair)
                .on_hover_text_at_pointer(tr("Click something that should be neutral gray"));
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let map = Mapping {
                        rect: image_rect,
                        turns,
                        view: settings.crop(),
                    };
                    let at = map.to_canvas(pos);
                    self.pick_white_balance(at);
                }
            }
        } else if let Compare::Split(fraction) = session.compare() {
            let x = image_rect.left() + image_rect.width() * fraction;
            let grip = Rect::from_center_size(Pos2::new(x, stage.center().y), Vec2::new(18.0, stage.height()));
            let response = ui
                .interact(grip, egui::Id::unique("edit-split"), Sense::drag())
                .on_hover_cursor(CursorIcon::ResizeHorizontal);
            if response.dragged() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let fraction = ((pos.x - image_rect.left()) / image_rect.width().max(1.0)).clamp(0.02, 0.98);
                    session.compare = Compare::Split(fraction);
                }
            }
        }
    }
}

fn pill(painter: &egui::Painter, center: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.0), WHITE);
    let rect = Rect::from_center_size(center, galley.size() + Vec2::new(16.0, 8.0));
    painter.rect_filled(rect, 10, Color32::from_black_alpha(160));
    painter.galley(rect.center() - galley.size() * 0.5, galley, WHITE);
}

fn set_live(session: &mut EditSession, kind: Kind, settings: Settings) {
    session.live = Some(Live {
        kind,
        settings,
        amend: false,
    });
}

/// The furthest step from `from` toward `to` that keeps the crop on the
/// straightened photo.
fn furthest_inside(from: [f32; 4], to: [f32; 4], angle: f32, canvas: [f32; 2]) -> [f32; 4] {
    if edit::inside(to, angle, canvas[0], canvas[1]) {
        return to;
    }
    let lerp = |t: f32| std::array::from_fn(|i| from[i] + (to[i] - from[i]) * t);
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..14 {
        let middle = (low + high) * 0.5;
        if edit::inside(lerp(middle), angle, canvas[0], canvas[1]) {
            low = middle;
        } else {
            high = middle;
        }
    }
    lerp(low)
}

fn resize_crop(start: [f32; 4], x: Option<usize>, y: Option<usize>, at: [f32; 2], ratio: Option<f32>, canvas: [f32; 2]) -> [f32; 4] {
    let min = 0.03;
    let mut crop = start;
    match x {
        Some(0) => crop[0] = at[0].clamp(0.0, crop[2] - min),
        Some(2) => crop[2] = at[0].clamp(crop[0] + min, 1.0),
        _ => {}
    }
    match y {
        Some(1) => crop[1] = at[1].clamp(0.0, crop[3] - min),
        Some(3) => crop[3] = at[1].clamp(crop[1] + min, 1.0),
        _ => {}
    }
    let Some(ratio) = ratio else { return crop };
    let [cw, ch] = canvas;
    match (x, y) {
        (Some(_), Some(edge)) => {
            let height = (crop[2] - crop[0]) * cw / ratio / ch;
            if edge == 1 {
                crop[1] = crop[3] - height;
            } else {
                crop[3] = crop[1] + height;
            }
        }
        (Some(_), None) => {
            let height = (crop[2] - crop[0]) * cw / ratio / ch;
            let middle = (crop[1] + crop[3]) * 0.5;
            crop[1] = middle - height * 0.5;
            crop[3] = middle + height * 0.5;
        }
        (None, Some(_)) => {
            let width = (crop[3] - crop[1]) * ch * ratio / cw;
            let middle = (crop[0] + crop[2]) * 0.5;
            crop[0] = middle - width * 0.5;
            crop[2] = middle + width * 0.5;
        }
        (None, None) => {}
    }
    crop
}

fn crop_overlay(ui: &mut egui::Ui, session: &mut EditSession, stage: Rect, image_rect: Rect, turns: usize, canvas: [f32; 2], settings: &Settings) {
    let map = Mapping {
        rect: image_rect,
        turns,
        view: edit::FULL,
    };
    let crop = settings.crop();
    let angle = settings.get(edit::Control::STRAIGHTEN);
    let frame = Rect::from_two_pos(map.to_screen([crop[0], crop[1]]), map.to_screen([crop[2], crop[3]]));
    let painter = ui.painter_at(stage);
    let shade = Color32::from_black_alpha(150);
    let outer = image_rect;
    for rect in [
        Rect::from_min_max(outer.min, Pos2::new(outer.right(), frame.top())),
        Rect::from_min_max(Pos2::new(outer.left(), frame.bottom()), outer.max),
        Rect::from_min_max(Pos2::new(outer.left(), frame.top()), Pos2::new(frame.left(), frame.bottom())),
        Rect::from_min_max(Pos2::new(frame.right(), frame.top()), Pos2::new(outer.right(), frame.bottom())),
    ] {
        if rect.is_positive() {
            painter.rect_filled(rect, 0, shade);
        }
    }
    paint_guides(&painter, frame, session.guide);
    painter.rect_stroke(frame, 0, Stroke::new(1.5, WHITE), egui::StrokeKind::Middle);
    let handle_at = |(x, y): (Option<usize>, Option<usize>)| {
        let fx = x.map_or((crop[0] + crop[2]) * 0.5, |i| crop[i]);
        let fy = y.map_or((crop[1] + crop[3]) * 0.5, |i| crop[i]);
        map.to_screen([fx, fy])
    };
    for handle in CROP_HANDLES {
        let pos = handle_at(handle);
        let size = if handle.0.is_some() && handle.1.is_some() { 10.0 } else { 8.0 };
        let rect = Rect::from_center_size(pos, Vec2::splat(size));
        painter.rect_filled(rect, 1, WHITE);
        painter.rect_stroke(rect, 1, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
    }

    let response = ui.interact(stage, egui::Id::unique("edit-crop"), Sense::click_and_drag());
    let nearest = |pos: Pos2| {
        CROP_HANDLES
            .iter()
            .map(|handle| (*handle, handle_at(*handle).distance(pos)))
            .filter(|(_, distance)| *distance < 14.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(handle, _)| handle)
    };
    if let Some(pos) = response.hover_pos() {
        let icon = match nearest(pos) {
            Some(handle) => {
                let offset = handle_at(handle) - frame.center();
                match (handle.0.is_some() && handle.1.is_some(), offset.x.abs() > 1.0, offset.y.abs() > 1.0) {
                    (true, _, _) if offset.x * offset.y > 0.0 => CursorIcon::ResizeNwSe,
                    (true, _, _) => CursorIcon::ResizeNeSw,
                    (false, true, _) => CursorIcon::ResizeHorizontal,
                    _ => CursorIcon::ResizeVertical,
                }
            }
            None if frame.contains(pos) => CursorIcon::Move,
            None => CursorIcon::Default,
        };
        ui.ctx().set_cursor_icon(icon);
    }
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            session.drag = match nearest(pos) {
                Some((x, y)) => Some(StageDrag::Edge { x, y }),
                None if frame.contains(pos) => Some(StageDrag::MoveCrop {
                    start: crop,
                    from: map.to_canvas(pos),
                }),
                None => None,
            };
        }
    }
    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            let at = map.to_canvas(pos);
            let wanted = match session.drag {
                Some(StageDrag::Edge { x, y }) => Some(resize_crop(crop, x, y, at, session.aspect_ratio(canvas, turns), canvas)),
                Some(StageDrag::MoveCrop { start, from }) => {
                    let (w, h) = (start[2] - start[0], start[3] - start[1]);
                    let left = (start[0] + at[0] - from[0]).clamp(0.0, 1.0 - w);
                    let top = (start[1] + at[1] - from[1]).clamp(0.0, 1.0 - h);
                    Some([left, top, left + w, top + h])
                }
                _ => None,
            };
            if let Some(wanted) = wanted {
                let mut next = settings.clone();
                next.set_crop(furthest_inside(crop, wanted, angle, canvas));
                set_live(session, Kind::Crop, next);
            }
        }
    }
    if response.drag_stopped() {
        session.drag = None;
    }
    if response.double_clicked() {
        session.tool = Tool::Adjust;
    }
}

fn paint_guides(painter: &egui::Painter, frame: Rect, guide: usize) {
    let stroke = Stroke::new(1.0, Color32::from_white_alpha(110));
    let vertical = |t: f32| {
        let x = frame.left() + frame.width() * t;
        painter.line_segment([Pos2::new(x, frame.top()), Pos2::new(x, frame.bottom())], stroke);
    };
    let horizontal = |t: f32| {
        let y = frame.top() + frame.height() * t;
        painter.line_segment([Pos2::new(frame.left(), y), Pos2::new(frame.right(), y)], stroke);
    };
    match guide {
        0 => {
            for t in [1.0 / 3.0, 2.0 / 3.0] {
                vertical(t);
                horizontal(t);
            }
        }
        1 => {
            for t in [0.382, 0.618] {
                vertical(t);
                horizontal(t);
            }
        }
        2 => {
            for step in 1..8 {
                vertical(step as f32 / 8.0);
                horizontal(step as f32 / 8.0);
            }
        }
        3 => {
            painter.line_segment([frame.left_top(), frame.right_bottom()], stroke);
            painter.line_segment([frame.right_top(), frame.left_bottom()], stroke);
        }
        _ => {}
    }
}

fn shape_handles(shape: &Shape) -> Vec<(MaskHandle, [f32; 2])> {
    match *shape {
        Shape::Linear { from, to } => vec![
            (MaskHandle::Center, [(from[0] + to[0]) * 0.5, (from[1] + to[1]) * 0.5]),
            (MaskHandle::From, from),
            (MaskHandle::To, to),
        ],
        Shape::Radial { center, radius, .. } => vec![
            (MaskHandle::Center, center),
            (MaskHandle::RadiusX, [center[0] + radius[0], center[1]]),
            (MaskHandle::RadiusY, [center[0], center[1] + radius[1]]),
        ],
    }
}

fn move_handle(start: Shape, handle: MaskHandle, from: [f32; 2], at: [f32; 2]) -> Shape {
    let delta = [at[0] - from[0], at[1] - from[1]];
    let shifted = |p: [f32; 2]| [p[0] + delta[0], p[1] + delta[1]];
    match start {
        Shape::Linear { from: a, to: b } => match handle {
            MaskHandle::From => Shape::Linear { from: at, to: b },
            MaskHandle::To => Shape::Linear { from: a, to: at },
            _ => Shape::Linear {
                from: shifted(a),
                to: shifted(b),
            },
        },
        Shape::Radial { center, radius, feather } => match handle {
            MaskHandle::RadiusX => Shape::Radial {
                center,
                radius: [(at[0] - center[0]).abs().max(0.01), radius[1]],
                feather,
            },
            MaskHandle::RadiusY => Shape::Radial {
                center,
                radius: [radius[0], (at[1] - center[1]).abs().max(0.01)],
                feather,
            },
            _ => Shape::Radial {
                center: shifted(center),
                radius,
                feather,
            },
        },
    }
}

fn mask_overlay(ui: &mut egui::Ui, session: &mut EditSession, stage: Rect, image_rect: Rect, turns: usize, settings: &Settings) {
    let map = Mapping {
        rect: image_rect,
        turns,
        view: settings.crop(),
    };
    let painter = ui.painter_at(stage);
    let line = Stroke::new(1.5, WHITE);
    let diagonal = image_rect.size().length();
    if let Some(mask) = session.mask.and_then(|index| settings.masks.get(index)) {
        match mask.shape {
            Shape::Linear { from, to } => {
                let (a, b) = (map.to_screen(from), map.to_screen(to));
                let across = (b - a).normalized().rot90() * diagonal;
                painter.line_segment([a - across, a + across], line);
                painter.add(egui::Shape::dashed_line(&[b - across, b + across], line, 6.0, 5.0));
                painter.line_segment([a, b], Stroke::new(1.0, Color32::from_white_alpha(140)));
            }
            Shape::Radial { center, radius, feather } => {
                for (scale, stroke) in [(1.0, line), ((1.0 - feather).max(0.02), Stroke::new(1.0, Color32::from_white_alpha(140)))] {
                    let points: Vec<Pos2> = (0..72)
                        .map(|step| {
                            let angle = step as f32 / 72.0 * std::f32::consts::TAU;
                            map.to_screen([
                                center[0] + radius[0] * scale * angle.cos(),
                                center[1] + radius[1] * scale * angle.sin(),
                            ])
                        })
                        .collect();
                    painter.add(egui::Shape::closed_line(points, stroke));
                }
            }
        }
        for (_, at) in shape_handles(&mask.shape).into_iter().skip(1) {
            let rect = Rect::from_center_size(map.to_screen(at), Vec2::splat(8.0));
            painter.rect_filled(rect, 1, WHITE);
            painter.rect_stroke(rect, 1, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
        }
    }
    for (index, mask) in settings.masks.iter().enumerate() {
        let pin = map.to_screen(shape_handles(&mask.shape)[0].1);
        let selected = session.mask == Some(index);
        painter.circle_filled(pin, 7.0, if selected { ACCENT } else { Color32::from_gray(90) });
        painter.circle_stroke(pin, 7.0, Stroke::new(2.0, WHITE));
    }

    let response = ui.interact(stage, egui::Id::unique("edit-mask"), Sense::click_and_drag());
    let selected = session.mask;
    let hit = |pos: Pos2| -> Option<(usize, MaskHandle)> {
        if let Some(index) = selected.filter(|index| *index < settings.masks.len()) {
            let near = shape_handles(&settings.masks[index].shape)
                .into_iter()
                .map(|(handle, at)| (handle, map.to_screen(at).distance(pos)))
                .filter(|(_, distance)| *distance < 12.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((handle, _)) = near {
                return Some((index, handle));
            }
        }
        settings
            .masks
            .iter()
            .enumerate()
            .find(|(_, mask)| map.to_screen(shape_handles(&mask.shape)[0].1).distance(pos) < 12.0)
            .map(|(index, _)| (index, MaskHandle::Center))
    };
    if let Some(pos) = response.hover_pos() {
        if hit(pos).is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }
    if response.clicked() {
        if let Some((index, _)) = response.interact_pointer_pos().and_then(hit) {
            session.mask = Some(index);
        }
    }
    if response.drag_started() {
        session.drag = response.interact_pointer_pos().and_then(|pos| {
            let (index, handle) = hit(pos)?;
            session.mask = Some(index);
            Some(StageDrag::Mask {
                handle,
                start: settings.masks[index].shape,
                from: map.to_canvas(pos),
            })
        });
    }
    if response.dragged() {
        if let (Some(StageDrag::Mask { handle, start, from }), Some(pos), Some(index)) =
            (session.drag, response.interact_pointer_pos(), session.mask)
        {
            if index < settings.masks.len() {
                let mut next = settings.clone();
                next.masks[index].shape = move_handle(start, handle, from, map.to_canvas(pos));
                set_live(session, Kind::Mask, next);
            }
        }
    }
    if response.drag_stopped() {
        session.drag = None;
    }
}
