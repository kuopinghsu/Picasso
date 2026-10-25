//! Travel Map: the countries, and the states and provinces of the largest
//! ones, your geotagged photos were taken in, on a globe or a flat map, with
//! how much of the world that covers. Only the photos' locations count;
//! nothing is marked by hand.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use eframe::egui::{self, epaint::Vertex, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use crate::config::Config;
use crate::geo;
use crate::i18n::{tr, trf};
use crate::track::Bounds;
use crate::types::Photo;
use crate::world::{self, world, Globe, CONTINENTS, FLAT_EXTENT};

use super::places::{grouped, truncated};
use super::stats::{meter, percent};
use super::theme::{self, ACCENT};
use super::PicassoApp;

const MIN_ZOOM: f32 = 0.7;
const MAX_ZOOM: f32 = 40.0;
/// Countries visited at which the next goal is set.
const MILESTONES: [usize; 7] = [10, 25, 50, 75, 100, 150, 200];
/// Widest the column beside the map gets.
const SIDE_W: f32 = 300.0;
/// Outline points closer than this many pixels to the last one drawn are
/// skipped; a world view would otherwise draw a hundred thousand.
const MIN_STEP: f32 = 0.8;
/// Fill of a place with a single photo, and of the place with the most.
const FEW: Color32 = Color32::from_rgb(0xE6, 0xB3, 0xCF);
const MANY: Color32 = Color32::from_rgb(0x9C, 0x2F, 0x6A);
/// Where the globe faces before there are photos to face.
const DEFAULT_FACING: [f32; 2] = [115.0, 20.0];

/// The Travel Map's view and where the photos in it were taken.
pub(super) struct Travel {
    globe: bool,
    /// Longitude and latitude facing the viewer on the globe.
    facing: [f32; 2],
    zoom: f32,
    /// How far the flat map is dragged from the middle, in points.
    pan: Vec2,
    /// Whether the view was turned to the photos yet.
    placed: bool,
    visits: Option<Rc<Visits>>,
    /// Where the map was last drawn.
    rect: Rect,
    painted: Option<Painted>,
}

impl Travel {
    pub(super) fn load(config: &Config) -> Travel {
        Travel {
            globe: config.flag("travel_globe").unwrap_or(true),
            facing: DEFAULT_FACING,
            zoom: 1.0,
            pan: Vec2::ZERO,
            placed: false,
            visits: None,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0)),
            painted: None,
        }
    }

    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.facing = self.visits.as_ref().and_then(|visits| visits.top_country()).map_or(DEFAULT_FACING, |country| world().areas[country].label);
    }

    fn photos(&self, area: usize) -> usize {
        self.visits.as_ref().map_or(0, |visits| visits.photos[area])
    }
}

/// Where the photos in view were taken, by area.
pub(super) struct Visits {
    epoch: u64,
    /// Photos in each area of the world map; a state's photos count for its
    /// country too.
    photos: Vec<usize>,
    /// Where they are, in world pixels at zoom 0.
    bounds: Vec<Bounds>,
    /// Most photos in an area drawn on its own: a state, or a country
    /// without states.
    most: usize,
}

impl Visits {
    pub(super) fn compute(photos: &[Photo], epoch: u64) -> Visits {
        let world = world();
        let mut visits = Visits {
            epoch,
            photos: vec![0; world.areas.len()],
            bounds: vec![Bounds::EMPTY; world.areas.len()],
            most: 0,
        };
        // Photos are taken in bunches; look each spot of about 1 km up once.
        let mut spots: HashMap<(i32, i32), Option<usize>> = HashMap::new();
        for (lat, lon) in photos.iter().filter_map(Photo::place) {
            let spot = ((lat * 100.0).round() as i32, (lon * 100.0).round() as i32);
            let Some(area) = *spots.entry(spot).or_insert_with(|| world.locate(lat, lon)) else { continue };
            let (x, y) = geo::latlon_to_world(lat, lon, 0);
            for index in std::iter::once(area).chain(world.areas[area].parent) {
                visits.photos[index] += 1;
                visits.bounds[index].add([x, y]);
            }
        }
        visits.most = (0..world.areas.len())
            .filter(|&index| world.children[index].is_empty())
            .map(|index| visits.photos[index])
            .max()
            .unwrap_or(0);
        visits
    }

