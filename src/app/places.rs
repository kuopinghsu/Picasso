//! Places: a map of geotagged photos. Click a pin to open it.
//! With Geotag on, a click stores a pin for the current selection.
//! GPS tracks from GPX and KML files are drawn under the pins.

use std::collections::HashMap;

use eframe::egui::{self, epaint::TextShape, Align2, Color32, FontId, Pos2, Rect, Sense, TextureHandle, Vec2};

use crate::geo::{self, TileJob, TILE_PX};
use crate::i18n::{tr, trf};
use crate::liberty::{LabelKind, MapLabel};
use crate::track::{format_distance, Bounds, MapTrack};
use crate::types::{day_key, folder_label, format_day, format_duration, format_month, format_timestamp, Photo};

use super::theme::{self, ACCENT, STAR, WHITE};
use super::PicassoApp;

/// Map background while tiles load.
pub(super) fn map_backdrop() -> Color32 {
    if theme::is_dark() {
        Color32::from_rgb(0x26, 0x33, 0x37)
    } else {
        Color32::from_rgb(0xD3, 0xE7, 0xE9)
    }
}

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
            ui.separator();
            if ui.checkbox(&mut self.map_starred, tr("Starred only")).on_hover_text(tr("Show only the starred photos on the map")).changed() {
                self.config.set("map_starred", self.map_starred);
                self.reload_view();
            }
        });
        ui.add_space(4.0);

        let size = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(size.max(Vec2::new(80.0, 80.0)), Sense::click_and_drag());
        self.map_size = rect.size();
        if response.dragged() {
            self.map_pan += response.drag_delta();
            self.map_card = None;
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

        ui.painter().rect_filled(rect, 0, map_backdrop());
        let view = MapView {
            lat: self.map_lat,
            lon: self.map_lon,
            zoom: self.map_zoom,
            pan: self.map_pan,
        };
        let painter = ui.painter_at(rect);
        self.paint_tiles(&painter, rect, view);
        let on_track = if self.show_tracks { self.paint_tracks(&painter, rect, view, response.hover_pos()) } else { None };
        let clusters = self.map_clusters(rect);
        let hovered = response.hover_pos().filter(|_| !response.dragged()).and_then(|pointer| hovered_cluster(&clusters, pointer));
        let hot = self.map_hot.take().and_then(|key| clusters.iter().position(|cluster| cluster.key == key));
        self.paint_clusters(&painter, &clusters, hovered.or(hot));
        self.track_hot = None;
        self.paint_selected_pins(&painter, rect);
        let on_track = on_track.filter(|_| hovered.is_none() && !response.dragged());
        self.paint_map_notes(&painter, rect, view);
        if hovered.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if let Some(track) = on_track.and_then(|index| self.tracks.get(index)) {
            response.clone().on_hover_ui_at_pointer(|ui| {
                track_tooltip(ui, track);
                let hint = tr("Click to select the photos taken along it. Double-click to zoom to it.");
                ui.label(egui::RichText::new(hint).small().color(theme::muted()));
            });
        }
        let hit = hovered.map(|index| clusters[index].members.as_slice());
        if response.double_clicked() {
            if let Some(&[index]) = hit {
                    self.open_at(index);
            } else if let Some(bounds) = on_track.and_then(|index| self.tracks.get(index)).map(|track| track.bounds) {
                self.frame_bounds(bounds);
            }
        } else if response.clicked() && !response.dragged() {
            if let Some(&[index]) = hit {
                let id = self.photos[index].id;
                self.selection.clear();
                self.selection.insert(id);
                self.anchor = Some(id);
                self.picked_track = None;
            } else if let Some(members) = hit {
                if self.map_zoom < MAX_ZOOM && self.split_closer(members) {
                    self.zoom_to_photos(members);
                } else {
                    self.select_photos(members);
                }
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
            } else if let Some(index) = on_track {
                self.select_along_track(index);
            } else {
                self.picked_track = None;
            }
        }
        self.update_map_card(ui, rect, &clusters, hovered);
        self.show_map_card(ui.ctx(), rect);
        self.map_zoom_buttons(ui, rect);
        let hover = hovered.map(|index| clusters[index].key);
        if hover != self.map_hover {
            self.map_hover = hover;
            ui.ctx().request_repaint();
        }
        self.map_view_clusters = clusters.into_iter().filter(|cluster| rect.contains(cluster.pos)).collect();

        let tracks_shown = self.show_tracks && !self.tracks.is_empty();
        let narrowed = self.map_starred || !self.search.trim().is_empty();
        if self.photos.is_empty() && self.selection.is_empty() && !self.loading && (narrowed || !tracks_shown) {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                tr(if narrowed {
                    "No photos on the map match the search or the starred filter"
                } else {
                    "No geotagged photos yet. Select photos, press Geotag, then click the map."
                }),
                FontId::proportional(14.0),
                theme::text(),
            );
        }
    }

    pub(super) fn frame_map(&mut self) {
        let chosen = self.photos.iter().filter(|photo| self.selection.contains(&photo.id)).find_map(Photo::place);
        let places: Vec<(f64, f64)> = self.photos.iter().filter_map(Photo::place).collect();
        if chosen.is_none() && !self.search.trim().is_empty() && !places.is_empty() {
            let mut bounds = Bounds::EMPTY;
            for (lat, lon) in &places {
                let (x, y) = geo::latlon_to_world(*lat, *lon, 0);
                bounds.add([x, y]);
            }
            self.frame_bounds(bounds);
            self.map_zoom = self.map_zoom.min(14);
        } else if let Some((lat, lon)) = chosen.or_else(|| places.first().copied()) {
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
    pub(super) fn frame_bounds(&mut self, bounds: Bounds) {
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
        let picked = self.picked_track.and_then(|id| self.tracks.iter().position(|track| track.id == id));
        let lit = hovered.or(self.track_hot).or(picked);
        drawn.sort_by_key(|(index, _)| Some(*index) == lit);
        for (index, runs) in &drawn {
            let color = track_color(self.tracks[*index].id);
            let wide = Some(*index) == lit;
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

    /// Opens Places to show something chosen elsewhere: a photo, an area, or
    /// a track. The search and **Starred only** go off, or they could hide
    /// what was asked for.
    pub(super) fn open_places_to_show(&mut self) {
        self.search.clear();
        self.search_due = None;
        if self.map_starred {
            self.map_starred = false;
            self.config.set("map_starred", false);
        }
        self.set_nav(crate::types::Nav::Places);
    }

    pub(super) fn show_on_map(&mut self, id: i64) {
        let place = self.db.photo(id).and_then(|photo| photo.place());
        self.selection.clear();
        self.selection.insert(id);
        self.open_places_to_show();
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
        let next = (self.map_zoom as i32 + steps).clamp(i32::from(MIN_ZOOM), i32::from(MAX_ZOOM)) as u8;
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
                // A tile the source doesn't have, such as imagery closer
                // than a place was photographed from the air, is drawn as
                // the enlarged part of the closest tile above it that it has.
                let Some(depth) = (0..=MAX_ENLARGE.min(z)).find(|depth| !self.tile_fail.contains(&covering(z, x, y, *depth)))
                else {
                    continue;
                };
                let key = covering(z, x, y, depth);
                let Some(loaded) = self.tiles.get(&key) else {
                    if !wanted.contains(&key) {
                        wanted.push(key);
                    }
                    continue;
                };
                let cells = (1u32 << depth) as f32;
                let mask = (1u32 << depth) - 1;
                let min = egui::pos2((x & mask) as f32 / cells, (y & mask) as f32 / cells);
                painter.image(loaded.texture.id(), tile, Rect::from_min_size(min, Vec2::splat(1.0 / cells)), Color32::WHITE);
                if depth == 0 {
                    labels.extend(
                        loaded.labels.iter().map(|label| (tile.min + Vec2::from(label.pos) * TILE_PX as f32, label)),
                    );
                }
            }
        }
        paint_labels(painter, rect, labels);
        for key in wanted.into_iter().take(8) {
            if self.tile_wait.contains(&key) {
                continue;
            }
            self.tile_wait.insert(key);
            let (z, x, y) = key;
            let _ = self.tile_tx.send(TileJob { source, key: self.carto_key.clone(), z, x, y });
        }
    }

    fn map_key_missing(&self) -> bool {
        self.map_source.needs_key() && self.carto_key.is_empty()
    }

    /// The provider's credit in the bottom-right corner of the map in
    /// `rect`, a scale bar in the bottom-left one, and a note in the middle
    /// when the source needs a key.
    pub(super) fn paint_map_notes(&self, painter: &egui::Painter, rect: Rect, view: MapView) {
        let ink = Color32::from_gray(60);
        if self.map_key_missing() {
            let galley = painter.layout(
                trf("{} needs an API key. Add it in Settings → Map.", &[&self.map_source.name()]),
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
        let credit = Rect::from_min_size(at, galley.size()).expand(2.0);
        painter.rect_filled(credit, 2, Color32::from_white_alpha(190));
        painter.galley(at, galley, ink);
        self.paint_scale_bar(painter, rect, view, credit);
    }

    /// A round distance about a hundred points long at the latitude in the
    /// middle of the map, above `credit` when they would overlap.
    fn paint_scale_bar(&self, painter: &egui::Painter, rect: Rect, view: MapView, credit: Rect) {
        let (cx, cy) = geo::latlon_to_world(view.lat, view.lon, view.zoom);
        let (lat, _) = geo::world_to_latlon(cx - f64::from(view.pan.x), cy - f64::from(view.pan.y), view.zoom);
        let (meters, width) = scale_bar(geo::meters_per_point(lat, view.zoom), 100.0);
        let label = if meters >= 1000.0 { format!("{} km", meters / 1000.0) } else { format!("{meters} m") };
        let ink = Color32::from_gray(50);
        let galley = painter.layout_no_wrap(label, FontId::proportional(10.5), ink);
        let size = Vec2::new(width.max(galley.size().x) + 8.0, galley.size().y + 11.0);
        let mut back = Rect::from_min_size(rect.left_bottom() + Vec2::new(6.0, -size.y - 4.0), size);
        if back.intersects(credit) {
            back = back.translate(Vec2::new(0.0, credit.top() - 4.0 - back.bottom()));
        }
        painter.rect_filled(back, 3, Color32::from_white_alpha(190));
        let base = back.left_bottom() + Vec2::new(4.0, -4.0);
        let stroke = egui::Stroke::new(1.5, ink);
        painter.line(vec![base - Vec2::new(0.0, 5.0), base, base + Vec2::new(width, 0.0), base + Vec2::new(width, -5.0)], stroke);
        painter.galley(back.left_top() + Vec2::new(4.0, 2.0), galley, ink);
    }

    /// Forgets loaded and failed tiles, after the map source or its key changes.
    pub(super) fn reset_tiles(&mut self) {
        self.tiles.clear();
        self.tile_wait.clear();
        self.tile_fail.clear();
    }

    /// The geotagged photos in and around `rect`, grouped into clusters,
    /// biggest first.
    fn map_clusters(&self, rect: Rect) -> Vec<Cluster> {
        let (cx, cy) = geo::latlon_to_world(self.map_lat, self.map_lon, self.map_zoom);
        let middle = rect.center() + self.map_pan;
        let area = rect.expand(CLUSTER_CELL as f32);
        let mut indices = Vec::new();
        let mut points = Vec::new();
        for (index, photo) in self.photos.iter().enumerate() {
            let Some((lat, lon)) = photo.place() else { continue };
            let (x, y) = geo::latlon_to_world(lat, lon, self.map_zoom);
            if area.contains(middle + Vec2::new((x - cx) as f32, (y - cy) as f32)) {
                indices.push(index);
                points.push([x, y]);
            }
        }
        cluster_points(&points, CLUSTER_CELL)
            .into_iter()
            .map(|(members, [x, y])| {
                let members: Vec<usize> = members.into_iter().map(|member| indices[member]).collect();
                Cluster {
                    pos: middle + Vec2::new((x - cx) as f32, (y - cy) as f32),
                    key: cluster_key(members.iter().map(|member| self.photos[*member].id)),
                    members,
                }
            })
            .collect()
    }

    /// Draws one dot per photo on its own and one circle, sized by the
    /// count, per cluster. Smaller clusters go on top.
    fn paint_clusters(&self, painter: &egui::Painter, clusters: &[Cluster], hovered: Option<usize>) {
        for (index, cluster) in clusters.iter().enumerate() {
            let lit = Some(index) == hovered;
            let count = cluster.members.len();
            let pinned = cluster.members.iter().all(|member| self.photos[*member].pinned());
            let color = if pinned { STAR } else { ACCENT };
            if count == 1 {
                let grow = if lit { 2.0 } else { 0.0 };
                painter.circle_filled(cluster.pos, 5.0 + grow, color);
                painter.circle_stroke(cluster.pos, 6.0 + grow, egui::Stroke::new(1.5, WHITE));
                continue;
            }
            let radius = cluster_radius(count) + if lit { 2.0 } else { 0.0 };
            painter.circle_filled(cluster.pos + Vec2::new(0.0, 1.5), radius + 1.0, Color32::from_black_alpha(50));
            painter.circle_filled(cluster.pos, radius, color.gamma_multiply(if lit { 1.0 } else { 0.85 }));
            painter.circle_stroke(cluster.pos, radius, egui::Stroke::new(2.0, Color32::from_white_alpha(230)));
            let size = if count >= 1000 { 11.0 } else { 12.5 };
            painter.text(cluster.pos, Align2::CENTER_CENTER, short_count(count), FontId::proportional(size), WHITE);
        }
    }

    /// Selects the photos on the map taken along track `index`, and lights
    /// the track.
    fn select_along_track(&mut self, index: usize) {
        let Some(track) = self.tracks.get(index) else { return };
        let ids: Vec<i64> = self
            .photos
            .iter()
            .filter(|photo| photo.place().is_some_and(|(lat, lon)| track.passes(lat, lon, photo.taken_at)))
            .map(|photo| photo.id)
            .collect();
        self.activity = match ids.len() {
            0 => trf("No photos on the map were taken along {}", &[&track.name]),
            1 => trf("{} photo taken along {}", &[&1, &track.name]),
            count => trf("{} photos taken along {}", &[&count, &track.name]),
        };
        self.picked_track = Some(track.id);
        self.anchor = ids.first().copied();
        self.selection = ids.into_iter().collect();
        self.map_card = None;
    }

    fn select_photos(&mut self, indices: &[usize]) {
        self.picked_track = None;
        self.selection = indices.iter().map(|index| self.photos[*index].id).collect();
        self.anchor = indices.first().map(|index| self.photos[*index].id);
    }

    /// False when the photos at `indices` lie so close together that they
    /// would still form one cluster at the closest zoom.
    fn split_closer(&self, indices: &[usize]) -> bool {
        let mut bounds = Bounds::EMPTY;
        for (lat, lon) in indices.iter().filter_map(|index| self.photos[*index].place()) {
            let (x, y) = geo::latlon_to_world(lat, lon, MAX_ZOOM);
            bounds.add([x, y]);
        }
        let [w, h] = bounds.size();
        w.max(h) >= CLUSTER_CELL
    }

    /// Frames the photos at `indices`, at least two levels closer than now.
    fn zoom_to_photos(&mut self, indices: &[usize]) {
        let mut bounds = Bounds::EMPTY;
        for (lat, lon) in indices.iter().filter_map(|index| self.photos[*index].place()) {
            let (x, y) = geo::latlon_to_world(lat, lon, 0);
            bounds.add([x, y]);
        }
        let before = self.map_zoom;
        self.frame_bounds(bounds);
        if self.map_zoom <= before {
            self.map_zoom = (before + 2).min(MAX_ZOOM);
        }
        self.map_card = None;
    }

    fn latlon_to_screen(&self, lat: f64, lon: f64, rect: Rect) -> Pos2 {
        let (cx, cy) = geo::latlon_to_world(self.map_lat, self.map_lon, self.map_zoom);
        let (x, y) = geo::latlon_to_world(lat, lon, self.map_zoom);
        rect.center() + self.map_pan + Vec2::new((x - cx) as f32, (y - cy) as f32)
    }

    /// Opens the card of the cluster under the pointer, and closes it a
    /// moment after the pointer leaves both, so it can reach the card.
    fn update_map_card(&mut self, ui: &egui::Ui, rect: Rect, clusters: &[Cluster], hovered: Option<usize>) {
        if self.map_card.as_ref().is_some_and(|card| card.zoom != self.map_zoom) {
            self.map_card = None;
        }
        let now = ui.input(|input| input.time);
        if let Some(cluster) = hovered.map(|index| &clusters[index]) {
            let key = cluster.key;
            if let Some(card) = self.map_card.as_mut().filter(|card| card.key == key) {
                card.left = None;
            } else {
                let (lat, lon) = self.screen_to_latlon(cluster.pos, rect);
                let count = cluster.members.len();
                self.map_card = Some(MapCard {
                    ids: cluster.members.iter().map(|member| self.photos[*member].id).collect(),
                    key,
                    lat,
                    lon,
                    zoom: self.map_zoom,
                    reach: if count == 1 { 7.0 } else { cluster_radius(count) + 2.0 },
                    rect: Rect::NOTHING,
                    left: None,
                });
            }
        } else if let Some(card) = self.map_card.as_mut() {
        let pointer = ui.input(|input| input.pointer.hover_pos());
            if pointer.is_some_and(|pointer| card.rect.expand(6.0).contains(pointer)) {
                card.left = None;
            } else {
                let left = *card.left.get_or_insert(now);
                let remaining = CARD_GRACE - (now - left);
                if remaining <= 0.0 {
                    self.map_card = None;
                } else {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(remaining));
                }
            }
        }
    }

    /// Draws the card over its cluster: when the photos were taken, their
    /// folder, how many there are, and a few thumbnails to open.
    fn show_map_card(&mut self, ctx: &egui::Context, rect: Rect) {
        const WIDTH: f32 = 288.0;
        const GAP: f32 = 4.0;
        const SHOWN: usize = 6;
        let Some(card) = self.map_card.as_ref() else { return };
        let (lat, lon, reach, height) = (card.lat, card.lon, card.reach, card.rect.height());
        let mut indices: Vec<usize> = card.ids.iter().filter_map(|id| self.index_of(*id)).collect();
        if indices.is_empty() {
            self.map_card = None;
            return;
        }
        indices.sort_by_key(|index| (self.photos[*index].taken_at, *index));
        let count = indices.len();
        let first = &self.photos[indices[0]];
        let (title, detail) = if count == 1 {
            (format_timestamp(first.taken_at), first.filename.clone())
        } else {
            (
                date_span(indices.iter().map(|index| self.photos[*index].taken_at)),
                folder_summary(indices.iter().map(|index| self.photos[*index].path.as_str())),
            )
        };
        let picks = spread(&indices, SHOWN);
        let mut tiles = Vec::with_capacity(picks.len());
        for index in &picks {
            let (id, path, rotation) = {
                let photo = &self.photos[*index];
                (photo.id, photo.path.clone(), photo.rotation)
            };
            self.ensure_thumb(id, &path);
            tiles.push((*index, id, rotation, self.thumb_texture(id)));
        }

        let anchor = self.latlon_to_screen(lat, lon, rect);
        let height = if height > 0.0 { height } else { 260.0 };
        let above = anchor.y - reach - 8.0 - height >= rect.top();
        let (pivot, at) = if above {
            (Align2::CENTER_BOTTOM, anchor - Vec2::new(0.0, reach + 8.0))
        } else {
            (Align2::CENTER_TOP, anchor + Vec2::new(0.0, reach + 8.0))
        };
        let mut action = None;
        let area = egui::Area::new(egui::Id::unique("map-card"))
            .order(egui::Order::Foreground)
            .pivot(pivot)
            .fixed_pos(at)
            .constrain_to(rect.shrink(6.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(12.0).corner_radius(10.0).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&title).strong().size(14.0).color(theme::text()));
                        if count > 1 {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(trf("{} photos", &[&grouped(count)])).color(theme::muted()));
                            });
                        }
                    });
                    ui.label(egui::RichText::new(&detail).size(12.0).color(theme::muted()));
                    ui.add_space(6.0);

                    let columns = if count == 1 { 1 } else { 3 };
                    let tile_w = (WIDTH - GAP * (columns - 1) as f32) / columns as f32;
                    let tile_h = if count == 1 { WIDTH * 2.0 / 3.0 } else { tile_w * 0.75 };
                    let rows = tiles.len().div_ceil(columns);
                    let grid_h = rows as f32 * tile_h + rows.saturating_sub(1) as f32 * GAP;
                    let (grid, _) = ui.allocate_exact_size(Vec2::new(WIDTH, grid_h), Sense::hover());
                    for (n, (index, id, rotation, texture)) in tiles.iter().enumerate() {
                        let offset = Vec2::new((n % columns) as f32 * (tile_w + GAP), (n / columns) as f32 * (tile_h + GAP));
                        let tile = Rect::from_min_size(grid.min + offset, Vec2::new(tile_w, tile_h));
                        let response = ui
                            .interact(tile, egui::Id::unique(("map-card-photo", *id)), Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        let painter = ui.painter();
                        painter.rect_filled(tile, 4, theme::placeholder());
                        if let Some((texture, w, h)) = texture {
                            let uv = cover_uv_sized(*w, *h, *rotation, tile.size());
                            theme::paint_image_rounded(painter, texture, tile, uv, *rotation, 4.0);
                        }
                        let hidden = count - tiles.len();
                        if hidden > 0 && n + 1 == tiles.len() {
                            painter.rect_filled(tile, 4, Color32::from_black_alpha(130));
                            painter.text(tile.center(), Align2::CENTER_CENTER, format!("+{}", grouped(hidden + 1)), FontId::proportional(15.0), WHITE);
                        }
                        if response.hovered() {
                            painter.rect_stroke(tile, 4, egui::Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
                        }
                        if response.clicked() {
                            action = Some(CardAction::Open(*index));
                        }
                    }

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if count == 1 {
                            if ui.button(tr("Open")).clicked() {
                                action = Some(CardAction::Open(indices[0]));
                            }
                            return;
                        }
                        if ui.button(tr("Select these photos")).clicked() {
                            action = Some(CardAction::Select);
                        }
                        if ui.button(tr("Zoom in")).clicked() {
                            action = Some(CardAction::Zoom);
                        }
                    });
                });
            });
        if let Some(card) = self.map_card.as_mut() {
            card.rect = area.response.rect;
        }
        match action {
            Some(CardAction::Open(index)) => {
                self.map_card = None;
                self.open_at(index);
            }
            Some(CardAction::Select) => self.select_photos(&indices),
            Some(CardAction::Zoom) => self.zoom_to_photos(&indices),
            None => {}
        }
    }

    /// The right-hand panel on Places: what the map shows, or the details
    /// of the selected photo with a way back.
    pub(super) fn places_side_ui(&mut self, ui: &mut egui::Ui) {
        let photo = if self.selection.is_empty() { None } else { self.side_photo() };
        ui.horizontal(|ui| {
            if photo.is_some() {
                let text = egui::RichText::new(format!("‹  {}", tr("In this area"))).color(theme::accent());
                if ui.add(egui::Button::new(text).frame(false)).clicked() {
                    self.selection.clear();
                    self.anchor = None;
                }
            } else {
                ui.label(egui::RichText::new(tr("In this area")).size(15.0).strong().color(theme::text()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = egui::Button::new(egui::RichText::new("×").size(18.0).color(theme::muted())).frame(false);
                if ui.add(close).on_hover_text(tr("Close panel")).clicked() {
                    self.toggle_side(self.side);
                }
            });
        });
        ui.add_space(6.0);
        let Some(photo) = photo else {
            self.area_list(ui);
            return;
        };
        if self.selection.len() > 1 {
            let text = trf("{} photos selected · showing one", &[&self.selection.len()]);
            ui.label(egui::RichText::new(text).size(12.0).color(theme::muted()));
            ui.add_space(4.0);
        }
        egui::ScrollArea::vertical()
            .id_salt("places-photo")
            .auto_shrink([false, false])
            .show(ui, |ui| self.side_info(ui, &photo));
    }

    /// The clusters and tracks on the map, biggest first, each lit on the
    /// map while the pointer is over it.
    fn area_list(&mut self, ui: &mut egui::Ui) {
        let count = self.photos.len();
        let groups: Vec<(ClusterKey, Vec<usize>)> = self
            .map_view_clusters
            .iter()
            .filter(|cluster| cluster.members.iter().all(|member| *member < count))
            .map(|cluster| {
                let mut members = cluster.members.clone();
                members.sort_by_key(|member| (self.photos[*member].taken_at, *member));
                (cluster.key, members)
            })
            .collect();
        let tracks: Vec<usize> = if self.show_tracks {
            let view = self.view_bounds();
            let seen = |bounds: &Bounds| bounds.min[0] <= view.max[0] && bounds.max[0] >= view.min[0] && bounds.min[1] <= view.max[1] && bounds.max[1] >= view.min[1];
            (0..self.tracks.len()).filter(|index| seen(&self.tracks[*index].bounds)).collect()
        } else {
            Vec::new()
        };
        if groups.is_empty() && tracks.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(tr("No photos in this area")).color(theme::text()));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(tr("Zoom out or move the map to find your photos.")).size(12.0).color(theme::muted()));
            });
            return;
        }
        let total: usize = groups.iter().map(|(_, members)| members.len()).sum();
        if total > 0 {
            let photos = trf(if total == 1 { "{} photo" } else { "{} photos" }, &[&grouped(total)]);
            let places = trf(if groups.len() == 1 { "{} place" } else { "{} places" }, &[&groups.len()]);
            ui.label(egui::RichText::new(format!("{photos} · {places}")).size(12.0).color(theme::muted()));
            let span = date_span(groups.iter().flat_map(|(_, members)| members.iter()).map(|member| self.photos[*member].taken_at));
            ui.label(egui::RichText::new(span).size(12.0).color(theme::muted()));
        }
        ui.add_space(6.0);

        let mut action = None;
        egui::ScrollArea::vertical().id_salt("places-area").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (key, members) in &groups {
                self.area_row(ui, *key, members, &mut action);
            }
            if !tracks.is_empty() {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(tr("Tracks")).size(12.0).strong().color(theme::muted()));
                ui.add_space(2.0);
                for index in &tracks {
                    self.track_row(ui, *index, &mut action);
                }
            }
        });
        match action {
            Some(AreaAction::Zoom(members)) => self.zoom_to_photos(&members),
            Some(AreaAction::Open(index)) => self.open_at(index),
            Some(AreaAction::Track(index)) => {
                if let Some(bounds) = self.tracks.get(index).map(|track| track.bounds) {
                    self.frame_bounds(bounds);
                    self.map_card = None;
                }
            }
            None => {}
        }
    }

    /// One cluster: when, where, how many, and a strip of thumbnails.
    /// Click it to zoom the map to it, or a thumbnail to open that photo.
    fn area_row(&mut self, ui: &mut egui::Ui, key: ClusterKey, members: &[usize], action: &mut Option<AreaAction>) {
        const THUMBS: usize = 5;
        const GAP: f32 = 4.0;
        let width = ui.available_width();
        let thumb = ((width - 16.0 - GAP * (THUMBS - 1) as f32) / THUMBS as f32).floor().max(16.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 56.0 + thumb), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let over = ui.rect_contains_pointer(rect);
        if over {
            self.map_hot = Some(key);
        }
        if over || self.map_hover == Some(key) {
            ui.painter().rect_filled(rect, 8, theme::select());
        }
        let inner = rect.shrink(8.0);
        let count = members.len();
        let first = &self.photos[members[0]];
        let (title, detail) = if count == 1 {
            (format_timestamp(first.taken_at), first.filename.clone())
        } else {
            (
                date_span(members.iter().map(|member| self.photos[*member].taken_at)),
                folder_summary(members.iter().map(|member| self.photos[*member].path.as_str())),
            )
        };
        let painter = ui.painter().clone();
        let number = painter.layout_no_wrap(if count > 1 { grouped(count) } else { String::new() }, FontId::proportional(12.0), theme::muted());
        let title = truncated(&painter, title, 13.0, theme::text(), inner.width() - number.size().x - 8.0);
        let detail = truncated(&painter, detail, 11.5, theme::muted(), inner.width());
        painter.galley(Pos2::new(inner.right() - number.size().x, inner.top() + 1.0), number, theme::muted());
        painter.galley(inner.min, title, theme::text());
        painter.galley(inner.min + Vec2::new(0.0, 19.0), detail, theme::muted());

        for (n, index) in spread(members, THUMBS).into_iter().enumerate() {
            let tile = Rect::from_min_size(Pos2::new(inner.left() + n as f32 * (thumb + GAP), inner.top() + 40.0), Vec2::splat(thumb));
            let (id, path, rotation) = {
                let photo = &self.photos[index];
                (photo.id, photo.path.clone(), photo.rotation)
            };
            self.ensure_thumb(id, &path);
            let tile_response = ui
                .interact(tile, egui::Id::unique(("area-photo", id)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            painter.rect_filled(tile, 4, theme::placeholder());
            if let Some((texture, w, h)) = self.thumb_texture(id) {
                theme::paint_image_rounded(&painter, &texture, tile, theme::cover_uv(w, h), rotation, 4.0);
            }
            if tile_response.hovered() {
                painter.rect_stroke(tile, 4, egui::Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
            }
            if tile_response.clicked() {
                *action = Some(AreaAction::Open(index));
            }
        }
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            *action = Some(AreaAction::Zoom(members.to_vec()));
        }
    }

    /// One GPS track: its color, name, date, length, and time. Click it to
    /// zoom the map to it.
    fn track_row(&mut self, ui: &mut egui::Ui, index: usize, action: &mut Option<AreaAction>) {
        let (name, color, detail) = {
            let track = &self.tracks[index];
            let mut parts = Vec::new();
            if let Some((start, _)) = track.time_span {
                parts.push(format_day(start));
            }
            parts.push(format_distance(track.meters));
            if let Some((start, end)) = track.time_span {
                parts.push(format_duration((end - start) as f64));
            }
            (track.name.clone(), track_color(track.id), parts.join(" · "))
        };
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 44.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let painter = ui.painter();
        if ui.rect_contains_pointer(rect) {
            self.track_hot = Some(index);
            painter.rect_filled(rect, 8, theme::select());
        }
        let mid = rect.left() + 15.0;
        painter.line_segment([Pos2::new(mid - 7.0, rect.center().y), Pos2::new(mid + 7.0, rect.center().y)], egui::Stroke::new(4.0, color));
        let text_left = rect.left() + 30.0;
        let room = rect.right() - 8.0 - text_left;
        painter.galley(Pos2::new(text_left, rect.top() + 6.0), truncated(painter, name, 13.0, theme::text(), room), theme::text());
        painter.galley(Pos2::new(text_left, rect.top() + 24.0), truncated(painter, detail, 11.5, theme::muted(), room), theme::muted());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            *action = Some(AreaAction::Track(index));
        }
        response.context_menu(|ui| {
            if ui.button(tr("Select the photos taken along it")).clicked() {
                self.select_along_track(index);
                ui.close();
            }
            if ui.button(tr("Edit track")).clicked() {
                let path = self.tracks[index].path.clone();
                self.open_track(&path, ui.ctx());
                ui.close();
            }
        });
    }

    /// The part of the world the Places map shows, in world pixels at zoom 0.
    fn view_bounds(&self) -> Bounds {
        let scale = f64::from(1u32 << self.map_zoom);
        let (cx, cy) = geo::latlon_to_world(self.map_lat, self.map_lon, self.map_zoom);
        let (cx, cy) = (cx - f64::from(self.map_pan.x), cy - f64::from(self.map_pan.y));
        let (half_w, half_h) = (f64::from(self.map_size.x) / 2.0, f64::from(self.map_size.y) / 2.0);
        Bounds { min: [(cx - half_w) / scale, (cy - half_h) / scale], max: [(cx + half_w) / scale, (cy + half_h) / scale] }
    }

    /// The zoom control in the top-left corner of the map.
    fn map_zoom_buttons(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let corner = rect.left_top() + Vec2::new(10.0, 10.0);
        let step = zoom_step(rect.height() - 40.0);
        if let Some(next) = zoom_control(ui, corner, self.map_zoom, step, "places-zoom") {
            self.zoom_map_at(i32::from(next) - i32::from(self.map_zoom), rect.center(), rect);
            self.map_card = None;
            ui.ctx().request_repaint();
        }
    }

    /// Selected photos get a bigger pin over the clusters. Names are written
    /// only for a few, since a large selection would bury the map in text.
    fn paint_selected_pins(&self, painter: &egui::Painter, rect: Rect) {
        if self.selection.is_empty() {
            return;
        }
        let (cx, cy) = geo::latlon_to_world(self.map_lat, self.map_lon, self.map_zoom);
        let middle = rect.center() + self.map_pan;
        let mut chosen = Vec::new();
        for photo in self.photos.iter().filter(|photo| self.selection.contains(&photo.id)) {
            let Some((lat, lon)) = photo.place() else { continue };
            let (x, y) = geo::latlon_to_world(lat, lon, self.map_zoom);
            let pos = middle + Vec2::new((x - cx) as f32, (y - cy) as f32);
            if rect.contains(pos) {
                chosen.push((pos, photo));
            }
        }
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

/// Closest zoom level of the Places map and the Map panel, where a hundred
/// points cover about 30 meters.
pub(super) const MAX_ZOOM: u8 = 19;

/// Farthest zoom level of the Places map and the Map panel.
pub(super) const MIN_ZOOM: u8 = 2;

/// Side of the zoom buttons.
pub(super) const ZOOM_BUTTON: f32 = 28.0;
/// Space above and below the ends of the zoom bar.
const ZOOM_BAR_PAD: f32 = 7.0;
/// Seconds the zoom control stays at full strength after the zoom changes.
const ZOOM_LINGER: f64 = 1.2;

/// Points per zoom level on the zoom bar, for a control at most `room`
/// points tall.
pub(super) fn zoom_step(room: f32) -> f32 {
    let levels = f32::from(MAX_ZOOM - MIN_ZOOM);
    ((room - 2.0 * (ZOOM_BUTTON + 4.0) - 2.0 * ZOOM_BAR_PAD) / levels).clamp(3.0, 8.0)
}

/// A + button, a bar with one stop per zoom level, and a − button, from
/// `top_left` down. Clicking or dragging along the bar goes straight to a
/// level. Returns the level picked, if it differs from `zoom`.
pub(super) fn zoom_control(ui: &mut egui::Ui, top_left: Pos2, zoom: u8, step: f32, id: &str) -> Option<u8> {
    let levels = f32::from(MAX_ZOOM - MIN_ZOOM);
    let height = 2.0 * (ZOOM_BUTTON + 4.0) + levels * step + 2.0 * ZOOM_BAR_PAD;
    let area = Rect::from_min_size(top_left, Vec2::new(ZOOM_BUTTON, height));
    // Faded into the map until the pointer comes near, the bar is dragged,
    // or the zoom changes some other way, such as a pinch or the wheel.
    let now = ui.input(|input| input.time);
    let memory = egui::Id::unique((id, "zoomed"));
    let (seen, changed_at) = ui.ctx().data(|data| data.get_temp::<(u8, f64)>(memory)).unwrap_or((zoom, f64::NEG_INFINITY));
    let changed_at = if seen == zoom { changed_at } else { now };
    ui.ctx().data_mut(|data| data.insert_temp(memory, (zoom, changed_at)));
    let zooming = now - changed_at < ZOOM_LINGER;
    if zooming {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(ZOOM_LINGER - (now - changed_at)));
    }
    let active = zooming || ui.rect_contains_pointer(area.expand(8.0)) || ui.ctx().is_being_dragged(egui::Id::unique(id));
    let presence = ui.ctx().animate_bool_with_time(egui::Id::unique((id, "fade")), active, 0.15);
    // Over the map, without moving what the map's parent lays out after it.
    let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(area));
    ui.set_opacity(0.5 + 0.5 * presence);
    let button = |ui: &mut egui::Ui, at: Pos2, label: &str, tip: &'static str| {
        let text = egui::RichText::new(label).size(16.0);
        ui.put(Rect::from_min_size(at, Vec2::splat(ZOOM_BUTTON)), egui::Button::new(text).fill(theme::bg()))
            .on_hover_text(tr(tip))
            .clicked()
    };
    let mut next = None;
    if button(ui, top_left, "+", "Zoom in") {
        next = Some(zoom.saturating_add(1));
    }
    let bar = Rect::from_min_size(top_left + Vec2::new(0.0, ZOOM_BUTTON + 4.0), Vec2::new(ZOOM_BUTTON, levels * step + 2.0 * ZOOM_BAR_PAD));
    if button(ui, bar.left_bottom() + Vec2::new(0.0, 4.0), "−", "Zoom out") {
        next = Some(zoom.saturating_sub(1));
    }
    let bottom = bar.bottom() - ZOOM_BAR_PAD;
    let response = ui.interact(bar, egui::Id::unique(id), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() || response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            next = Some(MIN_ZOOM + ((bottom - pointer.y) / step).round().clamp(0.0, levels) as u8);
        }
    }

    let painter = ui.painter_at(bar.expand(2.0));
    painter.rect(bar, 6, theme::bg(), egui::Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
    let x = bar.center().x;
    let shown = next.unwrap_or(zoom).clamp(MIN_ZOOM, MAX_ZOOM);
    let knob = Pos2::new(x, bottom - f32::from(shown - MIN_ZOOM) * step);
    if step >= 5.0 {
        for level in 0..=MAX_ZOOM - MIN_ZOOM {
            let y = bottom - f32::from(level) * step;
            painter.line_segment([Pos2::new(x - 4.0, y), Pos2::new(x + 4.0, y)], egui::Stroke::new(1.0, theme::hairline()));
        }
    }
    painter.line_segment([Pos2::new(x, bar.top() + ZOOM_BAR_PAD), Pos2::new(x, bottom)], egui::Stroke::new(3.0, theme::hairline()));
    painter.line_segment([knob, Pos2::new(x, bottom)], egui::Stroke::new(3.0, theme::accent()));
    let lit = response.hovered() || response.dragged();
    painter.circle(knob, if lit { 6.5 } else { 5.5 }, theme::accent(), egui::Stroke::new(2.0, Color32::WHITE));
    next.map(|level| level.clamp(MIN_ZOOM, MAX_ZOOM)).filter(|level| *level != zoom)
}

