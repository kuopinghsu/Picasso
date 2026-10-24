//! Places: a map of geotagged photos. Click a pin to open it.
//! With Geotag on, a click stores a pin for the current selection.
//! GPS tracks from GPX and KML files are drawn under the pins.

use eframe::egui::{self, epaint::TextShape, Align2, Color32, FontId, Pos2, Rect, Sense, TextureHandle, Vec2};

use crate::geo::{self, TileJob, TILE_PX};
use crate::i18n::{tr, trf};
use crate::liberty::{LabelKind, MapLabel};
use crate::track::{format_distance, Bounds, MapTrack};
use crate::types::{format_duration, format_timestamp, Photo};

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
            if ui.button(tr("Open in OpenStreetMap")).on_hover_text(tr("Open this view in OpenStreetMap")).clicked() {
                open_osm(self.map_lat, self.map_lon, self.map_zoom);
            }
            ui.separator();
            let hint = tr("Show the GPS tracks of the GPX and KML files in your folders. Double-click a track to zoom to it.");
            if ui.checkbox(&mut self.show_tracks, tr("Tracks")).on_hover_text(hint).changed() {
                self.config.set("map_tracks", self.show_tracks);
            }
            if self.show_tracks && !self.tracks.is_empty() {
                let count = self.tracks.len();
                let text = if count == 1 { "{} track" } else { "{} tracks" };
                ui.label(egui::RichText::new(trf(text, &[&count])).color(theme::muted()));
            }
        });
        ui.add_space(4.0);

        let size = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(size.max(Vec2::new(80.0, 80.0)), Sense::click_and_drag());
        self.map_size = rect.size();
        if response.dragged() {
            self.map_pan += response.drag_delta();
        }
        if response.drag_stopped() {
            self.bake_map_pan();
        }
        if response.hovered() {
            let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
            let steps = wheel_steps(&mut self.map_wheel, scroll) + pinch_steps(&mut self.map_pinch, pinch);
            if let Some(pointer) = response.hover_pos().filter(|_| steps != 0) {
                self.zoom_map_at(steps, pointer, rect);
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
        let on_track = if self.show_tracks { self.paint_tracks(&painter, rect, view, response.hover_pos()) } else { None };
        let hit = self.paint_pins(ui, rect);
        let on_track = on_track.filter(|_| hit.is_none() && !response.dragged());
        self.paint_map_notes(&painter, rect);
        if let Some(track) = on_track.and_then(|index| self.tracks.get(index)) {
            response.clone().on_hover_ui_at_pointer(|ui| track_tooltip(ui, track));
        }
        if response.double_clicked() {
            if let Some(id) = hit {
                if let Some(index) = self.index_of(id) {
                    self.open_at(index);
                }
            } else if let Some(bounds) = on_track.and_then(|index| self.tracks.get(index)).map(|track| track.bounds) {
                self.frame_bounds(bounds);
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

        let tracks_shown = self.show_tracks && !self.tracks.is_empty();
        if self.photos.is_empty() && self.selection.is_empty() && !self.loading && !tracks_shown {
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
        } else if self.show_tracks && !self.tracks.is_empty() {
            let all = self.tracks.iter().fold(Bounds::EMPTY, |all, track| all.union(track.bounds));
            self.frame_bounds(all);
        }
        self.map_pan = Vec2::ZERO;
    }

    /// Centers the Places map on `bounds`, in world pixels at zoom 0, at
    /// the closest zoom that shows all of it.
    fn frame_bounds(&mut self, bounds: Bounds) {
        if bounds.is_empty() {
            return;
        }
        let [x, y] = bounds.center();
        let (lat, lon) = geo::world_to_latlon(x, y, 0);
        let [w, h] = bounds.size();
        let room = self.map_size * 0.85;
        let mut zoom = 16u8;
        while zoom > 2 && (w * f64::from(1u32 << zoom) > f64::from(room.x) || h * f64::from(1u32 << zoom) > f64::from(room.y)) {
            zoom -= 1;
        }
        self.map_lat = lat;
        self.map_lon = lon;
        self.map_zoom = zoom;
        self.map_pan = Vec2::ZERO;
    }

    /// Draws the GPS tracks over the tiles of `view`. Returns the index of
    /// the track within a few points of `pointer`, drawn on top.
    pub(super) fn paint_tracks(&self, painter: &egui::Painter, rect: Rect, view: MapView, pointer: Option<Pos2>) -> Option<usize> {
        const REACH: f32 = 6.0;
        let scale = f64::from(1u32 << view.zoom);
        let (cx, cy) = geo::latlon_to_world(view.lat, view.lon, view.zoom);
        let middle = rect.center() + view.pan;
        let to_screen = |[x, y]: [f64; 2]| middle + Vec2::new((x * scale - cx) as f32, (y * scale - cy) as f32);
        let area = rect.expand(8.0);
        let pointer = pointer.filter(|pointer| rect.contains(*pointer));
        let mut drawn: Vec<(usize, Vec<Vec<Pos2>>)> = Vec::new();
        let mut hovered = None;
        let mut best = REACH;
        for (index, track) in self.tracks.iter().enumerate() {
            let mut runs = Vec::new();
            for segment in &track.segments {
                let seen = Rect::from_two_pos(to_screen(segment.bounds.min), to_screen(segment.bounds.max));
                if seen.intersects(area) {
                    visible_runs(segment.points.iter().map(|point| to_screen(*point)), area, &mut runs);
                }
            }
            if runs.is_empty() {
                continue;
            }
            if let Some(pointer) = pointer {
                let distance = runs.iter().map(|run| distance_to_run(pointer, run)).fold(f32::INFINITY, f32::min);
                if distance < best {
                    best = distance;
                    hovered = Some(index);
                }
            }
            drawn.push((index, runs));
        }
        drawn.sort_by_key(|(index, _)| Some(*index) == hovered);
        for (index, runs) in &drawn {
            let color = track_color(self.tracks[*index].id);
            let wide = Some(*index) == hovered;
            let (casing, line) = if wide { (8.0, 5.0) } else { (5.0, 3.0) };
            for run in runs {
                if run.len() == 1 {
                    painter.circle_filled(run[0], casing / 2.0, Color32::WHITE);
                    painter.circle_filled(run[0], line / 2.0, color);
                } else {
                    painter.add(egui::Shape::line(run.clone(), egui::Stroke::new(casing, Color32::from_white_alpha(220))));
                }
            }
            for run in runs.iter().filter(|run| run.len() > 1) {
                painter.add(egui::Shape::line(run.clone(), egui::Stroke::new(line, color)));
            }
        }
        hovered
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

    /// Zooms the Places map by `steps` levels, keeping the place under
    /// `pointer` where it is.
    fn zoom_map_at(&mut self, steps: i32, pointer: Pos2, rect: Rect) {
        let next = (self.map_zoom as i32 + steps).clamp(2, 16) as u8;
        if next == self.map_zoom {
            return;
        }
        let (lat, lon) = self.screen_to_latlon(pointer, rect);
        let (x, y) = geo::latlon_to_world(lat, lon, next);
        let offset = pointer - rect.center();
        let (lat, lon) = geo::world_to_latlon(x - f64::from(offset.x), y - f64::from(offset.y), next);
        self.map_lat = lat;
        self.map_lon = lon;
        self.map_zoom = next;
        self.map_pan = Vec2::ZERO;
        self.tiles.clear();
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

/// Scroll that makes one zoom step. A mouse wheel notch scrolls 40 points.
const WHEEL_STEP: f32 = 30.0;

/// Zoom levels to move for this frame's scroll `delta`, one per wheel
/// notch. egui spreads a notch over several frames, so the deltas add up in
/// `wheel` until they make a step; what is left of a notch after its step
/// is dropped when the scrolling stops.
pub(super) fn wheel_steps(wheel: &mut f32, delta: f32) -> i32 {
    if delta.abs() < 0.01 {
        *wheel = 0.0;
        return 0;
    }
    if wheel.signum() != delta.signum() {
        *wheel = 0.0;
    }
    *wheel += delta;
    if wheel.abs() < WHEEL_STEP {
        return 0;
    }
    let steps = (*wheel / WHEEL_STEP).trunc();
    *wheel = 0.0;
    steps as i32
}

/// Spread of the fingers, as log2 of the scale, that makes one zoom step.
const PINCH_STEP: f32 = 0.5;

/// Zoom levels to move for this frame's trackpad pinch `factor`. The pinch
/// adds up in `pinch` until it makes a step; turning back starts over.
pub(super) fn pinch_steps(pinch: &mut f32, factor: f32) -> i32 {
    if !factor.is_finite() || factor <= 0.0 || (factor - 1.0).abs() < 1e-4 {
        return 0;
    }
    let delta = factor.log2();
    if pinch.signum() != delta.signum() {
        *pinch = 0.0;
    }
    *pinch += delta;
    let steps = (*pinch / PINCH_STEP).trunc();
    *pinch -= steps * PINCH_STEP;
    steps as i32
}

/// Track colors, picked by track id so each keeps its color.
const TRACK_COLORS: [Color32; 6] = [
    Color32::from_rgb(0xE5, 0x39, 0x35),
    Color32::from_rgb(0x1E, 0x6F, 0xD9),
    Color32::from_rgb(0x2E, 0x9D, 0x44),
    Color32::from_rgb(0x8E, 0x24, 0xAA),
    Color32::from_rgb(0xF0, 0x7C, 0x00),
    Color32::from_rgb(0x00, 0x89, 0x7B),
];

fn track_color(id: i64) -> Color32 {
    TRACK_COLORS[id.rem_euclid(TRACK_COLORS.len() as i64) as usize]
}

/// Adds the stretches of a line that cross `area` to `runs`. Points closer
/// than a pixel and a half to the one kept before are dropped, so a long
/// track costs little to draw at any zoom. A track smaller than that is
/// left as a run of one point.
fn visible_runs(points: impl Iterator<Item = Pos2>, area: Rect, runs: &mut Vec<Vec<Pos2>>) {
    let mut run: Vec<Pos2> = Vec::new();
    let mut prev: Option<Pos2> = None;
    for pos in points {
        if let Some(last) = prev {
            if Rect::from_two_pos(last, pos).intersects(area) {
                if run.is_empty() {
                    run.push(last);
                }
                if run.last().is_some_and(|kept| kept.distance(pos) >= 1.5) {
                    run.push(pos);
                }
            } else if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
        }
        prev = Some(pos);
    }
    if !run.is_empty() {
        runs.push(run);
    }
}

fn distance_to_run(pointer: Pos2, run: &[Pos2]) -> f32 {
    if let [only] = run {
        return pointer.distance(*only);
    }
    run.windows(2)
        .map(|pair| {
            let along = pair[1] - pair[0];
            let t = ((pointer - pair[0]).dot(along) / along.length_sq().max(1e-6)).clamp(0.0, 1.0);
            pointer.distance(pair[0] + along * t)
        })
        .fold(f32::INFINITY, f32::min)
}

fn track_tooltip(ui: &mut egui::Ui, track: &MapTrack) {
    ui.label(egui::RichText::new(&track.name).strong());
    match track.time_span {
        Some((start, end)) => {
            ui.label(format_timestamp(start));
            ui.label(format!("{} · {}", format_distance(track.meters), format_duration((end - start) as f64)));
        }
        None => {
            ui.label(format_distance(track.meters));
        }
    }
    let file = std::path::Path::new(&track.path).file_name().map(|name| name.to_string_lossy().into_owned());
    ui.label(egui::RichText::new(file.unwrap_or_default()).small().color(theme::muted()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wheel_notch_spread_over_frames_zooms_one_level() {
        let mut wheel = 0.0;
        // One 40-point notch, as egui smooths it.
        let steps: i32 = [12.0, 10.0, 8.0, 5.0, 3.0, 1.5, 0.5, 0.0].iter().map(|delta| wheel_steps(&mut wheel, *delta)).sum();
        assert_eq!(steps, 1);
        let steps: i32 = [-20.0, -20.0, -20.0, -20.0, 0.0].iter().map(|delta| wheel_steps(&mut wheel, *delta)).sum();
        assert_eq!(steps, -2);
        assert_eq!(wheel_steps(&mut wheel, 10.0), 0);
        assert_eq!(wheel_steps(&mut wheel, -10.0), 0, "turning back starts over");
    }

    #[test]
    fn a_pinch_zooms_one_level_per_step_of_spread() {
        let mut pinch = 0.0;
        let steps: i32 = [1.1_f32; 8].iter().map(|factor| pinch_steps(&mut pinch, *factor)).sum();
        assert_eq!(steps, 2, "spreading 2.1 times");
        let steps: i32 = [0.9_f32; 4].iter().map(|factor| pinch_steps(&mut pinch, *factor)).sum();
        assert_eq!(steps, -1, "pinching back to 0.66 times");
        assert_eq!(pinch_steps(&mut pinch, 1.0), 0);
    }
}