    /// The country with the most photos.
    fn top_country(&self) -> Option<usize> {
        world().countries.iter().copied().filter(|&country| self.photos[country] > 0).max_by_key(|&country| self.photos[country])
    }
}

/// Totals over the countries.
struct Summary {
    /// Countries with photos and how many, most first.
    countries: Vec<(usize, usize)>,
    total: usize,
    /// States and provinces with photos.
    regions: usize,
    /// Visited and all countries in each of `CONTINENTS`.
    continents: [(usize, usize); CONTINENTS.len()],
}

impl Summary {
    fn of(visits: Option<&Visits>) -> Summary {
        let world = world();
        let photos = |area: usize| visits.map_or(0, |visits| visits.photos[area]);
        let mut summary = Summary {
            countries: Vec::new(),
            total: world.countries.len(),
            regions: (0..world.areas.len()).filter(|&area| world.areas[area].parent.is_some() && photos(area) > 0).count(),
            continents: [(0, 0); CONTINENTS.len()],
        };
        for &country in &world.countries {
            let continent = world.areas[country].continent;
            summary.continents[continent].1 += 1;
            if photos(country) > 0 {
                summary.continents[continent].0 += 1;
                summary.countries.push((country, photos(country)));
            }
        }
        summary
            .countries
            .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| world.areas[a.0].name().cmp(world.areas[b.0].name())));
        summary
    }

    fn visited(&self) -> usize {
        self.countries.len()
    }

    fn continents_started(&self) -> usize {
        self.continents.iter().filter(|(visited, _)| *visited > 0).count()
    }

    /// The next number of countries to reach, if any are left.
    fn next_milestone(&self) -> Option<usize> {
        MILESTONES
            .into_iter()
            .chain(std::iter::once(self.total))
            .find(|goal| *goal > self.visited() && *goal <= self.total)
    }

    /// The four big numbers: countries, share of the world, continents,
    /// and either the next goal or the states and provinces.
    fn tiles(&self, goal: bool) -> [(String, String); 4] {
        let last = if goal {
            match self.next_milestone() {
                Some(next) => (grouped(next), trf("Next goal · {} to go", &[&(next - self.visited())])),
                None => ("✓".into(), tr("Every country").to_string()),
            }
        } else {
            (grouped(self.regions), tr("States and provinces").to_string())
        };
        [
            (grouped(self.visited()), tr("Countries visited").to_string()),
            (format!("{}%", percent(self.visited(), self.total)), trf("of {} countries", &[&self.total])),
            (format!("{}/{}", self.continents_started(), CONTINENTS.len()), tr("Continents").to_string()),
            last,
        ]
    }
}

/// Fill of a place with `photos` of the most any place has: deeper with
/// more, on a log scale so a few photos still show.
fn shade(photos: usize, most: usize) -> Color32 {
    let t = if most <= 1 { 1.0 } else { ((photos as f32).ln() / (most as f32).ln()).clamp(0.0, 1.0) };
    FEW.lerp_to_gamma(MANY, t)
}

/// How the map is laid on the screen.
#[derive(Clone, Copy)]
struct View {
    rect: Rect,
    globe: Option<Globe>,
    /// Screen position of the globe's center or the flat map's origin.
    origin: Pos2,
    /// Points per unit: the globe's radius, or per flat projection unit.
    scale: f32,
}

impl View {
    fn of(travel: &Travel, rect: Rect) -> View {
        if travel.globe {
            View {
                rect,
                globe: Some(Globe::facing(travel.facing[0], travel.facing[1])),
                origin: rect.center(),
                scale: rect.width().min(rect.height()) * 0.46 * travel.zoom,
            }
        } else {
            View {
                rect,
                globe: None,
                origin: rect.center() + travel.pan,
                scale: flat_fit(rect) * travel.zoom,
            }
        }
    }

    fn at(&self, [x, y]: [f32; 2]) -> Pos2 {
        self.origin + Vec2::new(x * self.scale, -y * self.scale)
    }

    /// Longitude and latitude under `pos`.
    fn lon_lat(&self, pos: Pos2) -> Option<(f32, f32)> {
        let x = (pos.x - self.origin.x) / self.scale;
        let y = (self.origin.y - pos.y) / self.scale;
        match self.globe {
            Some(globe) => globe.unproject(x, y),
            None => world::natural_earth_inverse(x, y),
        }
    }
}

