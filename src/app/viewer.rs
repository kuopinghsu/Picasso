//! Single-photo stage, filmstrip, slideshow, and face naming targets.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::types::{displayed_size, folder_label, format_bytes, format_timestamp, RelRect};

use super::theme::{self, FACE, MUTED, TEXT, TOOLBAR, VIEWER_BAR, VIEWER_BG, WHITE};
use super::{PicassoApp, Zoom};

impl PicassoApp {
    pub(super) fn viewer_ui(&mut self, ui: &mut egui::Ui) {
        if self.photos.is_empty() {
            self.close_viewer();
            return;
        }
        self.request_neighbors();
        let slideshow = self.slideshow.is_some();
        self.viewer_bar(ui, slideshow);
        let film_h = 96.0;
        let info_w = if self.show_info && !slideshow { 280.0 } else { 0.0 };
        let stage_h = (ui.available_height() - film_h).max(120.0);
        ui.horizontal(|ui| {
            let stage_w = (ui.available_width() - info_w).max(80.0);
            let (stage_rect, _) = ui.allocate_exact_size(Vec2::new(stage_w, stage_h), Sense::hover());
            self.paint_stage(ui, stage_rect, slideshow);
            if info_w > 0.0 {
                self.info_panel(ui, stage_h);
            }
        });
        self.filmstrip(ui, film_h);
    }

