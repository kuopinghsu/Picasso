//! Right-hand panel of the library, opened from the Info and Map buttons in
//! the photo tray as in Picasa 3: properties and EXIF of the selected photo,
//! or a map of where it was taken.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::geo;
use crate::i18n::{tr, trf};
use crate::types::{displayed_size, folder_label, format_bytes, format_timestamp, Nav, Photo};

use super::places::{MapView, MAX_ZOOM, MIN_ZOOM};
use super::theme::{self, ACCENT, WHITE};
use super::PicassoApp;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum SidePanel {
    None,
    Info,
    Map,
}

impl SidePanel {
    pub(super) fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("info") => Self::Info,
            Some("map") => Self::Map,
            _ => Self::None,
        }
    }

    fn setting(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Info => "info",
            Self::Map => "map",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum TrayIcon {
    Info,
    Map,
}


impl PicassoApp {
    /// Opens `panel`, or closes it when it is already showing.
    pub(super) fn toggle_side(&mut self, panel: SidePanel) {
        self.side = if self.side == panel { SidePanel::None } else { panel };
        self.db.set_setting("side_panel", self.side.setting());
    }

    /// The photo the panel describes: the focused one of the selection.
    pub(super) fn side_photo(&self) -> Option<Photo> {
        self.primary_index()
            .and_then(|index| self.photos.get(index).cloned())
            .or_else(|| {
                let id = self.anchor.filter(|id| self.selection.contains(id))?;
                self.db.photo(id)
            })
    }

    pub(super) fn side_ui(&mut self, ui: &mut egui::Ui) {
        if self.nav == Nav::Places {
            self.places_side_ui(ui);
            return;
        }
        ui.horizontal(|ui| {
            for (panel, label) in [(SidePanel::Info, "Info"), (SidePanel::Map, "Map")] {
                let on = self.side == panel;
                let text = egui::RichText::new(tr(label)).color(if on { ACCENT } else { theme::text() });
                let button = egui::Button::new(text)
                    .fill(if on { theme::select() } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE);
                if ui.add(button).clicked() && !on {
                    self.toggle_side(panel);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = egui::Button::new(egui::RichText::new("×").size(18.0).color(theme::muted())).frame(false);
                if ui.add(close).on_hover_text(tr("Close panel")).clicked() {
                    self.toggle_side(self.side);
                }
            });
        });
        ui.add_space(8.0);

        let Some(photo) = self.side_photo() else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(tr("Select a photo to see its details.")).color(theme::muted()));
            });
            return;
        };
        if self.selection.len() > 1 {
            let text = trf("{} photos selected · showing one", &[&self.selection.len()]);
            ui.label(egui::RichText::new(text).size(12.0).color(theme::muted()));
            ui.add_space(4.0);
        }
        egui::ScrollArea::vertical()
            .id_salt("side-panel")
            .auto_shrink([false, false])
            .show(ui, |ui| match self.side {
                SidePanel::Map => self.side_map(ui, &photo),
                _ => self.side_info(ui, &photo),
            });
    }

    fn side_preview(&mut self, ui: &mut egui::Ui, photo: &Photo) {
        self.ensure_thumb(photo.id, &photo.path);
        let width = ui.available_width();
        let (frame, _) = ui.allocate_exact_size(Vec2::new(width, (width * 0.7).min(240.0)), Sense::hover());
        ui.painter().rect_filled(frame, theme::THUMB_RADIUS, theme::placeholder());
        if let Some((texture, _, _)) = self.thumb_texture(photo.id) {
            let (w, h) = self.shown_size(photo);
            let scale = (frame.width() / w).min(frame.height() / h);
            let rect = Rect::from_center_size(frame.center(), Vec2::new(w * scale, h * scale));
            let full = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            theme::paint_image_rounded(ui.painter(), &texture, rect, full, photo.rotation, 6.0);
        }
    }

    pub(super) fn side_info(&mut self, ui: &mut egui::Ui, photo: &Photo) {
        self.side_preview(ui, photo);
        ui.add_space(10.0);
        ui.add(egui::Label::new(egui::RichText::new(&photo.filename).size(15.0).strong().color(theme::text())).wrap());
        if let Some(parent) = std::path::Path::new(&photo.path).parent() {
            let folder = parent.to_string_lossy();
            ui.add(egui::Label::new(egui::RichText::new(folder.trim_start_matches(r"\\?\")).size(11.5).color(theme::muted())).wrap());
        }
        if !photo.caption.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(egui::RichText::new(&photo.caption).italics()).wrap());
        }

        section(ui, tr("Properties"));
        let (w, h) = displayed_size(photo);
        let mut rows = vec![
            ("Taken", format_timestamp(photo.taken_at)),
            ("Dimensions", format!("{} × {}", w as u32, h as u32)),
            ("File size", format_bytes(photo.bytes)),
        ];
        if photo.modified_at != photo.taken_at {
            rows.insert(1, ("Modified", format_timestamp(photo.modified_at)));
        }
        if photo.is_video() && photo.duration > 0.0 {
            rows.insert(1, ("Duration", crate::types::format_duration(photo.duration)));
        }
        if !photo.camera.is_empty() {
            rows.push(("Camera", photo.camera.clone()));
        }
        if photo.rotation != 0 {
            rows.push(("Rotated", trf("{}° clockwise", &[&photo.rotation])));
        }
        if let Some(folder) = self.folders.iter().find(|folder| folder.id == photo.folder_id) {
            rows.push(("Library folder", folder_label(&folder.path)));
        }
        if let Some((lat, lon)) = photo.place() {
            let kind = if photo.pinned() { tr("pinned") } else { "GPS" };
            rows.push(("Location", format!("{lat:.5}, {lon:.5} ({kind})")));
        }
        property_grid(ui, ("props", photo.id), &rows);

        section(ui, tr(if photo.is_video() { "Video" } else { "Camera EXIF" }));
        match self.exif_for(photo) {
            None => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(egui::RichText::new(tr("Reading EXIF…")).size(12.0).color(theme::muted()));
                });
            }
            Some(exif) => {
                let rows: Vec<(&'static str, String)> = exif
                    .into_iter()
                    .filter(|(label, _)| *label != "Taken" && !(*label == "Camera" && !photo.camera.is_empty()))
                    .collect();
                if rows.is_empty() {
                    ui.label(egui::RichText::new(tr("No EXIF data in this file")).size(12.0).color(theme::muted()));
                } else {
                    property_grid(ui, ("exif", photo.id), &rows);
                }
            }
        }

        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("Show in folder")).clicked() {
                crate::shell::reveal(&photo.path);
            }
            if photo.place().is_some() && self.nav != Nav::Places && ui.button(tr("Show map")).clicked() {
                self.toggle_side(SidePanel::Map);
            }
        });
    }

    /// EXIF rows of `photo`, read in the background on first request.
    fn exif_for(&mut self, photo: &Photo) -> Option<Vec<(&'static str, String)>> {
        if let Some(rows) = self.exif.get(&photo.id) {
            return Some(rows.clone());
        }
        if let Some((_, rows)) = self.display_info(photo.id) {
            return Some(rows);
        }
        if self.exif_waiting.insert(photo.id) {
            let _ = self.urgent_tx.send(crate::worker::ThumbJob::Exif {
                id: photo.id,
                path: photo.path.clone(),
            });
        }
        None
    }

    fn side_map(&mut self, ui: &mut egui::Ui, photo: &Photo) {
        let Some((lat, lon)) = photo.place() else {
            self.side_preview(ui, photo);
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(tr("No GPS location")).size(15.0).strong().color(theme::text()));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(tr("This photo's EXIF has no GPS coordinates."))
                        .size(12.0)
                        .color(theme::muted()),
                );
                ui.add_space(10.0);
                if ui.button(tr("Geotag it in Places")).clicked() {
                    self.show_on_map(photo.id);
                }
            });
            return;
        };
        if self.side_map_photo != Some(photo.id) {
            self.side_map_photo = Some(photo.id);
            self.side_pan = Vec2::ZERO;
        }
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, (width * 1.1).clamp(220.0, 420.0)), Sense::click_and_drag());
        if response.dragged() {
            self.side_pan += response.drag_delta();
        }
        if response.double_clicked() {
            self.side_pan = Vec2::ZERO;
        }
        let mut zoom_by = 0i32;
        if response.hovered() {
            let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
            zoom_by = super::places::wheel_steps(&mut self.side_wheel, scroll) + super::places::pinch_steps(&mut self.side_pinch, pinch);
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, super::places::MAP_BACKDROP);
        let view = MapView {
            lat,
            lon,
            zoom: self.side_zoom,
            pan: self.side_pan,
        };
        self.paint_tiles(&painter, rect, view);
        if self.show_tracks {
            self.paint_tracks(&painter, rect, view, None);
        }

        let (px, py) = geo::latlon_to_world(lat, lon, self.side_zoom);
        let to_screen = |other_lat: f64, other_lon: f64| {
            let (x, y) = geo::latlon_to_world(other_lat, other_lon, self.side_zoom);
            rect.center() + Vec2::new((x - px) as f32, (y - py) as f32) + self.side_pan
        };
        let mut covered = super::places::PinCells::default();
        for other in &self.photos {
            if other.id == photo.id {
                continue;
            }
            if let Some((other_lat, other_lon)) = other.place() {
                let pos = to_screen(other_lat, other_lon);
                if rect.contains(pos) && covered.first(pos) {
                    painter.circle_filled(pos, 4.0, ACCENT);
                    painter.circle_stroke(pos, 5.0, Stroke::new(1.5, WHITE));
                }
            }
        }
        paint_pin(&painter, rect.center() + self.side_pan);
        painter.rect_stroke(rect, theme::THUMB_RADIUS, Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
        self.paint_map_notes(&painter, rect, view);

        // Clear of the provider's credit, which may wrap onto two lines.
        let step = super::places::zoom_step(rect.height() - 52.0);
        if let Some(next) = super::places::zoom_control(ui, rect.right_top() + Vec2::new(-36.0, 8.0), self.side_zoom, step, "side-zoom") {
            zoom_by = i32::from(next) - i32::from(self.side_zoom);
        }
        if zoom_by != 0 {
            let next = (self.side_zoom as i32 + zoom_by).clamp(MIN_ZOOM as i32, MAX_ZOOM as i32) as u8;
            if next != self.side_zoom {
                self.side_pan *= 2f32.powi(i32::from(next) - i32::from(self.side_zoom));
                self.side_zoom = next;
            }
        }

        ui.add_space(8.0);
        let kind = tr(if photo.pinned() { "Pinned in Picasso" } else { "From camera GPS" });
        ui.label(egui::RichText::new(format!("{lat:.5}, {lon:.5}")).color(theme::text()));
        ui.label(egui::RichText::new(kind).size(12.0).color(theme::muted()));
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("Open in OpenStreetMap")).clicked() {
                super::places::open_osm(lat, lon, self.side_zoom.max(12));
            }
            if ui.button(tr("Show in Places")).clicked() {
                self.show_on_map(photo.id);
            }
        });
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(14.0);
    ui.label(egui::RichText::new(title.to_uppercase()).size(11.0).strong().color(theme::muted()));
    ui.add_space(2.0);
}