/// Points per flat projection unit that fit the whole world in `rect`.
fn flat_fit(rect: Rect) -> f32 {
    (rect.width() / (2.0 * FLAT_EXTENT[0])).min(rect.height() / (2.0 * FLAT_EXTENT[1])) * 0.94
}

/// The last map drawn and what it was drawn for.
struct Painted {
    key: PaintKey,
    shapes: Vec<Shape>,
}

#[derive(PartialEq)]
struct PaintKey {
    rect: Rect,
    globe: bool,
    facing: [f32; 2],
    zoom: f32,
    pan: Vec2,
    epoch: u64,
    hover: Option<usize>,
    dark: bool,
}

impl PicassoApp {
    pub(super) fn travel_ui(&mut self, ui: &mut egui::Ui) {
        if self.travel.visits.as_ref().is_none_or(|visits| visits.epoch != self.view_epoch) {
            self.travel.visits = Some(Rc::new(Visits::compute(&self.photos, self.view_epoch)));
        }
        if !self.travel.placed && !self.loading {
            self.travel.placed = true;
            self.travel.reset_view();
        }
        self.travel_toolbar(ui);
        ui.add_space(4.0);
        let full = ui.available_rect_before_wrap();
        let wide = full.width() >= 860.0;
        let (map_rect, side_rect) = if wide {
            let side = Rect::from_min_max(Pos2::new(full.right() - SIDE_W, full.top()), full.max);
            (Rect::from_min_max(full.min, Pos2::new(side.left() - 14.0, full.bottom())), side)
        } else {
            let split = full.top() + (full.height() * 0.6).max(240.0).min(full.height());
            (
                Rect::from_min_max(full.min, Pos2::new(full.right(), split)),
                Rect::from_min_max(Pos2::new(full.left(), split + 10.0), full.max),
            )
        };
        let summary = Summary::of(self.travel.visits.as_deref());
        self.travel_map(ui, map_rect, summary.visited() == 0);
        ui.advance_cursor_after_rect(full);
        if side_rect.height() > 40.0 {
            let mut side = ui.new_child(egui::UiBuilder::new().max_rect(side_rect));
            egui::ScrollArea::vertical().id_salt("travel-side").auto_shrink([false, false]).show(&mut side, |ui| {
                self.travel_side(ui, &summary);
            });
        }
    }

