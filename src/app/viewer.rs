//! Single-photo stage, filmstrip, and slideshow.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::i18n::{tr, trf};
use crate::photo_info::Histogram;
use crate::types::{displayed_size, folder_label, format_bytes, format_duration, format_timestamp, Photo};
use crate::video::Player;

use super::theme::{self, ACCENT, VIEWER_BAR, VIEWER_BG, VIEWER_MUTED, VIEWER_PANEL, WHITE};
use super::{PicassoApp, Zoom};

/// The video on screen in the viewer, with its latest decoded frame.
pub(super) struct VideoView {
    pub id: i64,
    pub player: Player,
    texture: Option<egui::TextureHandle>,
    /// Position under the pointer while the seek bar is dragged.
    scrub: Option<f64>,
}

impl PicassoApp {
    pub(super) fn viewer_ui(&mut self, ui: &mut egui::Ui) {
        if self.photos.is_empty() {
            self.close_viewer();
            return;
        }
        *ui.visuals_mut() = theme::viewer_visuals();
        self.request_neighbors();
        let slideshow = self.slideshow.is_some();
        let photo = self.photos.get(self.viewer_index).cloned();
        let current = photo.clone().filter(|photo| photo.is_video() && !slideshow);
        self.sync_video(ui.ctx(), current.as_ref());
        let editable = photo.clone().filter(|photo| !photo.is_video() && !slideshow);
        self.sync_edit(editable.as_ref());
        self.viewer_bar(ui, slideshow);
        let film_h = 96.0;
        let controls_h = if self.video.is_some() { 44.0 } else { 0.0 };
        let editing = self.editing && !slideshow;
        let side_w = if editing {
            300.0
        } else if self.show_info && !slideshow {
            280.0
        } else {
            0.0
        };
        let stage_h = (ui.available_height() - film_h - controls_h).max(120.0);
        ui.horizontal(|ui| {
            let stage_w = (ui.available_width() - side_w).max(80.0);
            ui.vertical(|ui| {
                let (stage_rect, _) = ui.allocate_exact_size(Vec2::new(stage_w, stage_h), Sense::hover());
                self.paint_stage(ui, stage_rect, slideshow);
                if self.video.is_some() {
                    ui.allocate_ui(Vec2::new(stage_w, controls_h), |ui| self.video_controls(ui));
                }
            });
            if editing {
                self.edit_panel(ui, stage_h + controls_h, photo.as_ref());
            } else if side_w > 0.0 {
                self.info_panel(ui, stage_h + controls_h);
            }
        });
        self.filmstrip(ui, film_h);
    }

