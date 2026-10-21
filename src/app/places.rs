//! Places: an OpenStreetMap of geotagged photos. Click a pin to open it.
//! With Geotag on, a click stores a pin for the current selection.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::geo::{self, TileJob, TILE_PX};
use crate::i18n::{tr, trf};
use crate::types::Photo;

use super::theme::{self, ACCENT, STAR};
use super::PicassoApp;

/// Center, zoom level, and unbaked drag offset of a map.
#[derive(Clone, Copy)]
pub(super) struct MapView {
    pub lat: f64,
    pub lon: f64,
    pub zoom: u8,
    pub pan: Vec2,
}

impl PicassoApp {
    pub(super) fn places_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let label = if self.map_place { "Click the map…" } else { "Geotag" };
            if ui.button(tr(label)).on_hover_text(tr("Place the selected photos on the map")).clicked() {
                if self.selection.is_empty() {
                    self.activity = tr("Select photos, then Geotag").into();
                } else {
                    self.map_place = !self.map_place;
                }
            }
            if ui.button(tr("Clear geotag")).on_hover_text(tr("Remove the pin you placed. Camera GPS stays.")).clicked() {
                let ids: Vec<i64> = self.selection.iter().copied().collect();
                if !ids.is_empty() {
                    self.db.clear_pins(&ids);
                    self.activity = tr("Cleared geotag").into();
                    self.reload();
                }
            }
            if ui.button(tr("Open map")).on_hover_text(tr("Open this view in OpenStreetMap")).clicked() {
                open_osm(self.map_lat, self.map_lon, self.map_zoom);
            }
            ui.label(egui::RichText::new("© OpenStreetMap").size(11.0).color(theme::muted()));
        });
        ui.add_space(4.0);

        let size = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(size.max(Vec2::new(80.0, 80.0)), Sense::click_and_drag());
        if response.dragged() {
            self.map_pan += response.drag_delta();
        }
        if response.drag_stopped() {
            self.bake_map_pan();
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                let next = (self.map_zoom as i32 + if scroll > 0.0 { 1 } else { -1 }).clamp(2, 16) as u8;
                if next != self.map_zoom {
                    self.map_zoom = next;
                    self.map_pan = Vec2::ZERO;
                    self.tiles.clear();
                }
            }
        }

        ui.painter().rect_filled(rect, 0, Color32::from_rgb(0xD6, 0xE4, 0xF0));
        let view = MapView {
            lat: self.map_lat,
            lon: self.map_lon,
            zoom: self.map_zoom,
            pan: self.map_pan,
        };
        self.paint_tiles(&ui.painter_at(rect), rect, view);
        let hit = self.paint_pins(ui, rect);
        if response.double_clicked() {
            if let Some(id) = hit {
                if let Some(index) = self.photos.iter().position(|photo| photo.id == id) {
                    self.open_at(index);
                }
            }
        } else if response.clicked() && !response.dragged() {
            if let Some(id) = hit {
                self.selection.clear();
                self.selection.insert(id);
                self.anchor = Some(id);
            } else if self.map_place && !self.selection.is_empty() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let (lat, lon) = self.screen_to_latlon(pointer, rect);
                    let ids: Vec<i64> = self.selection.iter().copied().collect();
                    self.db.set_pins(&ids, lat, lon);
                    self.map_place = false;
                    let text = if ids.len() == 1 { "Geotagged {} photo" } else { "Geotagged {} photos" };
                    self.activity = trf(text, &[&ids.len()]);
                    self.map_lat = lat;
                    self.map_lon = lon;
                    self.reload();
                }
            }
        }

        if self.photos.is_empty() && self.selection.is_empty() {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                tr("No geotagged photos yet. Select photos, press Geotag, then click the map."),
                FontId::proportional(14.0),
                theme::text(),
            );
        }
    }

    pub(super) fn frame_map(&mut self) {
        let chosen = self
            .selection
            .iter()
            .find_map(|id| self.db.photo(*id).and_then(|photo| photo.place()));
        let places: Vec<(f64, f64)> = self.photos.iter().filter_map(Photo::place).collect();
        if let Some((lat, lon)) = chosen.or_else(|| places.first().copied()) {
            self.map_lat = lat;
            self.map_lon = lon;
            self.map_zoom = if places.len() <= 1 { 12 } else { 4 };
        }
        self.map_pan = Vec2::ZERO;
    }

    pub(super) fn show_on_map(&mut self, id: i64) {
        let place = self.db.photo(id).and_then(|photo| photo.place());
        self.selection.clear();
        self.selection.insert(id);
        self.set_nav(crate::types::Nav::Places);
        if let Some((lat, lon)) = place {
            self.map_lat = lat;
            self.map_lon = lon;
            self.map_zoom = 14;
            self.map_pan = Vec2::ZERO;
        }
    }

    fn bake_map_pan(&mut self) {
        let (x, y) = geo::latlon_to_world(self.map_lat, self.map_lon, self.map_zoom);
        let (lat, lon) = geo::world_to_latlon(x - f64::from(self.map_pan.x), y - f64::from(self.map_pan.y), self.map_zoom);
        self.map_lat = lat;
        self.map_lon = lon;
        self.map_pan = Vec2::ZERO;
    }

    fn screen_to_latlon(&self, pointer: Pos2, rect: Rect) -> (f64, f64) {
        let (cx, cy) = geo::latlon_to_world(self.map_lat, self.map_lon, self.map_zoom);
        let world_x = cx + f64::from(pointer.x - rect.center().x - self.map_pan.x);
        let world_y = cy + f64::from(pointer.y - rect.center().y - self.map_pan.y);
        geo::world_to_latlon(world_x, world_y, self.map_zoom)
    }

    fn world_to_screen(&self, world_x: f64, world_y: f64, rect: Rect) -> Pos2 {
        let (cx, cy) = geo::latlon_to_world(self.map_lat, self.map_lon, self.map_zoom);
        Pos2::new(
            rect.center().x + (world_x - cx) as f32 + self.map_pan.x,
            rect.center().y + (world_y - cy) as f32 + self.map_pan.y,
        )
    }

    /// Paints the OpenStreetMap tiles of `view` into `rect` and queues any
    /// that are not loaded yet.
    pub(super) fn paint_tiles(&mut self, painter: &egui::Painter, rect: Rect, view: MapView) {
        let z = view.zoom;
        let n = 1u32 << z;
        let (cx, cy) = geo::latlon_to_world(view.lat, view.lon, z);
        let origin_x = rect.center().x - cx as f32 + view.pan.x;
        let origin_y = rect.center().y - cy as f32 + view.pan.y;
        let left = ((rect.left() - origin_x) / TILE_PX as f32).floor().max(0.0) as u32;
        let top = ((rect.top() - origin_y) / TILE_PX as f32).floor().max(0.0) as u32;
        let right = ((rect.right() - origin_x) / TILE_PX as f32).ceil().min(n as f32) as u32;
        let bottom = ((rect.bottom() - origin_y) / TILE_PX as f32).ceil().min(n as f32) as u32;
        let mut wanted = Vec::new();
        for x in left..right.min(n) {
            for y in top..bottom.min(n) {
                let tile = Rect::from_min_size(
                    Pos2::new(origin_x + x as f32 * TILE_PX as f32, origin_y + y as f32 * TILE_PX as f32),
                    Vec2::splat(TILE_PX as f32),
                );
                if let Some(texture) = self.tiles.get(&(z, x, y)).cloned() {
                    painter.image(
                        texture.id(),
                        tile,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    wanted.push((x, y));
                }
            }
        }
        for (x, y) in wanted.into_iter().take(8) {
            let key = (z, x, y);
            if self.tile_wait.contains(&key) || self.tile_fail.contains(&key) {
                continue;
            }
            self.tile_wait.insert(key);
            let _ = self.tile_tx.send(TileJob { z, x, y });
        }
    }

    fn paint_pins(&self, ui: &mut egui::Ui, rect: Rect) -> Option<i64> {
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let mut hit = None;
        let mut best = 14.0_f32;
        for photo in &self.photos {
            let Some((lat, lon)) = photo.place() else { continue };
            let (x, y) = geo::latlon_to_world(lat, lon, self.map_zoom);
            let pos = self.world_to_screen(x, y, rect);
            if !rect.contains(pos) {
                continue;
            }
            let selected = self.selection.contains(&photo.id);
            let color = if photo.pinned() { STAR } else { ACCENT };
            ui.painter().circle_filled(pos, if selected { 7.0 } else { 5.0 }, color);
            ui.painter().circle_stroke(pos, if selected { 9.0 } else { 6.0 }, egui::Stroke::new(1.5, Color32::WHITE));
            if selected {
                ui.painter().text(
                    pos + Vec2::new(10.0, -8.0),
                    Align2::LEFT_CENTER,
                    &photo.filename,
                    FontId::proportional(11.0),
                    theme::text(),
                );
            }
            if let Some(pointer) = pointer {
                let distance = pointer.distance(pos);
                if distance < best {
                    best = distance;
                    hit = Some(photo.id);
                }
            }
        }
        hit
    }

    pub(super) fn pump_tiles(&mut self, ctx: &egui::Context) {
        for _ in 0..4 {
            match self.tile_rx.try_recv() {
                Ok(done) => {
                    let key = (done.z, done.x, done.y);
                    self.tile_wait.remove(&key);
                    if let Some((w, h, rgba)) = done.image {
                        if self.tiles.len() > 160 {
                            self.tiles.clear();
                        }
                        if let Some(texture) = super::upload(ctx, &format!("tile-{}-{}-{}", done.z, done.x, done.y), w, h, &rgba) {
                            self.tiles.insert(key, texture);
                        }
                    } else {
                        self.tile_fail.insert(key);
                    }
                }
                Err(_) => break,
            }
        }
        if !self.tile_wait.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }
}

pub(super) fn open_osm(lat: f64, lon: f64, zoom: u8) {
    let url = format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map={zoom}/{lat:.6}/{lon:.6}");
    let _ = open::that(url);
}