    fn travel_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut globe = self.travel.globe;
            ui.selectable_value(&mut globe, true, tr("Globe"));
            ui.selectable_value(&mut globe, false, tr("Flat"));
            if globe != self.travel.globe {
                self.travel.globe = globe;
                self.travel.reset_view();
                self.config.set("travel_globe", globe);
            }
            if ui.button(tr("Reset view")).clicked() {
                self.travel.reset_view();
            }
        });
    }

    fn travel_map(&mut self, ui: &mut egui::Ui, rect: Rect, empty: bool) {
        self.travel.rect = rect;
        let response = ui.interact(rect, egui::Id::unique("travel-map"), Sense::click_and_drag());
        if response.dragged() {
            let delta = response.drag_delta();
            if self.travel.globe {
                let view = View::of(&self.travel, rect);
                let [lon, lat] = &mut self.travel.facing;
                *lon = (*lon - (delta.x / view.scale).to_degrees() + 540.0).rem_euclid(360.0) - 180.0;
                *lat = (*lat + (delta.y / view.scale).to_degrees()).clamp(-89.0, 89.0);
            } else {
                self.travel.pan += delta;
            }
        }
        if response.hovered() {
            let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
            let factor = (scroll * 0.003).exp() * pinch;
            if (factor - 1.0).abs() > 1e-4 {
                let pointer = response.hover_pos().unwrap_or(rect.center());
                self.zoom_travel_map(rect, factor, pointer);
            }
        }
        if response.double_clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                self.zoom_travel_map(rect, 2.0, pointer);
            }
        }
        self.clamp_travel_pan(rect);

        let view = View::of(&self.travel, rect);
        let pointer = response.hover_pos().filter(|_| !response.dragged());
        let hover = pointer.and_then(|pos| view.lon_lat(pos)).and_then(|(lon, lat)| world().area_at(lat, lon));
        let key = PaintKey {
            rect,
            globe: self.travel.globe,
            facing: self.travel.facing,
            zoom: self.travel.zoom,
            pan: self.travel.pan,
            epoch: self.travel.visits.as_ref().map_or(0, |visits| visits.epoch),
            hover,
            dark: theme::is_dark(),
        };
        if self.travel.painted.as_ref().is_none_or(|painted| painted.key != key) {
            let shapes = paint_map(&self.travel, view, hover);
            self.travel.painted = Some(Painted { key, shapes });
        }
        let painter = ui.painter_at(rect);
        if let Some(painted) = &self.travel.painted {
            painter.extend(painted.shapes.iter().cloned());
        }
        self.travel_zoom_buttons(ui, rect);

        if empty && !self.loading {
            let hint = tr("Places turn colored where your geotagged photos were taken.");
            let text = truncated(&painter, hint.to_string(), 12.0, theme::muted(), rect.width() - 24.0);
            painter.galley(Pos2::new(rect.left() + 12.0, rect.bottom() - 10.0 - text.size().y), text, theme::muted());
        }
        let Some(area) = hover else { return };
        let photos = self.travel.photos(area);
        if photos > 0 {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let response = response.on_hover_ui_at_pointer(|ui| self.travel_tooltip(ui, area));
        if response.clicked() && photos > 0 {
            let bounds = self.travel.visits.as_ref().map(|visits| visits.bounds[area]);
            if let Some(bounds) = bounds {
                self.show_area_on_map(bounds);
            }
        }
    }

    /// Zooms by `factor`, keeping the place under `pointer` there on the
    /// flat map.
    fn zoom_travel_map(&mut self, rect: Rect, factor: f32, pointer: Pos2) {
        let zoom = (self.travel.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let applied = zoom / self.travel.zoom;
        if !self.travel.globe {
            let from_middle = pointer - rect.center();
            self.travel.pan = from_middle - (from_middle - self.travel.pan) * applied;
        }
        self.travel.zoom = zoom;
    }

    /// Keeps some of the flat map in sight.
    fn clamp_travel_pan(&mut self, rect: Rect) {
        if self.travel.globe {
            return;
        }
        let half = Vec2::new(FLAT_EXTENT[0], FLAT_EXTENT[1]) * flat_fit(rect) * self.travel.zoom + rect.size() * 0.5 - Vec2::splat(60.0);
        self.travel.pan = self.travel.pan.clamp(-half.max(Vec2::ZERO), half.max(Vec2::ZERO));
    }

    fn travel_zoom_buttons(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let corner = rect.left_top() + Vec2::new(10.0, 10.0);
        for (n, (label, factor, tip)) in [("+", 2.0, "Zoom in"), ("−", 0.5, "Zoom out")].into_iter().enumerate() {
            let button = Rect::from_min_size(corner + Vec2::new(0.0, n as f32 * 32.0), Vec2::splat(28.0));
            if ui.put(button, egui::Button::new(egui::RichText::new(label).size(16.0))).on_hover_text(tr(tip)).clicked() {
                self.zoom_travel_map(rect, factor, rect.center());
            }
        }
    }

    fn travel_tooltip(&self, ui: &mut egui::Ui, area: usize) {
        let world = world();
        ui.label(egui::RichText::new(world.areas[area].name()).strong().color(theme::text()));
        if let Some(parent) = world.areas[area].parent {
            ui.label(egui::RichText::new(world.areas[parent].name()).size(12.0).color(theme::muted()));
        }
        let photos = self.travel.photos(area);
        if photos == 0 {
            ui.label(egui::RichText::new(tr("No photos here")).size(12.0).color(theme::muted()));
            return;
        }
        ui.label(egui::RichText::new(super::photo_count(photos)).size(12.0).color(theme::text()));
        ui.label(egui::RichText::new(tr("Click to see these photos on the map")).size(11.5).color(theme::muted()));
    }

    fn travel_side(&mut self, ui: &mut egui::Ui, summary: &Summary) {
        let width = ui.available_width();
        tiles(ui, &summary.tiles(true), 2, width);

        if let Some(most) = self.travel.visits.as_ref().map(|visits| visits.most).filter(|most| *most > 0) {
            ui.add_space(10.0);
            heading(ui, tr("Photos"));
            let (bar, _) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
            let mut mesh = Mesh::default();
            mesh.colored_vertex(bar.left_top(), shade(1, most));
            mesh.colored_vertex(bar.right_top(), shade(most, most));
            mesh.colored_vertex(bar.right_bottom(), shade(most, most));
            mesh.colored_vertex(bar.left_bottom(), shade(1, most));
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(mesh);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("1").size(11.5).color(theme::muted()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(grouped(most)).size(11.5).color(theme::muted()));
                });
            });
        }

        ui.add_space(12.0);
        heading(ui, tr("Coverage by continent"));
        for (index, name) in CONTINENTS.iter().enumerate() {
            let (visited, total) = summary.continents[index];
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tr(name)).size(12.5).color(theme::text()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{visited} / {total}")).size(12.0).color(theme::muted()));
                });
            });
            let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
            meter(ui.painter(), bar, visited as f32 / total.max(1) as f32, ACCENT);
            ui.add_space(4.0);
        }

        if !summary.countries.is_empty() {
            ui.add_space(12.0);
            heading(ui, tr("Your countries"));
            ui.spacing_mut().item_spacing.y = 0.0;
            let most = summary.countries[0].1;
            let mut focus = None;
            for &(country, photos) in &summary.countries {
                if country_row(ui, world().areas[country].name(), shade(photos, most), photos) {
                    focus = Some(country);
                }
            }
            if let Some(country) = focus {
                self.focus_travel_area(country);
            }
        }
        ui.add_space(16.0);
    }

    /// Turns or moves the map to `area` and zooms to about its size.
    fn focus_travel_area(&mut self, area: usize) {
        let area = &world().areas[area];
        let bounds = area.bounds;
        let squeeze = area.label[1].to_radians().cos().max(0.2);
        let extent = ((bounds.east - bounds.west) * squeeze).max(bounds.north - bounds.south).max(1.0);
        let travel = &mut self.travel;
        let span = if travel.globe { 70.0 } else { 120.0 };
        travel.zoom = (span / extent).clamp(1.0, MAX_ZOOM);
        travel.facing = area.label;
        let scale = flat_fit(travel.rect) * travel.zoom;
        let [x, y] = world::natural_earth(area.label[0], area.label[1]);
        travel.pan = Vec2::new(-x * scale, y * scale);
    }

    /// The Travel card of the Statistics page: the big numbers for the
    /// photos in view. Returns whether to open the map.
    pub(super) fn travel_card(&mut self, ui: &mut egui::Ui, visits: &Visits) -> bool {
        let summary = Summary::of(Some(visits));
        let width = ui.available_width();
        tiles(ui, &summary.tiles(false), if width >= 520.0 { 4 } else { 2 }, width);
        ui.add_space(6.0);
        let columns = if width >= 520.0 { CONTINENTS.len() } else { 1 };
        let column_w = (width - 16.0 * (columns - 1) as f32) / columns as f32;
        for chunk in (0..CONTINENTS.len()).collect::<Vec<_>>().chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for &index in chunk {
                    let (visited, total) = summary.continents[index];
                    ui.vertical(|ui| {
                        ui.set_width(column_w);
                        let name = truncated(ui.painter(), tr(CONTINENTS[index]).to_string(), 12.0, theme::text(), column_w);
                        ui.label(name);
                        let (bar, _) = ui.allocate_exact_size(Vec2::new(column_w, 6.0), Sense::hover());
                        meter(ui.painter(), bar, visited as f32 / total.max(1) as f32, ACCENT);
                        ui.label(egui::RichText::new(format!("{visited} / {total}")).size(11.5).color(theme::muted()));
                    });
                }
            });
        }
        ui.add_space(8.0);
        ui.button(tr("Open Travel Map")).clicked()
    }
}