    /// Keeps a player for the video on screen and uploads its newest frame.
    fn sync_video(&mut self, ctx: &egui::Context, photo: Option<&Photo>) {
        let Some(photo) = photo else {
            self.video = None;
            return;
        };
        if self.video.as_ref().map(|view| view.id) != Some(photo.id) {
            if !self.audio_tried {
                self.audio_tried = true;
                self.audio_out = rodio::DeviceSinkBuilder::open_default_sink().ok().map(|mut sink| {
                    sink.log_on_drop(false);
                    sink
                });
            }
            self.video = Some(VideoView {
                id: photo.id,
                player: Player::open(std::path::Path::new(&photo.path)),
                texture: None,
                scrub: None,
            });
        }
        let mixer = self.audio_out.as_ref().map(|sink| sink.mixer());
        let Some(view) = self.video.as_mut() else { return };
        if let Some(frame) = view.player.tick(mixer) {
            let image = egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
            match view.texture.as_mut() {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    view.texture = Some(ctx.load_texture(format!("video-{}", photo.id), image, egui::TextureOptions::LINEAR));
                }
            }
        }
        if view.player.playing() || view.player.loading() {
            ctx.request_repaint();
        }
    }

    fn video_controls(&mut self, ui: &mut egui::Ui) {
        let Some(view) = self.video.as_mut() else { return };
        let bar = egui::Frame::new().fill(VIEWER_BAR).inner_margin(egui::Margin::symmetric(12, 6));
        bar.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let playing = view.player.playing();
                let icon = if playing { Control::Pause } else { Control::Play };
                if control_button(ui, icon).on_hover_text(tr("Play / Pause (Space)")).clicked() {
                    view.player.toggle();
                }
                let duration = view.player.duration();
                let shown = view.scrub.unwrap_or_else(|| view.player.position());
                let time = format!("{} / {}", format_duration(shown), format_duration(duration));
                ui.label(egui::RichText::new(time).color(WHITE).size(12.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if view.player.has_audio() {
                        ui.spacing_mut().slider_width = 80.0;
                        let mut volume = view.player.volume();
                        if ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false)).changed() {
                            view.player.set_volume(volume);
                        }
                        let icon = if view.player.muted() { Control::Muted } else { Control::Sound };
                        if control_button(ui, icon).on_hover_text(tr("Mute (M)")).clicked() {
                            view.player.toggle_mute();
                        }
                    }
                    let width = ui.available_width().max(60.0);
                    if let Some(to) = seek_bar(ui, width, shown, duration, &mut view.scrub) {
                        view.player.seek(to);
                    }
                });
            });
        });
    }

    fn viewer_bar(&mut self, ui: &mut egui::Ui, slideshow: bool) {
        let bar = egui::Frame::new().fill(VIEWER_BAR).inner_margin(egui::Margin::symmetric(12, 8));
        bar.show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(tr("‹  Library")).on_hover_text(tr("Return to organized thumbnails")).clicked() {
                    self.close_viewer();
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new(self.viewer_heading()).color(WHITE).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr(if slideshow { "Exit Slideshow" } else { "Slideshow" })).clicked() {
                        if slideshow {
                            self.slideshow = None;
                        } else {
                            self.start_slideshow();
                        }
                    }
                    if ui.button(tr("Info")).on_hover_text(tr("Details, histogram, and EXIF (I)")).clicked() {
                        self.toggle_info();
                    }
                    let video = self.photos.get(self.viewer_index).is_some_and(Photo::is_video);
                    if !slideshow && !video {
                        let edit = ui
                            .selectable_label(self.editing, tr("Edit"))
                            .on_hover_text(tr("Adjust light and color without changing the original (E)"));
                        if edit.clicked() {
                            self.toggle_editing();
                        }
                    }
                    if ui.button(tr("Rotate Right")).on_hover_text(tr("Rotate clockwise (Ctrl+])")).clicked() {
                        self.rotate_current(90);
                    }
                    if ui.button(tr("Rotate Left")).on_hover_text(tr("Rotate counterclockwise (Ctrl+[)")).clicked() {
                        self.rotate_current(-90);
                    }
                    if ui.button(tr("Open map")).on_hover_text(tr("Open this photo in OpenStreetMap")).clicked() {
                        if let Some(photo) = self.photos.get(self.viewer_index) {
                            let (lat, lon, zoom) = match photo.place() {
                                Some((lat, lon)) => (lat, lon, 14),
                                None => (self.map_lat, self.map_lon, self.map_zoom.max(2)),
                            };
                            super::places::open_osm(lat, lon, zoom);
                        }
                    }
                    if ui.button(tr("Fit")).clicked() {
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
                    if ui.button(star).on_hover_text(tr("Add/Remove Star")).clicked() {
                        if let Some(id) = self.photos.get(self.viewer_index).map(|p| p.id) {
                            self.toggle_star(id);
                        }
                    }
                    if let Some(photo) = self.photos.get(self.viewer_index) {
                        let (w, h) = displayed_size(photo);
                        let scale = self.scale_for(w, h, self.stage_rect.size());
                        let pct = scale * ui.ctx().pixels_per_point() * 100.0;
                        ui.label(egui::RichText::new(format!("{pct:.0}%")).color(VIEWER_MUTED));
                    }
                    if slideshow {
                        let playing = self.slideshow.as_ref().map(|s| s.playing).unwrap_or(false);
                        if ui.button(tr(if playing { "Pause" } else { "Play" })).clicked() {
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
        let video = self.video.as_ref().filter(|view| view.id == photo.id);
        let video_frame = video.and_then(|view| view.texture.clone());
        let edit_frame = self
            .edit
            .as_ref()
            .filter(|session| session.id == photo.id)
            .and_then(|session| session.texture().cloned());
        if let Some(texture) = video_frame {
            theme::paint_image(ui.painter(), &texture, image_rect, full_uv(), photo.rotation);
        } else if let Some(texture) = edit_frame {
            theme::paint_image(ui.painter(), &texture, image_rect, full_uv(), photo.rotation);
        } else if let Some((texture, _, _)) = self.display_texture(photo.id) {
            theme::paint_image(ui.painter(), &texture, image_rect, full_uv(), photo.rotation);
        } else if let Some((texture, _w, _h)) = self.thumb_texture(photo.id) {
            theme::paint_image(ui.painter(), &texture, image_rect, full_uv(), photo.rotation);
        } else {
            self.ensure_thumb(photo.id, &photo.path);
            ui.painter().text(
                stage.center(),
                Align2::CENTER_CENTER,
                tr("Loading…"),
                FontId::proportional(16.0),
                Color32::from_gray(180),
            );
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
        if let Some(view) = self.video.as_mut().filter(|view| view.id == photo.id) {
            if let Some(error) = view.player.error() {
                let painter = ui.painter();
                let galley = painter.layout_no_wrap(error.to_string(), FontId::proportional(14.0), WHITE);
                let pill = Rect::from_center_size(
                    Pos2::new(stage.center().x, stage.bottom() - 40.0),
                    galley.size() + Vec2::new(28.0, 16.0),
                );
                painter.rect_filled(pill, 8, Color32::from_black_alpha(180));
                painter.galley(pill.center() - galley.size() / 2.0, galley, WHITE);
            } else if !view.player.playing() {
                paint_play_button(ui.painter(), stage.center(), 36.0);
            }
            if response.clicked() && !response.dragged() {
                view.player.toggle();
            }
        } else if response.double_clicked() {
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
            let left = Rect::from_center_size(Pos2::new(stage.left() + 36.0, mid_y), Vec2::splat(44.0));
            let right = Rect::from_center_size(Pos2::new(stage.right() - 36.0, mid_y), Vec2::splat(44.0));
            if self.viewer_index > 0 && chevron(ui, left, "‹") {
                self.viewer_step(-1);
            }
            if self.viewer_index + 1 < self.photos.len() && chevron(ui, right, "›") {
                self.viewer_step(1);
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
        let frame = egui::Frame::new().fill(VIEWER_PANEL).inner_margin(egui::Margin::same(12));
        frame.show(ui, |ui| {
            ui.set_min_size(Vec2::new(256.0, height - 24.0));
            ui.set_max_width(256.0);
            egui::ScrollArea::vertical()
                .id_salt("info-panel")
                .auto_shrink([false, false])
                .max_height(height - 24.0)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| self.info_contents(ui));
                });
        });
    }

    fn info_contents(&mut self, ui: &mut egui::Ui) {
        let Some(photo) = self.photos.get(self.viewer_index).cloned() else {
            return;
        };
        let info = self.display_info(photo.id);
        {
            ui.label(egui::RichText::new(tr("Histogram")).strong());
            ui.add_space(4.0);
            match &info {
                Some((histogram, _)) => paint_histogram(ui, histogram),
                None => {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HISTOGRAM_H), Sense::hover());
                    ui.painter().rect_filled(rect, 2, Color32::from_gray(0x2A));
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        tr("Loading…"),
                        FontId::proportional(12.0),
                        Color32::from_gray(160),
                    );
                }
            }
            ui.add_space(10.0);
            ui.label(egui::RichText::new(tr("Details")).strong());
            ui.add_space(6.0);
            detail(ui, "File", &photo.filename);
            detail(ui, "Taken", &format_timestamp(photo.taken_at));
            if photo.modified_at != photo.taken_at {
                detail(ui, "Modified", &format_timestamp(photo.modified_at));
            }
            let (w, h) = displayed_size(&photo);
            detail(ui, "Size", &format!("{} × {}   {}", w as u32, h as u32, format_bytes(photo.bytes)));
            if !photo.camera.is_empty() {
                detail(ui, "Camera", &photo.camera);
            }
            if photo.rotation != 0 {
                detail(ui, "Rotated", &trf("{}° clockwise in Picasso", &[&photo.rotation]));
            }
            if let Some(folder) = self.folders.iter().find(|folder| folder.id == photo.folder_id) {
                detail(ui, "Folder", &folder_label(&folder.path));
            }
            if let Some((lat, lon)) = photo.place() {
                let kind = tr(if photo.pinned() { "pinned" } else { "GPS" });
                detail(ui, "Place", &format!("{lat:.5}, {lon:.5} ({kind})"));
            }
            ui.add_space(10.0);
            ui.label(egui::RichText::new(tr(if photo.is_video() { "Video" } else { "EXIF" })).strong());
            ui.add_space(4.0);
            match &info {
                None => {
                    ui.label(egui::RichText::new(tr("Loading…")).color(VIEWER_MUTED).size(12.0));
                }
                Some((_, rows)) => {
                    // Camera and date already appear under Details.
                    let rows: Vec<&(&'static str, String)> = rows
                        .iter()
                        .filter(|(label, _)| *label != "Taken" && !(*label == "Camera" && !photo.camera.is_empty()))
                        .collect();
                    if rows.is_empty() {
                        ui.label(egui::RichText::new(tr("No EXIF data in this file")).color(VIEWER_MUTED).size(12.0));
                    } else {
                        egui::Grid::new(("exif", photo.id))
                            .num_columns(2)
                            .striped(true)
                            .spacing(Vec2::new(10.0, 3.0))
                            .show(ui, |ui| {
                                for (label, value) in rows {
                                    ui.label(egui::RichText::new(tr(label)).color(VIEWER_MUTED).size(12.0));
                                    ui.add(egui::Label::new(egui::RichText::new(value).size(12.0)).wrap());
                                    ui.end_row();
                                }
                            });
                    }
                }
            }
            ui.add_space(10.0);
            ui.label(tr("Caption"));
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
            if ui.button(tr("Show in folder")).clicked() {
                if let Some(parent) = std::path::Path::new(&photo.path).parent() {
                    let _ = open::that(parent);
                }
            }
            if ui.button(tr("Show on map")).clicked() {
                self.show_on_map(photo.id);
            }
            if ui.button(tr("Open map")).clicked() {
                let (lat, lon, zoom) = photo
                    .place()
                    .map(|(lat, lon)| (lat, lon, 14))
                    .unwrap_or((self.map_lat, self.map_lon, self.map_zoom.max(2)));
                super::places::open_osm(lat, lon, zoom);
            }
            if ui.button(tr("Move to trash")).clicked() {
                self.modal = super::Modal::Trash { ids: vec![photo.id] };
            }
        }
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
                        let is_video = self.photos[index].is_video();
                        self.ensure_thumb(id, &path);
                        let response = ui.interact(rect, egui::Id::unique(("film", id)), Sense::click());
                        if let Some((texture, w, h)) = self.thumb_texture(id) {
                            theme::paint_image_rounded(ui.painter(), &texture, rect, theme::cover_uv(w, h), rotation, 5.0);
                        } else {
                            ui.painter().rect_filled(rect, 5, Color32::from_gray(40));
                        }
                        if is_video {
                            paint_play_button(ui.painter(), rect.center(), 11.0);
                        }
                        if index != current && !response.hovered() {
                            ui.painter().rect_filled(rect, 5, Color32::from_black_alpha(110));
                        }
                        if index == current {
                            ui.painter().rect_stroke(rect, 6, egui::Stroke::new(2.0, ACCENT), egui::StrokeKind::Outside);
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
        format!("{name}    {}", trf("{} of {}", &[&(self.viewer_index + 1), &self.photos.len()]))
    }
}

fn chevron(ui: &mut egui::Ui, rect: Rect, glyph: &str) -> bool {
    let response = ui
        .interact(rect, egui::Id::unique(("chev", glyph, rect.left() as i32)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let alpha = if response.hovered() { 170 } else { 90 };
    ui.painter().circle_filled(rect.center(), 20.0, Color32::from_black_alpha(alpha));
    ui.painter().text(
        rect.center() + Vec2::new(0.0, -2.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(26.0),
        WHITE,
    );
    response.clicked()
}

#[derive(Clone, Copy)]
enum Control {
    Play,
    Pause,
    Sound,
    Muted,
}

fn control_button(ui: &mut egui::Ui, control: Control) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(32.0, 28.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS, Color32::from_white_alpha(24));
    }
    let c = rect.center();
    let stroke = egui::Stroke::new(1.8, WHITE);
    match control {
        Control::Play => paint_triangle(painter, c, 7.0, WHITE),
        Control::Pause => {
            for dx in [-3.5, 3.5] {
                painter.rect_filled(Rect::from_center_size(c + Vec2::new(dx, 0.0), Vec2::new(3.5, 13.0)), 1, WHITE);
            }
        }
        Control::Sound | Control::Muted => {
            let speaker = c + Vec2::new(-4.0, 0.0);
            painter.rect_filled(Rect::from_center_size(speaker + Vec2::new(-3.0, 0.0), Vec2::new(4.0, 6.0)), 1, WHITE);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    speaker + Vec2::new(-1.0, -3.0),
                    speaker + Vec2::new(4.0, -7.0),
                    speaker + Vec2::new(4.0, 7.0),
                    speaker + Vec2::new(-1.0, 3.0),
                ],
                WHITE,
                egui::Stroke::NONE,
            ));
            if matches!(control, Control::Muted) {
                let x = c + Vec2::new(6.0, 0.0);
                painter.line_segment([x + Vec2::new(-3.0, -3.0), x + Vec2::new(3.0, 3.0)], stroke);
                painter.line_segment([x + Vec2::new(-3.0, 3.0), x + Vec2::new(3.0, -3.0)], stroke);
            } else {
                for radius in [4.0_f32, 8.0] {
                    let arc: Vec<Pos2> = (-3..=3)
                        .map(|step| {
                            let angle = step as f32 * 0.22;
                            speaker + Vec2::new(4.0 + radius * angle.cos(), radius * angle.sin())
                        })
                        .collect();
                    painter.add(egui::Shape::line(arc, egui::Stroke::new(1.5, WHITE)));
                }
            }
        }
    }
    response
}