/// Two-column grid of `rows`, whose labels are English keys for `tr`.
fn property_grid(ui: &mut egui::Ui, id: (&str, i64), rows: &[(&'static str, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(Vec2::new(12.0, 5.0))
        .min_col_width(70.0)
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(egui::RichText::new(tr(label)).size(12.0).color(theme::muted()));
                ui.add(egui::Label::new(egui::RichText::new(value).size(12.5).color(theme::text())).wrap());
                ui.end_row();
            }
        });
}

fn paint_pin(painter: &egui::Painter, tip: Pos2) {
    let head = tip + Vec2::new(0.0, -18.0);
    let red = Color32::from_rgb(0xE0, 0x45, 0x3A);
    painter.add(egui::Shape::convex_polygon(
        vec![head + Vec2::new(-6.5, 3.0), head + Vec2::new(6.5, 3.0), tip],
        red,
        Stroke::NONE,
    ));
    painter.circle_filled(head, 8.0, red);
    painter.circle_stroke(head, 8.0, Stroke::new(1.5, WHITE));
    painter.circle_filled(head, 3.0, WHITE);
}

/// Info and Map toggles in the photo tray.
pub(super) fn tray_icon(ui: &mut egui::Ui, icon: TrayIcon, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(32.0, 28.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    let background = if on {
        theme::select()
    } else if response.hovered() {
        theme::hover()
    } else {
        theme::tray()
    };
    painter.rect_filled(rect, theme::RADIUS, background);
    let color = if on { ACCENT } else { theme::text() };
    let c = rect.center();
    match icon {
        TrayIcon::Info => {
            painter.circle_stroke(c, 8.0, Stroke::new(1.6, color));
            painter.circle_filled(c + Vec2::new(0.0, -4.0), 1.3, color);
            painter.line_segment([c + Vec2::new(0.0, -1.0), c + Vec2::new(0.0, 4.5)], Stroke::new(1.8, color));
        }
        TrayIcon::Map => theme::paint_pin(painter, c, 5.5, color, background),
    }
    response
}