/// Builds the map: sea, graticule, land colored by photos, then borders.
fn paint_map(travel: &Travel, view: View, hover: Option<usize>) -> Vec<Shape> {
    let world = world();
    let dark = theme::is_dark();
    let sea = if dark { Color32::from_rgb(0x17, 0x2B, 0x3D) } else { Color32::from_rgb(0xEC, 0xF5, 0xFB) };
    let rim = if dark { Color32::from_rgb(0x2A, 0x40, 0x55) } else { Color32::from_rgb(0xC9, 0xDB, 0xE8) };
    let grid = Stroke::new(0.6, if dark { Color32::from_white_alpha(18) } else { Color32::from_rgba_unmultiplied(0x5A, 0x7A, 0x96, 40) });
    let border = if dark { Color32::from_rgb(0x10, 0x1C, 0x27) } else { Color32::WHITE };
    let most = travel.visits.as_ref().map_or(0, |visits| visits.most);
    let mut shapes = Vec::new();

    match view.globe {
        Some(_) => shapes.push(Shape::circle_filled(view.origin, view.scale, sea)),
        None => {
            let mut edge: Vec<Pos2> = (-90..=90).rev().step_by(5).map(|lat| view.at(world::natural_earth(180.0, lat as f32))).collect();
            edge.extend((-90..=90).step_by(5).map(|lat| view.at(world::natural_earth(-180.0, lat as f32))));
            shapes.push(Shape::convex_polygon(edge, sea, Stroke::NONE));
        }
    }
    let mut lines: Vec<Vec<Pos2>> = Vec::new();
    for lon in (-180..180).step_by(30) {
        let points: Vec<[f32; 2]> = (-90..=90).step_by(3).map(|lat| [lon as f32, lat as f32]).collect();
        trace(&view, &points, &mut lines);
    }
    for lat in (-60..=60).step_by(30) {
        let points: Vec<[f32; 2]> = (-180..=180).step_by(3).map(|lon| [lon as f32, lat as f32]).collect();
        trace(&view, &points, &mut lines);
    }
    shapes.extend(lines.drain(..).map(|line| Shape::line(line, grid)));

    let mut mesh = Mesh::default();
    let mut projected: Vec<[f32; 3]> = Vec::new();
    for (index, area) in world.areas.iter().enumerate() {
        // A country with states is drawn plain under them; only gaps
        // between the states show it.
        let photos = if world.children[index].is_empty() { travel.photos(index) } else { 0 };
        let mut color = if photos > 0 { shade(photos, most) } else { land_color(&world.areas[world.country_of(index)].id, dark) };
        if hover == Some(index) {
            color = if photos > 0 { color.lerp_to_gamma(Color32::WHITE, 0.3) } else { color.lerp_to_gamma(ACCENT, 0.25) };
        }
        fill_area(&view, area, color, &mut mesh, &mut projected);
    }
    shapes.push(Shape::Mesh(Arc::new(mesh)));

    let mut regions = Vec::new();
    let mut countries = Vec::new();
    for area in &world.areas {
        if !visible(&view, area) {
            continue;
        }
        let out = if area.parent.is_some() { &mut regions } else { &mut countries };
        for &(start, len) in &area.rings {
            let range = start as usize..(start + len) as usize;
            trace_ring(&view, &area.points[range.clone()], &area.sphere[range.clone()], &area.flat[range], out);
        }
    }
    let thin = Stroke::new(0.5, border.gamma_multiply(0.7));
    let thick = Stroke::new(if view.scale > 900.0 { 1.2 } else { 0.8 }, border);
    shapes.extend(regions.into_iter().map(|line| Shape::line(line, thin)));
    shapes.extend(countries.into_iter().map(|line| Shape::line(line, thick)));
    if view.globe.is_some() {
        shapes.push(Shape::circle_stroke(view.origin, view.scale, Stroke::new(1.0, rim)));
    }
    shapes
}