/// Levels a tile may be enlarged by to stand in for a closer one the
/// source doesn't have.
const MAX_ENLARGE: u8 = 4;

/// The tile `depth` levels above tile `x`, `y` at zoom `z` that covers it.
fn covering(z: u8, x: u32, y: u32, depth: u8) -> (u8, u32, u32) {
    (z - depth, x >> depth, y >> depth)
}

/// The longest round distance (1, 2 or 5 times a power of ten meters) that
/// fits in `room` points, and its length in points.
fn scale_bar(meters_per_point: f64, room: f32) -> (f64, f32) {
    let most = meters_per_point * f64::from(room);
    let power = 10f64.powf(most.log10().floor());
    let meters = [5.0, 2.0, 1.0].into_iter().map(|step| step * power).find(|meters| *meters <= most).unwrap_or(power);
    (meters, (meters / meters_per_point) as f32)
}

/// Side, in screen points, of the squares photos are grouped by.
const CLUSTER_CELL: f64 = 64.0;

/// Seconds the card stays after the pointer leaves it and its cluster.
const CARD_GRACE: f64 = 0.4;

/// Tells clusters apart from frame to frame: count and lowest and highest id.
pub(super) type ClusterKey = (usize, i64, i64);

/// Photos close together on the map, drawn as one circle.
pub(super) struct Cluster {
    pos: Pos2,
    /// Indices into the photos.
    members: Vec<usize>,
    key: ClusterKey,
}