    fn viewer_bar(&mut self, ui: &mut egui::Ui, slideshow: bool) {
        let bar = egui::Frame::new().fill(TOOLBAR).inner_margin(egui::Margin::symmetric(8, 4));
        bar.show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Back to Library").on_hover_text("Return to organized thumbnails").clicked() {
                    self.close_viewer();
                }
                ui.label(egui::RichText::new(self.viewer_heading()).color(TEXT).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(if slideshow { "Exit Slideshow" } else { "Slideshow" }).clicked() {
                        if slideshow {
                            self.slideshow = None;
                        } else {
                            self.start_slideshow();
                        }
                    }
                    if ui.button(if self.show_info { "Metadata" } else { "Metadata" }).clicked() {
                        self.show_info = !self.show_info;
                    }
                    if ui.button(if self.show_faces { "Faces" } else { "Faces" }).clicked() {
                        self.show_faces = !self.show_faces;
                    }
                    if ui.button("Rotate Clockwise").clicked() {
                        self.rotate_current(90);
                    }
                    if ui.button("Rotate Left").clicked() {
                        self.rotate_current(-90);
                    }
                    if ui.button("Fit").clicked() {
                        self.zoom = Zoom::Fit;
                        self.pan = Vec2::ZERO;
                    }
                    if ui.button("100%").clicked() {
                        self.zoom = Zoom::Scale(1.0 / ui.ctx().pixels_per_point());
                        self.pan = Vec2::ZERO;
                    }
                    if ui.button("−").clicked() {
                        self.zoom_keys(1.0 / 1.25);
                    }
                    if ui.button("+").clicked() {
                        self.zoom_keys(1.25);
                    }
                    let starred = self.photos.get(self.viewer_index).map(|p| p.starred).unwrap_or(false);
                    let star = if starred { "★" } else { "☆" };
                    if ui.button(star).on_hover_text("Add/Remove Star").clicked() {
                        if let Some(id) = self.photos.get(self.viewer_index).map(|p| p.id) {
                            self.toggle_star(id);
                        }
                    }
                    if let Some(photo) = self.photos.get(self.viewer_index) {
                        let (w, h) = displayed_size(photo);
                        let scale = self.scale_for(w, h, self.stage_rect.size());
                        let pct = scale * ui.ctx().pixels_per_point() * 100.0;
                        ui.label(egui::RichText::new(format!("{pct:.0}%")).color(MUTED));
                    }
                    if slideshow {
                        let playing = self.slideshow.as_ref().map(|s| s.playing).unwrap_or(false);
                        if ui.button(if playing { "Pause" } else { "Play" }).clicked() {
                            if let Some(show) = self.slideshow.as_mut() {
                                show.playing = !show.playing;
                                show.last = std::time::Instant::now();
                            }
                        }
                        if let Some(show) = self.slideshow.as_mut() {
                            ui.add(egui::Slider::new(&mut show.interval, 1.0..=15.0).suffix(" s").fixed_decimals(0));
                        }
                    }
                });
            });
        });
    }

    fn paint_stage(&mut self, ui: &mut egui::Ui, stage: Rect, slideshow: bool) {
        self.stage_rect = stage;
        ui.painter().rect_filled(stage, 0, VIEWER_BG);
        let Some(photo) = self.photos.get(self.viewer_index).cloned() else {
            return;
        };
        let (img_w, img_h) = displayed_size(&photo);
        let scale = self.scale_for(img_w, img_h, stage.size());
        let size = Vec2::new(img_w * scale, img_h * scale);
        let image_rect = Rect::from_center_size(stage.center() + self.pan, size);
        let mut response = ui.interact(stage, egui::Id::unique("stage"), Sense::click_and_drag());
        if let Some((texture, _, _)) = self.display_texture(photo.id, photo.rotation) {
            theme::paint_image(ui.painter(), &texture, image_rect, full_uv());
        } else if let Some((texture, _w, _h)) = self.thumb_texture(photo.id) {
            theme::paint_image(ui.painter(), &texture, image_rect, full_uv());
        } else {
            self.ensure_thumb(photo.id, &photo.path, photo.rotation);
            ui.painter().text(
                stage.center(),
                Align2::CENTER_CENTER,
                "Loading…",
                FontId::proportional(16.0),
                Color32::from_gray(180),
            );
        }

        if self.show_faces {
            self.paint_faces(ui, image_rect);
        }

        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                let factor = (scroll * 0.002).exp();
                let pointer = response.hover_pos().unwrap_or(stage.center());
                self.zoom_at(factor, pointer, stage, img_w, img_h);
            }
        }
        if response.dragged() && !matches!(self.zoom, Zoom::Fit) {
            self.pan += response.drag_delta();
            self.clamp_pan(img_w, img_h, scale, stage);
        }
        if response.double_clicked() {
            if matches!(self.zoom, Zoom::Fit) {
                self.zoom = Zoom::Scale(1.0 / ui.ctx().pixels_per_point());
            } else {
                self.zoom = Zoom::Fit;
                self.pan = Vec2::ZERO;
            }
        } else if response.clicked() && slideshow {
            if let Some(pos) = response.interact_pointer_pos() {
                self.slideshow_click(pos, stage);
            }
        }
        if !matches!(self.zoom, Zoom::Fit) {
            response = response.on_hover_cursor(egui::CursorIcon::Grab);
        }

        if response.hovered() || slideshow {
            let mid_y = stage.center().y;
            let left = Rect::from_center_size(Pos2::new(stage.left() + 28.0, mid_y), Vec2::new(36.0, 56.0));
            let right = Rect::from_center_size(Pos2::new(stage.right() - 28.0, mid_y), Vec2::new(36.0, 56.0));
            if self.viewer_index > 0 && chevron(ui, left, "‹") {
                self.viewer_step(-1);
            }
            if self.viewer_index + 1 < self.photos.len() && chevron(ui, right, "›") {
                self.viewer_step(1);
            }
        }
    }

    fn paint_faces(&mut self, ui: &mut egui::Ui, image_rect: Rect) {
        let faces = self.viewer_faces.clone();
        for face in &faces {
            let rect = map_face(face.x, face.y, face.w, face.h, image_rect);
            let response = ui.interact(rect, egui::Id::unique(("face", face.id)), Sense::click());
            ui.painter().rect_stroke(rect, 0, egui::Stroke::new(2.0, FACE), egui::StrokeKind::Outside);
            let label = face.person_name.clone().unwrap_or_else(|| "Add a name".into());
            let text_pos = Pos2::new(rect.left(), rect.bottom() + 2.0);
            let galley = ui.painter().layout_no_wrap(label, FontId::proportional(12.0), Color32::BLACK);
            let bg = Rect::from_min_size(text_pos, galley.size() + Vec2::new(8.0, 3.0));
            ui.painter().rect_filled(bg, 0, FACE);
            ui.painter().galley(text_pos + Vec2::new(4.0, 1.0), galley, Color32::BLACK);
            if response.clicked() {
                let pos = rect.left_bottom() + Vec2::new(0.0, 6.0);
                self.begin_naming(face, pos);
            }
        }
    }

    fn slideshow_click(&mut self, pos: Pos2, stage: Rect) {
        let rel = (pos.x - stage.left()) / stage.width().max(1.0);
        if rel < 0.33 {
            self.viewer_step(-1);
        } else if rel > 0.67 {
            self.viewer_step(1);
        } else if let Some(show) = self.slideshow.as_mut() {
            show.playing = !show.playing;
            show.last = std::time::Instant::now();
        }
    }

    fn info_panel(&mut self, ui: &mut egui::Ui, height: f32) {
        let frame = egui::Frame::new()
            .fill(Color32::from_rgb(0xF8, 0xF9, 0xFA))
            .inner_margin(egui::Margin::same(12));
        frame.show(ui, |ui| {
            ui.set_min_size(Vec2::new(268.0, height));
            ui.set_max_width(268.0);
            let Some(photo) = self.photos.get(self.viewer_index).cloned() else {
                return;
            };
            ui.label(egui::RichText::new("Details").strong());
            ui.add_space(6.0);
            detail(ui, "File", &photo.filename);
            detail(ui, "Taken", &format_timestamp(photo.taken_at));
            let (w, h) = displayed_size(&photo);
            detail(ui, "Size", &format!("{} × {}   {}", w as u32, h as u32, format_bytes(photo.bytes)));
            if !photo.camera.is_empty() {
                detail(ui, "Camera", &photo.camera);
            }
            detail(ui, "Faces", &photo.face_count.to_string());
            if let Some(folder) = self.folders.iter().find(|folder| folder.id == photo.folder_id) {
                detail(ui, "Folder", &folder_label(&folder.path));
            }
            ui.add_space(8.0);
            ui.label("Caption");
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.caption_buf)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if response.changed() {
                self.db.set_caption(photo.id, &self.caption_buf);
                if let Some(slot) = self.photos.iter_mut().find(|item| item.id == photo.id) {
                    slot.caption = self.caption_buf.clone();
                }
            }
            ui.add_space(8.0);
            if ui.button("Show in folder").clicked() {
                if let Some(parent) = std::path::Path::new(&photo.path).parent() {
                    let _ = open::that(parent);
                }
            }
            if ui.button("Move to trash").clicked() {
                self.modal = super::Modal::Trash { ids: vec![photo.id] };
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new("People").strong());
            for face in self.viewer_faces.clone() {
                if face.photo_id != photo.id {
                    continue;
                }
                let mut name = face.person_name.clone().unwrap_or_else(|| "Add a name".into());
                if face.manual {
                    name.push_str(" · manual");
                } else if face.confirmed {
                    name.push_str(" · confirmed");
                }
                let button = ui.button(name).on_hover_text(format!("score {:.1}", face.score));
                if button.clicked() {
                    self.begin_naming(&face, ui.cursor().min);
                }
            }
        });
    }

    fn filmstrip(&mut self, ui: &mut egui::Ui, height: f32) {
        let bg = egui::Frame::new().fill(VIEWER_BAR).inner_margin(egui::Margin::same(8));
        bg.show(ui, |ui| {
            let cell = 72.0;
            let gap = 6.0;
            let count = self.photos.len();
            let current = self.viewer_index;
            egui::ScrollArea::horizontal()
                .auto_shrink([false, false])
                .show_viewport(ui, |ui, viewport| {
                    ui.set_width((cell + gap) * count as f32 + 8.0);
                    ui.set_min_height(height - 16.0);
                    let step = cell + gap;
                    let first = (viewport.min.x / step).floor().max(0.0) as usize;
                    let last = ((viewport.max.x / step).ceil() as usize + 1).min(count);
                    let origin = ui.min_rect().min;
                    for index in first..last {
                        let rect = Rect::from_min_size(
                            origin + Vec2::new(index as f32 * step, 0.0),
                            Vec2::new(cell, cell * 0.75),
                        );
                        let id = self.photos[index].id;
                        let path = self.photos[index].path.clone();
                        let rotation = self.photos[index].rotation;
                        self.ensure_thumb(id, &path, rotation);
                        let response = ui.interact(rect, egui::Id::unique(("film", id)), Sense::click());
                        if let Some((texture, w, h)) = self.thumb_texture(id) {
                            theme::paint_image(ui.painter(), &texture, rect, theme::cover_uv(w, h));
                        } else {
                            ui.painter().rect_filled(rect, 2, Color32::from_gray(40));
                        }
                        if index == current {
                            ui.painter().rect_stroke(rect, 0, egui::Stroke::new(2.0, WHITE), egui::StrokeKind::Outside);
                            if response.rect.width() > 0.0 {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                        }
                        if response.clicked() {
                            self.set_index(index, true);
                        }
                    }
                });
        });
    }

    pub(super) fn scale_for(&self, img_w: f32, img_h: f32, viewport: Vec2) -> f32 {
        let fit = if img_w <= 1.0 || img_h <= 1.0 || viewport.x <= 1.0 || viewport.y <= 1.0 {
            1.0
        } else {
            (viewport.x / img_w).min(viewport.y / img_h)
        };
        match self.zoom {
            Zoom::Fit => fit,
            Zoom::Scale(scale) => scale,
        }
    }

    pub(super) fn zoom_at(&mut self, factor: f32, pointer: Pos2, stage: Rect, img_w: f32, img_h: f32) {
        let old = self.scale_for(img_w, img_h, stage.size()).max(0.0001);
        let new = (old * factor).clamp(0.02, 12.0);
        let rel = (pointer - stage.center()) - self.pan;
        self.pan = (pointer - stage.center()) - rel * (new / old);
        self.zoom = Zoom::Scale(new);
        self.clamp_pan(img_w, img_h, new, stage);
    }

    pub(super) fn zoom_keys(&mut self, factor: f32) {
        let Some(photo) = self.photos.get(self.viewer_index).cloned() else {
            return;
        };
        let (w, h) = displayed_size(&photo);
        let stage = if self.stage_rect.width() > 2.0 {
            self.stage_rect
        } else {
            Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))
        };
        self.zoom_at(factor, stage.center(), stage, w, h);
    }

    fn clamp_pan(&mut self, img_w: f32, img_h: f32, scale: f32, stage: Rect) {
        let size = Vec2::new(img_w * scale, img_h * scale);
        let limit_x = ((size.x - stage.width()).max(0.0) * 0.5) + stage.width() * 0.35;
        let limit_y = ((size.y - stage.height()).max(0.0) * 0.5) + stage.height() * 0.35;
        self.pan.x = self.pan.x.clamp(-limit_x, limit_x);
        self.pan.y = self.pan.y.clamp(-limit_y, limit_y);
    }

    fn viewer_heading(&self) -> String {
        let Some(photo) = self.photos.get(self.viewer_index) else {
            return String::new();
        };
        let name = if photo.caption.is_empty() {
            photo.filename.clone()
        } else {
            photo.caption.clone()
        };
        format!("{}    {} of {}", name, self.viewer_index + 1, self.photos.len())
    }
}

fn chevron(ui: &mut egui::Ui, rect: Rect, glyph: &str) -> bool {
    let response = ui.interact(rect, egui::Id::unique(("chev", glyph, rect.left() as i32)), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 8, Color32::from_black_alpha(140));
    } else {
        ui.painter().rect_filled(rect, 8, Color32::from_black_alpha(70));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(28.0), WHITE);
    response.clicked()
}

fn map_face(x: f32, y: f32, w: f32, h: f32, image: Rect) -> Rect {
    Rect::from_min_size(
        Pos2::new(image.left() + x * image.width(), image.top() + y * image.height()),
        Vec2::new(w * image.width(), h * image.height()),
    )
}

fn full_uv() -> Rect {
    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(MUTED).size(12.0));
        ui.label(egui::RichText::new(value).size(12.0));
    });
}

#[allow(dead_code)]
fn _rect_ty(rect: RelRect) -> RelRect {
    rect
}