/// Whether any of `area` can be on screen.
fn visible(view: &View, area: &world::Area) -> bool {
    match view.globe {
        Some(_) => true,
        None => {
            let [x0, y0, x1, y1] = area.flat_bounds;
            Rect::from_two_pos(view.at([x0, y0]), view.at([x1, y1])).intersects(view.rect)
        }
    }
}

/// Adds the triangles of `area` in `color`. On the globe, points on the far
/// side are pulled onto the rim and triangles entirely there are left out.
fn fill_area(view: &View, area: &world::Area, color: Color32, mesh: &mut Mesh, projected: &mut Vec<[f32; 3]>) {
    if !visible(view, area) {
        return;
    }
    let base = mesh.vertices.len() as u32;
    let vertex = |pos: Pos2| Vertex { pos, uv: egui::epaint::WHITE_UV, color };
    match view.globe {
        None => {
            mesh.vertices.extend(area.flat.iter().map(|point| vertex(view.at(*point))));
            mesh.indices.extend(area.triangles.iter().map(|index| base + index));
        }
        Some(globe) => {
            projected.clear();
            projected.extend(area.sphere.iter().map(|point| globe.project(*point)));
            if projected.iter().all(|[_, _, depth]| *depth < 0.0) {
                return;
            }
            mesh.vertices.extend(projected.iter().map(|&[x, y, depth]| {
                let (x, y) = if depth < 0.0 { onto_rim(x, y) } else { (x, y) };
                vertex(view.at([x, y]))
            }));
            for triangle in area.triangles.chunks_exact(3) {
                if triangle.iter().any(|&index| projected[index as usize][2] >= 0.0) {
                    mesh.indices.extend(triangle.iter().map(|index| base + index));
                }
            }
        }
    }
}