/// The card of thumbnails over a cluster.
pub(super) struct MapCard {
    ids: Vec<i64>,
    key: ClusterKey,
    lat: f64,
    lon: f64,
    zoom: u8,
    /// Distance from the cluster's middle to keep clear.
    reach: f32,
    /// Where the card was drawn last frame.
    rect: Rect,
    /// When the pointer left the card and the cluster.
    left: Option<f64>,
}

enum CardAction {
    Open(usize),
    Select,
    Zoom,
}

enum AreaAction {
    Zoom(Vec<usize>),
    Open(usize),
    Track(usize),
}

/// `text` on one line, cut short with an ellipsis past `width`.
pub(super) fn truncated(painter: &egui::Painter, text: String, size: f32, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    painter.layout_job(job)
}

fn cluster_key(ids: impl Iterator<Item = i64>) -> ClusterKey {
    ids.fold((0, i64::MAX, i64::MIN), |(count, low, high), id| (count + 1, low.min(id), high.max(id)))
}

/// Radius of a cluster's circle, growing with the log of its count.
fn cluster_radius(count: usize) -> f32 {
    if count <= 1 {
        6.0
    } else {
        (8.0 + 3.2 * (count as f32).log2()).min(32.0)
    }
}

/// Groups `points`, in world pixels, by the `cell`-sized square each falls
/// in, then merges groups whose circles would overlap. The squares are fixed
/// to the world, so panning doesn't regroup. Returns each group's members and
/// middle, biggest first.
pub(super) fn cluster_points(points: &[[f64; 2]], cell: f64) -> Vec<(Vec<usize>, [f64; 2])> {
    let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (index, [x, y]) in points.iter().enumerate() {
        cells.entry(((x / cell).floor() as i64, (y / cell).floor() as i64)).or_default().push(index);
    }
    let mut groups: Vec<(Vec<usize>, [f64; 2])> = cells
        .into_values()
        .map(|members| {
            let n = members.len() as f64;
            let (x, y) = members.iter().fold((0.0, 0.0), |(x, y), member| (x + points[*member][0], y + points[*member][1]));
            (members, [x / n, y / n])
        })
        .collect();
    groups.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0[0].cmp(&b.0[0])));
    let mut kept: Vec<(Vec<usize>, [f64; 2])> = Vec::new();
    for (members, at) in groups {
        let reach = f64::from(cluster_radius(members.len()));
        let near = kept.iter().position(|(other, middle)| {
            let (dx, dy) = (middle[0] - at[0], middle[1] - at[1]);
            (dx * dx + dy * dy).sqrt() < reach + f64::from(cluster_radius(other.len()))
        });
        match near {
            Some(index) => {
                let (other, middle) = &mut kept[index];
                let (a, b) = (other.len() as f64, members.len() as f64);
                *middle = [(middle[0] * a + at[0] * b) / (a + b), (middle[1] * a + at[1] * b) / (a + b)];
                other.extend(members);
            }
            None => kept.push((members, at)),
        }
    }
    kept.sort_by_key(|k| std::cmp::Reverse(k.0.len()));
    kept
}