/// Play triangle pointing right, centered on `center`.
fn paint_triangle(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            center + Vec2::new(-0.7 * size, -size),
            center + Vec2::new(1.1 * size, 0.0),
            center + Vec2::new(-0.7 * size, size),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

pub(super) fn paint_play_button(painter: &egui::Painter, center: Pos2, radius: f32) {
    painter.circle_filled(center, radius, Color32::from_black_alpha(140));
    painter.circle_stroke(center, radius, egui::Stroke::new(2.0, Color32::from_white_alpha(200)));
    paint_triangle(painter, center + Vec2::new(radius * 0.08, 0.0), radius * 0.38, WHITE);
}

/// Time line of the video. Returns where to seek once the user clicks or
/// lets go of the knob.
fn seek_bar(ui: &mut egui::Ui, width: f32, position: f64, duration: f64, scrub: &mut Option<f64>) -> Option<f64> {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let track = Rect::from_center_size(rect.center(), Vec2::new((rect.width() - 14.0).max(10.0), 4.0));
    let at = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64 * duration;
    let pointer = response.interact_pointer_pos();
    if response.dragged() {
        if let Some(pos) = pointer {
            *scrub = Some(at(pos.x));
        }
    }
    let mut seek = None;
    if response.drag_stopped() {
        seek = scrub.take();
    } else if response.clicked() {
        seek = pointer.map(|pos| at(pos.x));
    }
    let shown = scrub.unwrap_or(position);
    let fraction = if duration > 0.0 { (shown / duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let painter = ui.painter();
    painter.rect_filled(track, 2, Color32::from_white_alpha(50));
    let played = Rect::from_min_max(track.min, Pos2::new(track.left() + track.width() * fraction, track.bottom()));
    painter.rect_filled(played, 2, ACCENT);
    let knob = Pos2::new(played.right(), track.center().y);
    let grow = response.hovered() || scrub.is_some();
    painter.circle_filled(knob, if grow { 7.0 } else { 5.0 }, WHITE);
    if let Some(hover) = response.hover_pos() {
        response.on_hover_text_at_pointer(format_duration(at(hover.x)));
    }
    seek
}

const HISTOGRAM_H: f32 = 96.0;

/// Draws luminance as a filled gray area with the red, green, and blue
/// channels as lines on top, plus the share of clipped shadows and highlights.
fn paint_histogram(ui: &mut egui::Ui, histogram: &Histogram) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HISTOGRAM_H), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2, Color32::from_gray(0x2A));
    let plot = rect.shrink2(Vec2::new(3.0, 4.0));
    let peak = histogram.peak() as f32;
    let x_at = |bin: usize| plot.left() + plot.width() * bin as f32 / 255.0;
    let y_at = |count: u32| plot.bottom() - plot.height() * (count as f32 / peak).min(1.0);

    let mut mesh = egui::Mesh::default();
    let fill = Color32::from_gray(0x9A);
    for (bin, &count) in histogram.bins[Histogram::LUMA].iter().enumerate() {
        let index = mesh.vertices.len() as u32;
        mesh.colored_vertex(Pos2::new(x_at(bin), plot.bottom()), fill);
        mesh.colored_vertex(Pos2::new(x_at(bin), y_at(count)), fill);
        if bin > 0 {
            mesh.add_triangle(index - 2, index - 1, index);
            mesh.add_triangle(index - 1, index, index + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));

    let channels = [
        (Histogram::RED, Color32::from_rgba_unmultiplied(0xFF, 0x5A, 0x50, 0xE0)),
        (Histogram::GREEN, Color32::from_rgba_unmultiplied(0x5C, 0xD6, 0x5C, 0xE0)),
        (Histogram::BLUE, Color32::from_rgba_unmultiplied(0x5A, 0x9C, 0xFF, 0xE0)),
    ];
    for (channel, color) in channels {
        let points: Vec<Pos2> = histogram.bins[channel]
            .iter()
            .enumerate()
            .map(|(bin, &count)| Pos2::new(x_at(bin), y_at(count)))
            .collect();
        painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
    }
    for quarter in 1..4 {
        let x = plot.left() + plot.width() * quarter as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            egui::Stroke::new(1.0, Color32::from_white_alpha(18)),
        );
    }

    let (shadows, highlights) = histogram.clipping();
    let summary = trf(
        "Shadows clipped {}%   Highlights clipped {}%",
        &[&format!("{:.1}", shadows * 100.0), &format!("{:.1}", highlights * 100.0)],
    );
    ui.label(egui::RichText::new(summary).color(VIEWER_MUTED).size(11.0));
    if let Some(pos) = response.hover_pos() {
        let bin = (((pos.x - plot.left()) / plot.width()) * 255.0).round().clamp(0.0, 255.0) as usize;
        let total = histogram.total().max(1) as f32;
        let share = |channel: usize| histogram.bins[channel][bin] as f32 / total * 100.0;
        let pct = |channel: usize| format!("{:.2}", share(channel));
        response.on_hover_text(trf(
            "Level {}\nLuminance {}%\nRed {}%  Green {}%  Blue {}%",
            &[
                &bin,
                &pct(Histogram::LUMA),
                &pct(Histogram::RED),
                &pct(Histogram::GREEN),
                &pct(Histogram::BLUE),
            ],
        ));
    }
}

fn full_uv() -> Rect {
    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

fn detail(ui: &mut egui::Ui, label: &'static str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tr(label)).color(VIEWER_MUTED).size(12.0));
        ui.label(egui::RichText::new(value).size(12.0));
    });
}