fn onto_rim(x: f32, y: f32) -> (f32, f32) {
    let length = (x * x + y * y).sqrt();
    if length < 1e-6 {
        (1.0, 0.0)
    } else {
        (x / length, y / length)
    }
}

/// Adds the visible stretches of an outline to `out`. The globe cuts it at
/// the rim; edges that only close a shape at the date line or the pole are
/// left out.
fn trace_ring(view: &View, ring: &[[f32; 2]], sphere: &[[f32; 3]], flat: &[[f32; 2]], out: &mut Vec<Vec<Pos2>>) {
    let mut run: Vec<Pos2> = Vec::new();
    let count = ring.len();
    for step in 0..=count {
        let index = step % count;
        let previous = (index + count - 1) % count;
        if step > 0 && world::Area::is_seam(ring[previous], ring[index]) {
            finish(&mut run, out);
            continue;
        }
        match view.globe {
            None => push(&mut run, view.at(flat[index])),
            Some(globe) => {
                let here = globe.project(sphere[index]);
                if step > 0 {
                    let before = globe.project(sphere[previous]);
                    if (before[2] >= 0.0) != (here[2] >= 0.0) {
                        let t = before[2] / (before[2] - here[2]);
                        let (x, y) = onto_rim(before[0] + (here[0] - before[0]) * t, before[1] + (here[1] - before[1]) * t);
                        push(&mut run, view.at([x, y]));
                        if here[2] < 0.0 {
                            finish(&mut run, out);
                        }
                    }
                }
                if here[2] >= 0.0 {
                    push(&mut run, view.at([here[0], here[1]]));
                }
            }
        }
    }
    finish(&mut run, out);
}

/// Adds the visible stretches of a line through `points`, longitude and
/// latitude, to `out`.
fn trace(view: &View, points: &[[f32; 2]], out: &mut Vec<Vec<Pos2>>) {
    let mut run = Vec::new();
    for &[lon, lat] in points {
        match view.globe {
            None => push(&mut run, view.at(world::natural_earth(lon, lat))),
            Some(globe) => {
                let [x, y, depth] = globe.project(world::unit_vector(lon, lat));
                if depth >= 0.0 {
                    push(&mut run, view.at([x, y]));
                } else {
                    finish(&mut run, out);
                }
            }
        }
    }
    finish(&mut run, out);
}

/// Adds `pos` to the line unless it is too close to the last point.
fn push(run: &mut Vec<Pos2>, pos: Pos2) {
    if run.last().is_none_or(|last| last.distance(pos) >= MIN_STEP) {
        run.push(pos);
    }
}

fn finish(run: &mut Vec<Pos2>, out: &mut Vec<Vec<Pos2>>) {
    if run.len() >= 2 {
        out.push(std::mem::take(run));
    } else {
        run.clear();
    }
}

/// A pale land color for a country without photos, the same for its
/// states, so neighbors differ a little.
fn land_color(id: &str, dark: bool) -> Color32 {
    const LIGHT: [(u8, u8, u8); 5] = [(0xE6, 0xE1, 0xD6), (0xDE, 0xE5, 0xD5), (0xE5, 0xDD, 0xE2), (0xDA, 0xE2, 0xE6), (0xEA, 0xE3, 0xD0)];
    const DARK: [(u8, u8, u8); 5] = [(0x3B, 0x42, 0x4D), (0x38, 0x46, 0x41), (0x45, 0x3F, 0x4A), (0x36, 0x43, 0x50), (0x46, 0x43, 0x3A)];
    let hash = id.bytes().fold(0x811C_9DC5u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193));
    let (r, g, b) = if dark { DARK } else { LIGHT }[hash as usize % LIGHT.len()];
    Color32::from_rgb(r, g, b)
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).strong().color(theme::muted()));
    ui.add_space(2.0);
}