/// The cluster under `pointer`, taking the ones drawn on top first.
fn hovered_cluster(clusters: &[Cluster], pointer: Pos2) -> Option<usize> {
    (0..clusters.len()).rev().find(|index| {
        let cluster = &clusters[*index];
        pointer.distance(cluster.pos) <= cluster_radius(cluster.members.len()) + 3.0
    })
}

/// A count short enough for a circle: 999, 1.2k, 12k.
fn short_count(count: usize) -> String {
    match count {
        0..1000 => count.to_string(),
        1000..10_000 => format!("{:.1}k", count as f32 / 1000.0),
        _ => format!("{}k", count / 1000),
    }
}

/// `count` with its thousands set apart: 3,185.
pub(super) fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (n, digit) in digits.chars().enumerate() {
        if n > 0 && (digits.len() - n).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The day the photos were taken, or the months they span.
pub(super) fn date_span(times: impl Iterator<Item = i64>) -> String {
    use chrono::Datelike;
    let (first, last) = times.fold((i64::MAX, i64::MIN), |(first, last), time| (first.min(time), last.max(time)));
    if first > last {
        return String::new();
    }
    if day_key(first) == day_key(last) {
        return format_day(first);
    }
    let month = |time: i64| {
        chrono::DateTime::from_timestamp(time, 0).map(|dt| {
            let dt = dt.with_timezone(&chrono::Local);
            (dt.year(), dt.month())
        })
    };
    match (month(first), month(last)) {
        (Some(a), Some(b)) if a == b => format_month(a.0, a.1),
        (Some(a), Some(b)) => format!("{} – {}", format_month(a.0, a.1), format_month(b.0, b.1)),
        _ => tr("Unknown date").to_string(),
    }
}

/// The folder most of the photos are in, and how many other folders hold
/// the rest.
pub(super) fn folder_summary<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for path in paths {
        let folder = std::path::Path::new(path).parent().map(|parent| folder_label(&parent.to_string_lossy())).unwrap_or_default();
        *counts.entry(folder).or_default() += 1;
    }
    let others = counts.len().saturating_sub(1);
    let top = counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(folder, _)| folder).unwrap_or_default();
    if others == 0 {
        top
    } else {
        format!("{top}  +{others}")
    }
}

