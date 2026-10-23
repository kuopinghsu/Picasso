//! Places: a map of geotagged photos. Click a pin to open it.
//! With Geotag on, a click stores a pin for the current selection.

use eframe::egui::{self, epaint::TextShape, Align2, Color32, FontId, Pos2, Rect, Sense, TextureHandle, Vec2};

use crate::geo::{self, TileJob, TILE_PX};
use crate::i18n::{tr, trf};
use crate::liberty::{LabelKind, MapLabel};
use crate::types::Photo;

use super::theme::{self, ACCENT, STAR};
use super::PicassoApp;

/// Map background while tiles load.
pub(super) const MAP_BACKDROP: Color32 = Color32::from_rgb(0xD6, 0xE4, 0xF0);

/// A loaded map tile and the names to write over it.
pub(super) struct MapTile {
    texture: TextureHandle,
    labels: Vec<MapLabel>,
}

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
                    self.reload_view();
                }
            }
            if ui.button(tr("Open map")).on_hover_text(tr("Open this view in OpenStreetMap")).clicked() {
                open_osm(self.map_lat, self.map_lon, self.map_zoom);
            }
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

        ui.painter().rect_filled(rect, 0, MAP_BACKDROP);
        let view = MapView {
            lat: self.map_lat,
            lon: self.map_lon,
            zoom: self.map_zoom,
            pan: self.map_pan,
        };
        let painter = ui.painter_at(rect);
        self.paint_tiles(&painter, rect, view);
        let hit = self.paint_pins(ui, rect);
        self.paint_map_notes(&painter, rect);
        if response.double_clicked() {
            if let Some(id) = hit {
                if let Some(index) = self.index_of(id) {
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
                    self.reload_view();
                }
            }
        }

        if self.photos.is_empty() && self.selection.is_empty() && !self.loading {
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
            self.frame_after_load = false;
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

    /// Paints the map tiles of `view` into `rect`, with their names, and
    /// queues any that are not loaded yet.
    pub(super) fn paint_tiles(&mut self, painter: &egui::Painter, rect: Rect, view: MapView) {
        let source = self.map_source;
        if self.map_key_missing() {
            return;
        }
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
        let mut labels = Vec::new();
        for x in left..right.min(n) {
            for y in top..bottom.min(n) {
                let tile = Rect::from_min_size(
                    Pos2::new(origin_x + x as f32 * TILE_PX as f32, origin_y + y as f32 * TILE_PX as f32),
                    Vec2::splat(TILE_PX as f32),
                );
                if let Some(loaded) = self.tiles.get(&(z, x, y)) {
                    painter.image(
                        loaded.texture.id(),
                        tile,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    labels.extend(
                        loaded.labels.iter().map(|label| (tile.min + Vec2::from(label.pos) * TILE_PX as f32, label)),
                    );
                } else {
                    wanted.push((x, y));
                }
            }
        }
        paint_labels(painter, rect, labels);
        for (x, y) in wanted.into_iter().take(8) {
            let key = (z, x, y);
            if self.tile_wait.contains(&key) || self.tile_fail.contains(&key) {
                continue;
            }
            self.tile_wait.insert(key);
            let _ = self.tile_tx.send(TileJob { source, key: self.carto_key.clone(), z, x, y });
        }
    }

    fn map_key_missing(&self) -> bool {
        self.map_source.needs_key() && self.carto_key.is_empty()
    }

    /// The provider's credit in the bottom-right corner of the map in
    /// `rect`, and a note in the middle when the source needs a key.
    pub(super) fn paint_map_notes(&self, painter: &egui::Painter, rect: Rect) {
        let ink = Color32::from_gray(60);
        if self.map_key_missing() {
            let galley = painter.layout(
                tr("CartoDB Positron needs an API key. Add it in Settings → Map.").to_string(),
                FontId::proportional(13.0),
                ink,
                (rect.width() - 48.0).clamp(80.0, 320.0),
            );
            let at = rect.center() - galley.size() / 2.0;
            painter.rect_filled(Rect::from_min_size(at, galley.size()).expand(10.0), 6, Color32::from_white_alpha(235));
            painter.galley(at, galley, ink);
        }
        let galley = painter.layout(
            self.map_source.attribution().to_string(),
            FontId::proportional(10.0),
            ink,
            (rect.width() - 16.0).max(60.0),
        );
        let at = rect.right_bottom() - galley.size() - Vec2::new(4.0, 2.0);
        painter.rect_filled(Rect::from_min_size(at, galley.size()).expand(2.0), 2, Color32::from_white_alpha(190));
        painter.galley(at, galley, ink);
    }

    /// Forgets loaded and failed tiles, after the map source or its key changes.
    pub(super) fn reset_tiles(&mut self) {
        self.tiles.clear();
        self.tile_wait.clear();
        self.tile_fail.clear();
    }

    fn paint_pins(&self, ui: &mut egui::Ui, rect: Rect) -> Option<i64> {
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let mut hit = None;
        let mut best = 14.0_f32;
        let (cx, cy) = geo::latlon_to_world(self.map_lat, self.map_lon, self.map_zoom);
        let middle = rect.center() + self.map_pan;
        let painter = ui.painter();
        let mut covered = PinCells::default();
        let mut chosen = Vec::new();
        for photo in &self.photos {
            let Some((lat, lon)) = photo.place() else { continue };
            let (x, y) = geo::latlon_to_world(lat, lon, self.map_zoom);
            let pos = middle + Vec2::new((x - cx) as f32, (y - cy) as f32);
            if !rect.contains(pos) {
                continue;
            }
            if let Some(pointer) = pointer {
                let distance = pointer.distance(pos);
                if distance < best {
                    best = distance;
                    hit = Some(photo.id);
                }
            }
            if self.selection.contains(&photo.id) {
                chosen.push((pos, photo));
            } else if covered.first(pos) {
                let color = if photo.pinned() { STAR } else { ACCENT };
                painter.circle_filled(pos, 5.0, color);
                painter.circle_stroke(pos, 6.0, egui::Stroke::new(1.5, Color32::WHITE));
            }
        }
        // Selected pins go on top. Names are written only for a few, since
        // a large selection would bury the map in text.
        let named = chosen.len() <= 50;
        let mut covered = PinCells::default();
        for (pos, photo) in chosen {
            if !named && !covered.first(pos) {
                continue;
            }
            let color = if photo.pinned() { STAR } else { ACCENT };
            painter.circle_filled(pos, 7.0, color);
            painter.circle_stroke(pos, 9.0, egui::Stroke::new(1.5, Color32::WHITE));
            if named {
                painter.text(
                    pos + Vec2::new(10.0, -8.0),
                    Align2::LEFT_CENTER,
                    &photo.filename,
                    FontId::proportional(11.0),
                    theme::text(),
                );
            }
        }
        hit
    }

    pub(super) fn pump_tiles(&mut self, ctx: &egui::Context) {
        for _ in 0..4 {
            match self.tile_rx.try_recv() {
                // Left over from the map source used before.
                Ok(done) if done.source != self.map_source => {}
                Ok(done) => {
                    let key = (done.z, done.x, done.y);
                    self.tile_wait.remove(&key);
                    if let Some(image) = done.image {
                        if self.tiles.len() > 160 {
                            self.tiles.clear();
                        }
                        let name = format!("tile-{}-{}-{}-{}", done.source.code(), done.z, done.x, done.y);
                        if let Some(texture) = super::upload(ctx, &name, image.w, image.h, &image.rgba) {
                            self.tiles.insert(key, MapTile { texture, labels: image.labels });
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

/// Screen cells that already hold a pin. Pins a few points apart cover each
/// other, so drawing one per cell looks the same and keeps a map with tens
/// of thousands of photos quick to draw.
#[derive(Default)]
pub(super) struct PinCells(std::collections::HashSet<(i32, i32)>);

impl PinCells {
    const SIZE: f32 = 4.0;

    /// True for the first pin in the cell at `pos`.
    pub(super) fn first(&mut self, pos: Pos2) -> bool {
        self.0.insert(((pos.x / Self::SIZE).floor() as i32, (pos.y / Self::SIZE).floor() as i32))
    }
}

/// Writes map names over the tiles, most important first, skipping any that
/// would overlap one already written.
fn paint_labels(painter: &egui::Painter, rect: Rect, mut labels: Vec<(Pos2, &MapLabel)>) {
    const HALO: [Vec2; 8] = [
        Vec2::new(-1.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(0.0, -1.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(-0.8, -0.8),
        Vec2::new(0.8, -0.8),
        Vec2::new(-0.8, 0.8),
        Vec2::new(0.8, 0.8),
    ];
    labels.sort_by_key(|(_, label)| label.rank);
    let mut taken: Vec<Rect> = Vec::new();
    let mut roads: Vec<(&str, Pos2)> = Vec::new();
    for (pos, label) in labels {
        if !rect.contains(pos) {
            continue;
        }
        let road = label.kind == LabelKind::Road;
        if road && label.span * TILE_PX as f32 * 0.9 < label.text.chars().count() as f32 * 4.0 {
            continue;
        }
        // The same street comes back from every tile and piece it crosses.
        if road && roads.iter().any(|(text, at)| *text == label.text && at.distance(pos) < 200.0) {
            continue;
        }
        let (size, color) = label_style(label.kind);
        let galley = painter.layout_no_wrap(label.text.clone(), FontId::proportional(size), color);
        let half = galley.size() / 2.0;
        if road && galley.size().x > label.span * TILE_PX as f32 * 0.9 {
            continue;
        }
        let (sin, cos) = label.angle.sin_cos();
        let reach = Vec2::new(half.x * cos.abs() + half.y * sin.abs(), half.x * sin.abs() + half.y * cos.abs());
        let bounds = Rect::from_center_size(pos, reach * 2.0).expand(3.0);
        if taken.iter().any(|other| other.intersects(bounds)) {
            continue;
        }
        taken.push(bounds);
        if road {
            roads.push((&label.text, pos));
        }
        // Text turns about its top-left corner, so start where the turned
        // middle lands on `pos`.
        let corner = pos - Vec2::new(half.x * cos - half.y * sin, half.x * sin + half.y * cos);
        let halo = Color32::from_white_alpha(225);
        for offset in HALO {
            painter.add(
                TextShape::new(corner + offset, galley.clone(), color)
                    .with_angle(label.angle)
                    .with_override_text_color(halo),
            );
        }
        painter.add(TextShape::new(corner, galley, color).with_angle(label.angle));
    }
}

/// Font size and color of a kind of name, after the Liberty style.
fn label_style(kind: LabelKind) -> (f32, Color32) {
    match kind {
        LabelKind::Country => (14.0, Color32::from_rgb(0x33, 0x33, 0x44)),
        LabelKind::State => (11.5, Color32::from_rgb(0x66, 0x33, 0x33)),
        LabelKind::City => (14.0, Color32::from_rgb(0x33, 0x33, 0x33)),
        LabelKind::Town => (12.5, Color32::from_rgb(0x33, 0x33, 0x33)),
        LabelKind::Village => (11.5, Color32::from_rgb(0x33, 0x33, 0x33)),
        LabelKind::Suburb => (10.5, Color32::from_rgb(0x55, 0x55, 0x55)),
        LabelKind::Water => (11.5, Color32::from_rgb(0x49, 0x5E, 0x91)),
        LabelKind::Road => (10.5, Color32::from_rgb(0x76, 0x65, 0x55)),
    }
}

pub(super) fn open_osm(lat: f64, lon: f64, zoom: u8) {
    let url = format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map={zoom}/{lat:.6}/{lon:.6}");
    let _ = open::that(url);
}