/// Big numbers over their labels, `per_row` across `width`.
fn tiles(ui: &mut egui::Ui, tiles: &[(String, String)], per_row: usize, width: f32) {
    let tile_w = (width - 10.0 * (per_row - 1) as f32) / per_row as f32;
    for row in tiles.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (value, label) in row {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(tile_w, 64.0), Sense::hover());
                let painter = ui.painter();
                painter.rect(rect, 10, theme::sidebar(), Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
                let inner = rect.shrink2(Vec2::new(12.0, 10.0));
                let value = truncated(painter, value.clone(), 20.0, theme::text(), inner.width());
                painter.galley(inner.min, value, theme::text());
                let label = truncated(painter, label.clone(), 11.5, theme::muted(), inner.width());
                painter.galley(Pos2::new(inner.left(), inner.bottom() - label.size().y), label, theme::muted());
            }
        });
        ui.add_space(4.0);
    }
}

/// A country in the list: its color, name, and photos. Returns whether it
/// was clicked.
fn country_row(ui: &mut egui::Ui, name: &str, color: Color32, photos: usize) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Show on map"));
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 6, theme::select());
    }
    painter.circle_filled(Pos2::new(rect.left() + 12.0, rect.center().y), 5.0, color);
    let figure = painter.layout_no_wrap(grouped(photos), FontId::proportional(11.5), theme::muted());
    let room = rect.width() - 34.0 - figure.size().x - 8.0;
    let title = truncated(painter, name.to_string(), 13.0, theme::text(), room);
    painter.galley(Pos2::new(rect.left() + 24.0, rect.center().y - title.size().y / 2.0), title, theme::text());
    painter.galley(Pos2::new(rect.right() - 8.0 - figure.size().x, rect.center().y - figure.size().y / 2.0), figure, theme::muted());
    response.clicked()
}

/// Countries and states sorted by how many photos, for tests.
#[cfg(test)]
fn ranked(visits: &Visits) -> Vec<(&'static str, usize)> {
    let world = world();
    let mut rows: Vec<(&str, usize)> =
        (0..world.areas.len()).filter(|&area| visits.photos[area] > 0).map(|area| (world.areas[area].id.as_str(), visits.photos[area])).collect();
    rows.sort_by_key(|&(id, photos)| (std::cmp::Reverse(photos), id));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(id: i64, lat: f64, lon: f64) -> Photo {
        Photo {
            id,
            folder_id: 1,
            path: format!("/p/{id}.jpg"),
            filename: format!("{id}.jpg"),
            width: 1,
            height: 1,
            bytes: 1,
            taken_at: 0,
            modified_at: 0,
            starred: false,
            caption: String::new(),
            camera: String::new(),
            lens: String::new(),
            focal_mm: 0.0,
            gps_lat: Some(lat),
            gps_lon: Some(lon),
            pin_lat: None,
            pin_lon: None,
            rotation: 0,
            duration: 0.0,
        }
    }

    #[test]
    fn photos_mark_countries_and_states() {
        let photos = vec![
            photo(1, 25.03, 121.56),
            photo(2, 25.04, 121.55),
            photo(3, 22.63, 120.30),
            photo(4, 34.05, -118.24),
            photo(5, 40.71, -74.0),
            photo(6, 0.0, -30.0),
        ];
        let visits = Visits::compute(&photos, 1);
        assert_eq!(ranked(&visits), vec![("TW", 3), ("US", 2), ("US_CA", 1), ("US_NY", 1)]);
        assert_eq!(visits.most, 3);
        assert_eq!(visits.top_country().map(|country| world().areas[country].id.as_str()), Some("TW"));
        let summary = Summary::of(Some(&visits));
        assert_eq!(summary.visited(), 2);
        assert_eq!(summary.regions, 2);
        assert_eq!(summary.continents_started(), 2);
        assert_eq!(summary.next_milestone(), Some(10));
    }

    #[test]
    fn more_photos_shade_deeper() {
        assert_eq!(shade(1, 1), MANY);
        assert_eq!(shade(1, 100), FEW);
        assert_eq!(shade(100, 100), MANY);
        let middle = shade(10, 100);
        assert!(middle != FEW && middle != MANY);
    }
}