/// Up to `count` of `items`, evenly spread from first to last.
fn spread(items: &[usize], count: usize) -> Vec<usize> {
    if items.len() <= count {
        return items.to_vec();
    }
    (0..count).map(|n| items[n * items.len() / count]).collect()
}

/// Texture area that fills `size` with a `w`×`h` thumbnail turned by
/// `rotation`, cropping what spills over.
fn cover_uv_sized(w: usize, h: usize, rotation: i32, size: Vec2) -> Rect {
    let turned = rotation.rem_euclid(180) == 90;
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let (shown_w, shown_h) = if turned { (h, w) } else { (w, h) };
    let aspect = size.x / size.y.max(1.0);
    let (fx, fy) = if shown_w / shown_h > aspect { (aspect * shown_h / shown_w, 1.0) } else { (1.0, shown_w / aspect / shown_h) };
    let (fx, fy) = if turned { (fy, fx) } else { (fx, fy) };
    Rect::from_center_size(Pos2::new(0.5, 0.5), Vec2::new(fx, fy))
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

pub(super) fn track_tooltip(ui: &mut egui::Ui, track: &MapTrack) {
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
        LabelKind::Water => (11.5, Color32::from_rgb(0x2F, 0x66, 0x70)),
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
    fn scale_bar_rounds_down_to_one_two_or_five() {
        assert_eq!(scale_bar(0.27, 100.0), (20.0, 20.0 / 0.27));
        assert_eq!(scale_bar(4.0, 100.0), (200.0, 50.0));
        assert_eq!(scale_bar(60.0, 100.0), (5000.0, 5000.0 / 60.0));
        assert_eq!(scale_bar(0.1, 100.0).0, 10.0);
    }

    #[test]
    fn a_missing_tile_falls_back_to_the_one_above() {
        assert_eq!(covering(19, 448935, 878371, 0), (19, 448935, 878371));
        assert_eq!(covering(19, 448935, 878371, 2), (17, 112233, 219592));
    }

    #[test]
    fn closest_zoom_shows_about_thirty_meters_per_hundred_points() {
        let meters = geo::meters_per_point(25.03, MAX_ZOOM) * 100.0;
        assert!((26.0..29.0).contains(&meters), "{meters}");
    }

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

    #[test]
    fn nearby_photos_share_a_cluster_and_distant_ones_do_not() {
        // Two either side of a cell edge, two in one cell, and one alone.
        let points = [[62.0, 10.0], [66.0, 12.0], [300.0, 300.0], [302.0, 303.0], [600.0, 20.0]];
        let clusters = cluster_points(&points, 64.0);
        let sizes: Vec<usize> = clusters.iter().map(|(members, _)| members.len()).collect();
        assert_eq!(sizes, [2, 2, 1]);
        let (members, middle) = &clusters[1];
        assert_eq!(members, &[0, 1], "dots that would overlap merge across the edge");
        assert!((middle[0] - 64.0).abs() < 1e-9 && (middle[1] - 11.0).abs() < 1e-9);
    }

    #[test]
    fn counts_read_short_in_a_circle_and_grouped_on_the_card() {
        assert_eq!(short_count(999), "999");
        assert_eq!(short_count(3185), "3.2k");
        assert_eq!(short_count(48_000), "48k");
        assert_eq!(grouped(3185), "3,185");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(12), "12");
    }
}
