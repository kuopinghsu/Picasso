//! Travel Map: the countries, and the states and provinces of the largest
//! ones, your geotagged photos were taken in, on a globe or a flat map, with
//! how much of the world that covers. Only the photos' locations count;
//! nothing is marked by hand.
//!
//! The map fills the page. The view switch, the years, the zoom buttons, and
//! a panel with the totals and the countries float over it.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{
    self, epaint::Vertex, Align, Color32, FontId, Layout, Margin, Mesh, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind,
    UiBuilder, Vec2,
};

use crate::config::Config;
use crate::geo;
use crate::i18n::{tr, trf};
use crate::track::Bounds;
use crate::types::Photo;
use crate::world::{self, world, Globe, CONTINENTS, FLAT_EXTENT};

use super::places::{date_span, grouped, truncated};
use super::stats::{meter, percent};
use super::theme::{self, ACCENT, WHITE};
use super::PicassoApp;

const MIN_ZOOM: f32 = 0.7;
const MAX_ZOOM: f32 = 40.0;
/// Countries visited at which the next goal is set.
const MILESTONES: [usize; 7] = [10, 25, 50, 75, 100, 150, 200];
/// Width of the panel floating beside the map.
const PANEL_W: f32 = 320.0;
/// Space between the map's edge and what floats on it.
const INSET: f32 = 12.0;
/// Outline points closer than this many pixels to the last one drawn are
/// skipped; a world view would otherwise draw a hundred thousand.
const MIN_STEP: f32 = 0.8;
/// Fill of a place with a single photo, and of the place with the most:
/// pale gold through [`theme::GOLD`] to burnt amber, warm against the sea
/// and the stone of places without photos.
const FEW: Color32 = Color32::from_rgb(0xFB, 0xDD, 0x8A);
const MANY: Color32 = Color32::from_rgb(0xC2, 0x65, 0x0A);
/// Where the globe faces before there are photos to face.
const DEFAULT_FACING: [f32; 2] = [115.0, 20.0];
/// Degrees a second the globe turns on its own after the map opens.
const IDLE_SPIN: f32 = 2.5;
/// Seconds it turns, while the window has focus and the map isn't touched.
/// Each frame of it redraws the whole world.
const SPIN_FOR: f32 = 20.0;
/// How quickly a flung map slows down, per second.
const GLIDE_DECAY: f32 = 3.5;
/// Seconds a flight to a place takes.
const FLIGHT: f64 = 0.9;

/// Where the map looks.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Camera {
    /// Longitude and latitude facing the viewer on the globe.
    facing: [f32; 2],
    zoom: f32,
    /// How far the flat map is moved from the middle, in points.
    pan: Vec2,
}

impl Camera {
    const HOME: Camera = Camera { facing: DEFAULT_FACING, zoom: 1.0, pan: Vec2::ZERO };
}

/// An eased move of the camera from one view to another.
struct Flight {
    from: Camera,
    to: Camera,
    start: f64,
    globe: bool,
}

impl Flight {
    /// The camera `t` of the way, from 0 to 1. A long way round the globe
    /// pulls back in the middle, so the trip can be followed.
    fn at(&self, t: f32) -> Camera {
        let eased = if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (2.0 - 2.0 * t).powi(3) / 2.0 };
        let (a, b) = (self.from, self.to);
        let turn = wrap_lon(b.facing[0] - a.facing[0]);
        let lift = b.facing[1] - a.facing[1];
        let dip = if self.globe { 0.35 * ((turn * turn + lift * lift).sqrt() / 90.0).min(1.0) * (t * std::f32::consts::PI).sin() } else { 0.0 };
        Camera {
            facing: [wrap_lon(a.facing[0] + turn * eased), a.facing[1] + lift * eased],
            zoom: a.zoom * (b.zoom / a.zoom).powf(eased) * (1.0 - dip),
            pan: a.pan + (b.pan - a.pan) * eased,
        }
    }
}

/// The Travel Map's view and where the photos in it were taken.
pub(super) struct Travel {
    globe: bool,
    camera: Camera,
    /// Whether the view was turned to the photos yet.
    placed: bool,
    visits: Option<Rc<Visits>>,
    /// The part of the map beside the panel, where it was last drawn.
    rect: Rect,
    painted: Option<Painted>,
    /// Only this year's photos count, when set.
    year: Option<i32>,
    /// The country or state the panel tells about.
    selected: Option<usize>,
    /// The country under the pointer in the panel's list.
    list_hover: Option<usize>,
    panel: bool,
    search: String,
    /// Only this continent's countries are listed, when set.
    continent: Option<usize>,
    flight: Option<Flight>,
    /// Speed a fling carries on with: degrees a second on the globe,
    /// points a second on the flat map.
    glide: Vec2,
    /// Seconds of the idle spin left; touching the map ends it.
    spin_left: f32,
    /// The place the panel's card was last shown for.
    card_for: Option<usize>,
    /// Time of the last animation step.
    tick: f64,
}

impl Travel {
    pub(super) fn load(config: &Config) -> Travel {
        Travel {
            globe: config.flag("travel_globe").unwrap_or(true),
            camera: Camera::HOME,
            placed: false,
            visits: None,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0)),
            painted: None,
            year: None,
            selected: None,
            list_hover: None,
            panel: config.flag("travel_panel").unwrap_or(true),
            search: String::new(),
            continent: None,
            flight: None,
            glide: Vec2::ZERO,
            spin_left: SPIN_FOR,
            card_for: None,
            tick: 0.0,
        }
    }

    /// The map was opened: the globe turns for a while again.
    pub(super) fn opened(&mut self) {
        self.spin_left = SPIN_FOR;
    }

    /// The photos that count: all of them, or the chosen year's.
    fn tally(&self) -> Option<&Tally> {
        let visits = self.visits.as_deref()?;
        match self.year {
            Some(year) => visits.years.iter().find(|(each, _)| *each == year).map(|(_, tally)| tally),
            None => Some(&visits.all),
        }
    }

    fn photos(&self, area: usize) -> usize {
        self.tally().map_or(0, |tally| tally.photos[area])
    }

    /// The whole world, turned to the country with the most photos.
    fn home(&self) -> Camera {
        let facing = self.tally().and_then(Tally::top_country).map_or(DEFAULT_FACING, |country| world().areas[country].label);
        Camera { facing, ..Camera::HOME }
    }

    /// Stops any motion; the user has the map now.
    fn touch(&mut self) {
        self.spin_left = 0.0;
        self.flight = None;
        self.glide = Vec2::ZERO;
    }

    fn fly_to(&mut self, to: Camera, now: f64) {
        self.touch();
        self.flight = Some(Flight { from: self.camera, to, start: now, globe: self.globe });
    }
}

/// Photos in each area of the world map; a state's photos count for its
/// country too.
pub(super) struct Tally {
    photos: Vec<usize>,
    /// Most photos in an area drawn on its own: a state, or a country
    /// without states.
    most: usize,
    /// Countries with photos.
    countries: usize,
}

impl Tally {
    fn new(areas: usize) -> Tally {
        Tally { photos: vec![0; areas], most: 0, countries: 0 }
    }

    fn finish(&mut self) {
        let world = world();
        self.most = (0..world.areas.len()).filter(|&index| world.children[index].is_empty()).map(|index| self.photos[index]).max().unwrap_or(0);
        self.countries = world.countries.iter().filter(|&&country| self.photos[country] > 0).count();
    }

    /// The country with the most photos.
    fn top_country(&self) -> Option<usize> {
        world().countries.iter().copied().filter(|&country| self.photos[country] > 0).max_by_key(|&country| self.photos[country])
    }
}

/// Where the photos in view were taken, by area, in all and by year.
pub(super) struct Visits {
    epoch: u64,
    all: Tally,
    /// Each year's photos, newest first.
    years: Vec<(i32, Tally)>,
    /// Where they are, in world pixels at zoom 0.
    bounds: Vec<Bounds>,
    /// First and last time a photo was taken in each area.
    span: Vec<Option<(i64, i64)>>,
}

impl Visits {
    pub(super) fn compute(photos: &[Photo], epoch: u64) -> Visits {
        use chrono::Datelike;
        let world = world();
        let areas = world.areas.len();
        let mut visits = Visits { epoch, all: Tally::new(areas), years: Vec::new(), bounds: vec![Bounds::EMPTY; areas], span: vec![None; areas] };
        let mut years: BTreeMap<i32, Tally> = BTreeMap::new();
        // Photos are taken in bunches; look each spot of about 1 km up once.
        let mut spots: HashMap<(i32, i32), Option<usize>> = HashMap::new();
        for photo in photos {
            let Some((lat, lon)) = photo.place() else { continue };
            let spot = ((lat * 100.0).round() as i32, (lon * 100.0).round() as i32);
            let Some(area) = *spots.entry(spot).or_insert_with(|| world.locate(lat, lon)) else { continue };
            let (x, y) = geo::latlon_to_world(lat, lon, 0);
            let year = chrono::DateTime::from_timestamp(photo.taken_at, 0).map(|time| time.with_timezone(&chrono::Local).year());
            let mut yearly = year.map(|year| years.entry(year).or_insert_with(|| Tally::new(areas)));
            for index in std::iter::once(area).chain(world.areas[area].parent) {
                visits.all.photos[index] += 1;
                if let Some(tally) = yearly.as_deref_mut() {
                    tally.photos[index] += 1;
                }
                visits.bounds[index].add([x, y]);
                let time = photo.taken_at;
                visits.span[index] = Some(visits.span[index].map_or((time, time), |(first, last)| (first.min(time), last.max(time))));
            }
        }
        visits.all.finish();
        visits.years = years
            .into_iter()
            .rev()
            .map(|(year, mut tally)| {
                tally.finish();
                (year, tally)
            })
            .collect();
        visits
    }

    /// Years with photos in `area`, oldest first.
    fn years_of(&self, area: usize) -> Vec<i32> {
        self.years.iter().rev().filter(|(_, tally)| tally.photos[area] > 0).map(|(year, _)| *year).collect()
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
    fn of(tally: Option<&Tally>) -> Summary {
        let world = world();
        let photos = |area: usize| tally.map_or(0, |tally| tally.photos[area]);
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
}

/// Fill of a place with `photos` of the most any place has: deeper with
/// more, on a log scale so a few photos still show.
fn shade(photos: usize, most: usize) -> Color32 {
    let t = if most <= 1 { 1.0 } else { ((photos as f32).ln() / (most as f32).ln()).clamp(0.0, 1.0) };
    ramp(t)
}

/// The shade `t` of the way from [`FEW`] to [`MANY`].
fn ramp(t: f32) -> Color32 {
    if t < 0.5 {
        FEW.lerp_to_gamma(theme::GOLD, t * 2.0)
    } else {
        theme::GOLD.lerp_to_gamma(MANY, t * 2.0 - 1.0)
    }
}

fn country_count(count: usize) -> String {
    trf(if count == 1 { "{} country" } else { "{} countries" }, &[&count])
}

fn wrap_lon(lon: f32) -> f32 {
    (lon + 540.0).rem_euclid(360.0) - 180.0
}

/// Turns the globe by `by`, degrees of longitude and latitude.
fn turn(facing: &mut [f32; 2], by: Vec2) {
    facing[0] = wrap_lon(facing[0] + by.x);
    facing[1] = (facing[1] + by.y).clamp(-89.0, 89.0);
}

/// Years as `2019, 2023–2026`, with runs of years joined.
fn years_text(years: &[i32]) -> String {
    let mut parts = Vec::new();
    let mut index = 0;
    while index < years.len() {
        let start = years[index];
        while index + 1 < years.len() && years[index + 1] == years[index] + 1 {
            index += 1;
        }
        let end = years[index];
        parts.push(if end == start { start.to_string() } else { format!("{start}–{end}") });
        index += 1;
    }
    parts.join(", ")
}

/// How the map is laid on the screen.
#[derive(Clone, Copy)]
struct View {
    /// The whole map, which is drawn under the floating panel too.
    rect: Rect,
    globe: Option<Globe>,
    /// Screen position of the globe's center or the flat map's origin.
    origin: Pos2,
    /// Points per unit: the globe's radius, or per flat projection unit.
    scale: f32,
}

impl View {
    /// The view of `travel` centered in `focus`, the part of `rect` beside
    /// the panel.
    fn of(travel: &Travel, focus: Rect, rect: Rect) -> View {
        let camera = travel.camera;
        if travel.globe {
            View {
                rect,
                globe: Some(Globe::facing(camera.facing[0], camera.facing[1])),
                origin: focus.center(),
                scale: focus.width().min(focus.height()) * 0.42 * camera.zoom,
            }
        } else {
            View { rect, globe: None, origin: focus.center() + camera.pan, scale: flat_fit(focus) * camera.zoom }
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
    focus: Rect,
    globe: bool,
    camera: Camera,
    epoch: u64,
    year: Option<i32>,
    hover: Option<usize>,
    selected: Option<usize>,
    dark: bool,
}

impl PicassoApp {
    pub(super) fn travel_ui(&mut self, ui: &mut egui::Ui) {
        if self.travel.visits.as_ref().is_none_or(|visits| visits.epoch != self.view_epoch) {
            let visits = Visits::compute(&self.photos, self.view_epoch);
            if self.travel.year.is_some_and(|year| !visits.years.iter().any(|(each, _)| *each == year)) {
                self.travel.year = None;
            }
            self.travel.visits = Some(Rc::new(visits));
        }
        if !self.travel.placed && !self.loading {
            self.travel.placed = true;
            self.travel.camera = self.travel.home();
        }
        let full = ui.available_rect_before_wrap();
        ui.advance_cursor_after_rect(full);
        let open = ui.ctx().animate_bool_with_time(egui::Id::unique("travel-panel-open"), self.travel.panel, 0.22);
        let panel_w = PANEL_W.min(full.width() * 0.55);
        let focus = Rect::from_min_max(full.min, Pos2::new(full.right() - (panel_w + INSET) * open, full.bottom()));
        let summary = Summary::of(self.travel.tally());

        self.animate_travel(ui.ctx());
        self.travel_map(ui, full, focus);
        self.travel_controls(ui, full, focus, &summary);
        if open > 0.0 {
            let slide = (1.0 - open) * (panel_w + INSET);
            let panel = Rect::from_min_max(
                Pos2::new(full.right() - INSET - panel_w + slide, full.top() + INSET),
                Pos2::new(full.right() - INSET + slide, full.bottom() - INSET),
            );
            self.travel_panel(ui, panel, full, &summary);
        } else {
            self.travel.list_hover = None;
        }
    }

    /// Moves the camera along a flight, a fling, or the idle spin.
    fn animate_travel(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        let travel = &mut self.travel;
        let elapsed = ((now - travel.tick) as f32).max(0.0);
        // Slow frames still move at speed; a pause doesn't make a jump.
        let dt = elapsed.min(0.5);
        travel.tick = now;
        if let Some(flight) = &travel.flight {
            let t = ((now - flight.start) / FLIGHT).clamp(0.0, 1.0) as f32;
            travel.camera = flight.at(t);
            if t >= 1.0 {
                travel.flight = None;
            }
            ctx.request_repaint();
        } else if travel.glide != Vec2::ZERO {
            let step = travel.glide * dt;
            if travel.globe {
                turn(&mut travel.camera.facing, step);
            } else {
                travel.camera.pan += step;
            }
            travel.glide *= (-GLIDE_DECAY * dt).exp();
            let stop = if travel.globe { 0.5 } else { 4.0 };
            if travel.glide.length() < stop {
                travel.glide = Vec2::ZERO;
            }
            ctx.request_repaint();
        } else if travel.globe && travel.placed && travel.spin_left > 0.0 && ctx.input(|input| input.focused) {
            travel.camera.facing[0] = wrap_lon(travel.camera.facing[0] + IDLE_SPIN * dt);
            travel.spin_left -= elapsed;
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    fn travel_map(&mut self, ui: &mut egui::Ui, full: Rect, focus: Rect) {
        self.travel.rect = focus;
        let response = ui.interact(full, egui::Id::unique("travel-map"), Sense::click_and_drag());
        let scale = View::of(&self.travel, focus, full).scale;
        let degrees = |by: Vec2| Vec2::new(-(by.x / scale).to_degrees(), (by.y / scale).to_degrees());
        if response.drag_started() || response.clicked() {
            self.travel.touch();
        }
        if response.dragged() {
            let delta = response.drag_delta();
            if self.travel.globe {
                turn(&mut self.travel.camera.facing, degrees(delta));
            } else {
                self.travel.camera.pan += delta;
            }
        }
        if response.drag_stopped() {
            let velocity = ui.input(|input| input.pointer.velocity());
            let (glide, most) = if self.travel.globe { (degrees(velocity), 240.0) } else { (velocity, 3000.0) };
            self.travel.glide = glide.clamp(Vec2::splat(-most), Vec2::splat(most));
        }
        if response.hovered() {
            let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
            let factor = (scroll * 0.003).exp() * pinch;
            if (factor - 1.0).abs() > 1e-4 {
                self.travel.touch();
                let pointer = response.hover_pos().unwrap_or(focus.center());
                self.zoom_travel_map(focus, factor, pointer);
            }
        }
        if response.double_clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                self.travel.touch();
                self.zoom_travel_map(focus, 2.0, pointer);
            }
        }
        self.clamp_travel_pan(focus);

        let view = View::of(&self.travel, focus, full);
        let pointer = response.hover_pos().filter(|_| !response.dragged());
        let map_hover = pointer.and_then(|pos| view.lon_lat(pos)).and_then(|(lon, lat)| world().area_at(lat, lon));
        let key = PaintKey {
            rect: full,
            focus,
            globe: self.travel.globe,
            camera: self.travel.camera,
            epoch: self.travel.visits.as_ref().map_or(0, |visits| visits.epoch),
            year: self.travel.year,
            hover: map_hover.or(self.travel.list_hover),
            selected: self.travel.selected,
            dark: theme::is_dark(),
        };
        if self.travel.painted.as_ref().is_none_or(|painted| painted.key != key) {
            let shapes = paint_map(&self.travel, view, key.hover, key.selected);
            self.travel.painted = Some(Painted { key, shapes });
        }
        let painter = ui.painter_at(full);
        if let Some(painted) = &self.travel.painted {
            painter.extend(painted.shapes.iter().cloned());
        }

        let Some(area) = map_hover else {
            if response.clicked() {
                self.travel.selected = None;
            }
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let response = response.on_hover_ui_at_pointer(|ui| self.travel_tooltip(ui, area));
        if response.clicked() {
            self.travel.selected = Some(area);
            if !self.travel.panel {
                self.travel.panel = true;
                self.config.set("travel_panel", true);
            }
        }
    }

    /// Zooms by `factor`, keeping the place under `pointer` there on the
    /// flat map.
    fn zoom_travel_map(&mut self, focus: Rect, factor: f32, pointer: Pos2) {
        let camera = &mut self.travel.camera;
        let zoom = (camera.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let applied = zoom / camera.zoom;
        if !self.travel.globe {
            let from_middle = pointer - focus.center();
            camera.pan = from_middle - (from_middle - camera.pan) * applied;
        }
        camera.zoom = zoom;
    }

    /// Keeps some of the flat map in sight.
    fn clamp_travel_pan(&mut self, focus: Rect) {
        if self.travel.globe {
            return;
        }
        let half = Vec2::new(FLAT_EXTENT[0], FLAT_EXTENT[1]) * flat_fit(focus) * self.travel.camera.zoom + focus.size() * 0.5 - Vec2::splat(60.0);
        self.travel.camera.pan = self.travel.camera.pan.clamp(-half.max(Vec2::ZERO), half.max(Vec2::ZERO));
    }

    /// What floats on the map: the view switch and the years at the top,
    /// the legend and the zoom buttons at the bottom.
    fn travel_controls(&mut self, ui: &mut egui::Ui, full: Rect, focus: Rect, summary: &Summary) {
        let now = ui.input(|input| input.time);
        let top = full.top() + INSET;
        let (switch, side) = segmented(ui, Pos2::new(full.left() + INSET, top), [tr("Globe"), tr("Flat")], usize::from(!self.travel.globe));
        if let Some(side) = side {
            let globe = side == 0;
            if globe != self.travel.globe {
                self.travel.globe = globe;
                self.travel.flight = None;
                self.travel.glide = Vec2::ZERO;
                self.travel.camera = self.travel.home();
                self.config.set("travel_globe", globe);
            }
        }

        let mut room = focus.right() - INSET - (switch.right() + 8.0);
        if !self.travel.panel {
            let (pill, clicked) = panel_pill(ui, Pos2::new(full.right() - INSET, top), &country_count(summary.visited()));
            room = pill.left() - 8.0 - (switch.right() + 8.0);
            if clicked {
                self.travel.panel = true;
                self.config.set("travel_panel", true);
            }
        }
        if let Some(year) = self.year_chips(ui, Pos2::new(switch.right() + 8.0, top), room) {
            self.travel.year = year;
        }

        let zoom = Rect::from_min_size(Pos2::new(focus.right() - INSET - 36.0, full.bottom() - INSET - 3.0 * 34.0), Vec2::new(36.0, 3.0 * 34.0));
        let factor = match zoom_pill(ui, zoom) {
            Some(ZoomAction::In) => 2.0,
            Some(ZoomAction::Out) => 0.5,
            Some(ZoomAction::Reset) => {
                let home = self.travel.home();
                self.travel.fly_to(home, now);
                1.0
            }
            None => 1.0,
        };
        if factor != 1.0 {
            // A press during a flight zooms from where it is headed.
            let from = self.travel.flight.as_ref().map_or(self.travel.camera, |flight| flight.to);
            let zoom = (from.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            let to = Camera { zoom, pan: from.pan * (zoom / from.zoom), ..from };
            self.travel.fly_to(to, now);
        }

        let most = self.travel.tally().map_or(0, |tally| tally.most);
        if most > 0 {
            legend(ui, Pos2::new(full.left() + INSET, full.bottom() - INSET), most);
        } else if !self.loading && self.travel.visits.as_ref().is_some_and(|visits| visits.all.countries == 0) {
            let hint = tr("Places turn colored where your geotagged photos were taken.");
            let width = (focus.width() - 2.0 * INSET).min(420.0);
            let galley = ui.painter().layout(hint.to_string(), FontId::proportional(13.0), theme::text(), width - 32.0);
            let card = Rect::from_center_size(
                Pos2::new(focus.center().x, full.bottom() - INSET - (galley.size().y + 24.0) / 2.0),
                Vec2::new(galley.size().x + 32.0, galley.size().y + 24.0),
            );
            float_card(ui.painter(), card, 12);
            ui.painter().galley(card.min + Vec2::new(16.0, 12.0), galley, theme::text());
        }
    }

    /// The All years chip and one per year. Returns the year chosen. A year
    /// stays chosen only while its chips can be seen, to be cleared again.
    fn year_chips(&self, ui: &mut egui::Ui, at: Pos2, room: f32) -> Option<Option<i32>> {
        let hidden = self.travel.year.map(|_| None);
        let Some(visits) = self.travel.visits.as_ref() else { return hidden };
        if visits.years.len() < 2 {
            return hidden;
        }
        let mut chips: Vec<(Option<i32>, String, usize)> = vec![(None, tr("All years").to_string(), visits.all.countries)];
        chips.extend(visits.years.iter().map(|(year, tally)| (Some(*year), year.to_string(), tally.countries)));
        let font = FontId::proportional(12.5);
        let widths: Vec<f32> = chips.iter().map(|(_, label, _)| ui.painter().layout_no_wrap(label.clone(), font.clone(), theme::text()).size().x + 22.0).collect();
        let content = widths.iter().sum::<f32>() + 4.0 * (chips.len() - 1) as f32 + 8.0;
        let width = content.min(room);
        if width < 90.0 {
            return hidden;
        }
        let card = Rect::from_min_size(at, Vec2::new(width, 32.0));
        ui.interact(card, egui::Id::unique("travel-years-back"), Sense::click_and_drag());
        float_card(ui.painter(), card, 16);
        let mut bar = ui.new_child(UiBuilder::new().max_rect(card.shrink(4.0)).layout(Layout::left_to_right(Align::Center)));
        bar.set_clip_rect(card.shrink(2.0));
        let mut chosen = None;
        egui::ScrollArea::horizontal()
            .id_salt("travel-years")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut bar, |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.horizontal(|ui| {
                    for ((year, label, countries), width) in chips.iter().zip(&widths) {
                        let response = chip(ui, label, *width, self.travel.year == *year);
                        if response.on_hover_text(country_count(*countries)).clicked() {
                            chosen = Some(*year);
                        }
                    }
                });
            });
        chosen
    }

    fn travel_tooltip(&self, ui: &mut egui::Ui, area: usize) {
        let world = world();
        ui.label(RichText::new(world.areas[area].name()).strong().color(theme::text()));
        if let Some(parent) = world.areas[area].parent {
            ui.label(RichText::new(world.areas[parent].name()).size(12.0).color(theme::muted()));
        }
        let photos = self.travel.photos(area);
        if photos == 0 {
            ui.label(RichText::new(tr("No photos here")).size(12.0).color(theme::muted()));
        } else {
            ui.label(RichText::new(super::photo_count(photos)).size(12.0).color(theme::text()));
        }
        ui.label(RichText::new(tr("Click for details")).size(11.5).color(theme::muted()));
    }

    /// The panel beside the map: what is chosen, the totals, the continents,
    /// and the countries.
    fn travel_panel(&mut self, ui: &mut egui::Ui, panel: Rect, full: Rect, summary: &Summary) {
        ui.interact(panel, egui::Id::unique("travel-panel-back"), Sense::click_and_drag());
        float_card(&ui.painter_at(full), panel, 14);
        let inner = panel.shrink2(Vec2::new(16.0, 12.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        child.set_clip_rect(panel.intersect(full));
        let mut hide = false;
        child.horizontal(|ui| {
            ui.label(RichText::new(tr("Your travels")).size(15.0).strong().color(theme::text()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = ui.add(egui::Button::new(RichText::new("›").size(20.0).color(theme::muted())).frame(false));
                hide = button.on_hover_text(tr("Hide panel")).clicked();
            });
        });
        child.add_space(6.0);
        let mut scroll = egui::ScrollArea::vertical().id_salt("travel-side").auto_shrink([false, false]);
        if self.travel.selected != self.travel.card_for {
            self.travel.card_for = self.travel.selected;
            if self.travel.selected.is_some() {
                scroll = scroll.vertical_scroll_offset(0.0);
            }
        }
        scroll.show(&mut child, |ui| {
            self.travel_side(ui, summary);
        });
        if hide {
            self.travel.panel = false;
            self.config.set("travel_panel", false);
        }
    }

    fn travel_side(&mut self, ui: &mut egui::Ui, summary: &Summary) {
        let now = ui.input(|input| input.time);
        self.travel.list_hover = None;
        // Room for the scroll bar, which floats over the right edge.
        ui.set_max_width(ui.available_width() - 10.0);
        let width = ui.available_width();
        if let Some(area) = self.travel.selected {
            self.area_card(ui, area);
            ui.add_space(14.0);
        }
        hero(ui, summary, width);

        ui.add_space(16.0);
        heading(ui, tr("Coverage by continent"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for (index, name) in CONTINENTS.iter().enumerate() {
                let (visited, total) = summary.continents[index];
                let active = self.travel.continent == Some(index);
                if continent_chip(ui, tr(name), visited, total, active).clicked() {
                    self.travel.continent = if active { None } else { Some(index) };
                }
            }
        });

        if summary.countries.is_empty() {
            ui.add_space(16.0);
            return;
        }
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            heading(ui, tr("Your countries"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(grouped(summary.visited())).size(12.0).color(theme::muted()));
            });
        });
        ui.add(egui::TextEdit::singleline(&mut self.travel.search).hint_text(tr("Search countries")).desired_width(f32::INFINITY));
        ui.add_space(4.0);
        let query = self.travel.search.trim().to_lowercase();
        let world = world();
        let rows: Vec<(usize, usize)> = summary
            .countries
            .iter()
            .copied()
            .filter(|&(country, _)| self.travel.continent.is_none_or(|continent| world.areas[country].continent == continent))
            .filter(|&(country, _)| query.is_empty() || world.areas[country].name().to_lowercase().contains(&query))
            .collect();
        if rows.is_empty() {
            ui.label(RichText::new(tr("No countries match")).size(12.0).color(theme::muted()));
        }
        let Some(visits) = self.travel.visits.clone() else { return };
        let most = summary.countries[0].1;
        let chosen = self.travel.selected.map(|area| world.country_of(area));
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut focus = None;
        for (country, photos) in rows {
            let years = years_text(&visits.years_of(country));
            let response = country_row(ui, world.areas[country].name(), &years, shade(photos, most), photos, photos as f32 / most as f32, chosen == Some(country));
            if response.hovered() {
                self.travel.list_hover = Some(country);
            }
            if response.clicked() {
                focus = Some(country);
            }
        }
        if let Some(country) = focus {
            self.travel.selected = Some(country);
            self.focus_travel_area(country, now);
        }
        ui.add_space(16.0);
    }

    /// The chosen country or state: its photos, when they were taken, and a
    /// way to see them.
    fn area_card(&mut self, ui: &mut egui::Ui, area: usize) {
        let world = world();
        let Some(visits) = self.travel.visits.clone() else { return };
        let photos = self.travel.photos(area);
        let (mut close, mut show) = (false, None);
        egui::Frame::new()
            .fill(theme::bg())
            .stroke(Stroke::new(1.0, theme::hairline()))
            .corner_radius(12)
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let name = truncated(ui.painter(), world.areas[area].name().to_string(), 17.0, theme::text(), ui.available_width() - 28.0);
                    ui.label(name);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let button = ui.add(egui::Button::new(RichText::new("×").size(18.0).color(theme::muted())).frame(false));
                        close = button.on_hover_text(tr("Close")).clicked();
                    });
                });
                let under = match world.areas[area].parent {
                    Some(parent) => world.areas[parent].name().to_string(),
                    None => tr(CONTINENTS[world.areas[area].continent]).to_string(),
                };
                ui.label(RichText::new(under).size(12.0).color(theme::muted()));
                ui.add_space(6.0);
                let count = match self.travel.year {
                    Some(year) => format!("{} · {}", super::photo_count(photos), year),
                    None => super::photo_count(photos),
                };
                ui.label(RichText::new(count).size(14.0).strong().color(if photos > 0 { theme::text() } else { theme::muted() }));
                if let Some((first, last)) = visits.span[area] {
                    ui.label(RichText::new(date_span([first, last].into_iter())).size(12.0).color(theme::muted()));
                }
                let years = visits.years_of(area);
                if years.len() > 1 {
                    ui.label(RichText::new(trf("Visited in {}", &[&years_text(&years)])).size(12.0).color(theme::muted()));
                }
                let children = &world.children[area];
                if !children.is_empty() {
                    let seen = children.iter().filter(|&&child| self.travel.photos(child) > 0).count();
                    ui.label(RichText::new(trf("{} of {} states and provinces", &[&seen, &children.len()])).size(12.0).color(theme::muted()));
                }
                if visits.all.photos[area] > 0 {
                    ui.add_space(8.0);
                    if ui.add(theme::primary_button(tr("Show photos"))).on_hover_text(tr("Click to see these photos on the map")).clicked() {
                        show = Some(visits.bounds[area]);
                    }
                }
            });
        if close {
            self.travel.selected = None;
        }
        if let Some(bounds) = show {
            self.show_area_on_map(bounds);
        }
    }

    /// Turns or moves the map to `area` and zooms to about its size.
    fn focus_travel_area(&mut self, area: usize, now: f64) {
        let area = &world().areas[area];
        let bounds = area.bounds;
        let squeeze = area.label[1].to_radians().cos().max(0.2);
        let extent = ((bounds.east - bounds.west) * squeeze).max(bounds.north - bounds.south).max(1.0);
        let span = if self.travel.globe { 70.0 } else { 120.0 };
        let zoom = (span / extent).clamp(1.0, MAX_ZOOM);
        let scale = flat_fit(self.travel.rect) * zoom;
        let [x, y] = world::natural_earth(area.label[0], area.label[1]);
        let to = Camera { facing: area.label, zoom, pan: Vec2::new(-x * scale, y * scale) };
        self.travel.fly_to(to, now);
    }

    /// The Travel card of the Statistics page: the share of the world and
    /// each continent for the photos in view, or one year's. Returns
    /// whether to open the map.
    pub(super) fn travel_card(&mut self, ui: &mut egui::Ui, visits: &Visits, year: Option<i32>) -> bool {
        let tally = match year {
            Some(year) => visits.years.iter().find(|(each, _)| *each == year).map(|(_, tally)| tally),
            None => Some(&visits.all),
        };
        let summary = Summary::of(tally);
        let width = ui.available_width();
        hero(ui, &summary, width);
        ui.add_space(12.0);
        let count_w = 44.0;
        let name_w = (width * 0.38).min(150.0);
        for (index, continent) in CONTINENTS.iter().enumerate() {
            let (visited, total) = summary.continents[index];
            let (row, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
            let painter = ui.painter();
            let color = if visited > 0 { theme::text() } else { theme::muted() };
            let name = truncated(painter, tr(continent).to_string(), 12.0, color, name_w - 8.0);
            painter.galley(Pos2::new(row.left(), row.center().y - name.size().y / 2.0), name, color);
            let bar = Rect::from_min_max(Pos2::new(row.left() + name_w, row.center().y - 3.0), Pos2::new(row.right() - count_w, row.center().y + 3.0));
            meter(painter, bar, visited as f32 / total.max(1) as f32, theme::accent());
            painter.text(
                Pos2::new(row.right(), row.center().y),
                egui::Align2::RIGHT_CENTER,
                format!("{visited}/{total}"),
                FontId::proportional(11.5),
                theme::muted(),
            );
        }
        ui.add_space(12.0);
        ui.add(theme::primary_button(tr("Open Travel Map"))).clicked()
    }
}

/// Fill of what floats on the map: the side bar color, a touch see-through.
fn float_fill() -> Color32 {
    let color = theme::sidebar();
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 242)
}

fn float_card(painter: &egui::Painter, rect: Rect, radius: u8) {
    painter.add(theme::card_shadow().as_shape(rect, radius));
    painter.rect(rect, radius, float_fill(), Stroke::new(1.0, theme::hairline()), StrokeKind::Inside);
}

/// A switch between two labels: a pill with the chosen side raised.
/// Returns where it is and the side clicked.
fn segmented(ui: &mut egui::Ui, at: Pos2, labels: [&str; 2], chosen: usize) -> (Rect, Option<usize>) {
    let font = FontId::proportional(13.0);
    let widths: Vec<f32> = labels.iter().map(|label| ui.painter().layout_no_wrap(label.to_string(), font.clone(), theme::text()).size().x + 28.0).collect();
    let height = 32.0;
    let rect = Rect::from_min_size(at, Vec2::new(widths.iter().sum::<f32>() + 8.0, height));
    ui.interact(rect, egui::Id::unique("travel-switch-back"), Sense::click_and_drag());
    float_card(ui.painter(), rect, 16);
    let side = |index: usize| {
        Rect::from_min_size(Pos2::new(rect.left() + 4.0 + widths[..index].iter().sum::<f32>(), rect.top() + 4.0), Vec2::new(widths[index], height - 8.0))
    };
    let t = ui.ctx().animate_value_with_time(egui::Id::unique("travel-switch"), chosen as f32, 0.18);
    let knob = Rect::from_min_max(side(0).min.lerp(side(1).min, t), side(0).max.lerp(side(1).max, t));
    let knob_fill = if theme::is_dark() { Color32::from_white_alpha(30) } else { WHITE };
    ui.painter().add(theme::card_shadow().as_shape(knob, 12));
    ui.painter().rect_filled(knob, 12, knob_fill);
    let mut clicked = None;
    for (index, label) in labels.iter().enumerate() {
        let area = side(index);
        let response = ui.interact(area, egui::Id::unique(("travel-switch", index)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            clicked = Some(index);
        }
        let color = if index == chosen || response.hovered() { theme::text() } else { theme::muted() };
        let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), color);
        ui.painter().galley(area.center() - galley.size() / 2.0, galley, color);
    }
    (rect, clicked)
}

/// The pill that opens the hidden panel, right-aligned at `right_top`.
/// Returns where it is and whether it was clicked.
fn panel_pill(ui: &mut egui::Ui, right_top: Pos2, label: &str) -> (Rect, bool) {
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(13.0), theme::text());
    let size = Vec2::new(galley.size().x + 44.0, 32.0);
    let rect = Rect::from_min_size(Pos2::new(right_top.x - size.x, right_top.y), size);
    let response = ui.interact(rect, egui::Id::unique("travel-panel-pill"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    float_card(ui.painter(), rect, 16);
    if response.hovered() {
        ui.painter().rect_filled(rect.shrink(1.0), 16, theme::hover());
    }
    let chevron = ui.painter().layout_no_wrap("‹".into(), FontId::proportional(18.0), theme::muted());
    ui.painter().galley(Pos2::new(rect.left() + 14.0, rect.center().y - chevron.size().y / 2.0), chevron, theme::muted());
    ui.painter().galley(Pos2::new(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, theme::text());
    let clicked = response.on_hover_text(tr("Show panel")).clicked();
    (rect, clicked)
}

/// A rounded label that can be chosen, `width` wide.
fn chip(ui: &mut egui::Ui, label: &str, width: f32, active: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    let color = if active {
        painter.rect_filled(rect, 12, ACCENT);
        WHITE
    } else {
        if response.hovered() {
            painter.rect_filled(rect, 12, theme::hover());
        }
        theme::text()
    };
    let galley = painter.layout_no_wrap(label.to_string(), FontId::proportional(12.5), color);
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

enum ZoomAction {
    In,
    Out,
    Reset,
}

/// Zoom in, zoom out, and back to the whole world, stacked in a pill.
fn zoom_pill(ui: &mut egui::Ui, rect: Rect) -> Option<ZoomAction> {
    ui.interact(rect, egui::Id::unique("travel-zoom-back"), Sense::click_and_drag());
    float_card(ui.painter(), rect, 12);
    let step = rect.height() / 3.0;
    let mut action = None;
    for (index, (tip, what)) in [("Zoom in", ZoomAction::In), ("Zoom out", ZoomAction::Out), ("Reset view", ZoomAction::Reset)].into_iter().enumerate() {
        let button = Rect::from_min_size(rect.min + Vec2::new(0.0, index as f32 * step), Vec2::new(rect.width(), step));
        let response = ui.interact(button, egui::Id::unique(("travel-zoom", index)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(button.shrink(3.0), 8, theme::hover());
        }
        if index > 0 {
            painter.hline(button.x_range().shrink(8.0), button.top(), Stroke::new(1.0, theme::hairline()));
        }
        let (center, ink) = (button.center(), Stroke::new(1.6, theme::text()));
        match what {
            ZoomAction::In => {
                painter.hline(center.x - 6.0..=center.x + 6.0, center.y, ink);
                painter.vline(center.x, center.y - 6.0..=center.y + 6.0, ink);
            }
            ZoomAction::Out => {
                painter.hline(center.x - 6.0..=center.x + 6.0, center.y, ink);
            }
            ZoomAction::Reset => {
                painter.circle_stroke(center, 6.5, ink);
                painter.circle_filled(center, 2.0, theme::text());
            }
        }
        if response.on_hover_text(tr(tip)).clicked() {
            action = Some(what);
        }
    }
    action
}

/// How the photo counts are shaded, in a card whose bottom left is `at`.
fn legend(ui: &mut egui::Ui, at: Pos2, most: usize) {
    let rect = Rect::from_min_size(Pos2::new(at.x, at.y - 54.0), Vec2::new(190.0, 54.0));
    ui.interact(rect, egui::Id::unique("travel-legend"), Sense::click_and_drag());
    let painter = ui.painter();
    float_card(painter, rect, 12);
    let inner = rect.shrink2(Vec2::new(12.0, 8.0));
    let title = painter.layout_no_wrap(tr("Photos").to_string(), FontId::proportional(11.5), theme::muted());
    painter.galley(inner.min, title, theme::muted());
    let bar = Rect::from_min_size(Pos2::new(inner.left(), inner.top() + 18.0), Vec2::new(inner.width(), 7.0));
    const STOPS: u32 = 8;
    let mut mesh = Mesh::default();
    for stop in 0..=STOPS {
        let t = stop as f32 / STOPS as f32;
        let color = if most <= 1 { MANY } else { ramp(t) };
        let x = bar.left() + bar.width() * t;
        mesh.colored_vertex(Pos2::new(x, bar.top()), color);
        mesh.colored_vertex(Pos2::new(x, bar.bottom()), color);
        if stop > 0 {
            let i = 2 * stop;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i, i + 1);
        }
    }
    painter.add(mesh);
    let low = painter.layout_no_wrap("1".into(), FontId::proportional(11.0), theme::muted());
    let high = painter.layout_no_wrap(grouped(most), FontId::proportional(11.0), theme::muted());
    painter.galley(Pos2::new(inner.left(), inner.bottom() - low.size().y), low, theme::muted());
    painter.galley(Pos2::new(inner.right() - high.size().x, inner.bottom() - high.size().y), high, theme::muted());
}

/// The share of the world in a ring, the countries beside it, and the way
/// to the next goal.
fn hero(ui: &mut egui::Ui, summary: &Summary, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 76.0), Sense::hover());
    let painter = ui.painter();
    let center = Pos2::new(rect.left() + 36.0, rect.center().y);
    let share = summary.visited() as f32 / summary.total.max(1) as f32;
    ring(painter, center, 31.0, share);
    let figure = painter.layout_no_wrap(format!("{}%", percent(summary.visited(), summary.total)), FontId::proportional(15.0), theme::text());
    painter.galley(center - figure.size() / 2.0, figure, theme::text());
    let left = rect.left() + 86.0;
    let room = rect.right() - left;
    let count = painter.layout_no_wrap(grouped(summary.visited()), FontId::proportional(28.0), theme::text());
    let count_h = count.size().y;
    painter.galley(Pos2::new(left, rect.top() + 4.0), count, theme::text());
    let label = truncated(painter, tr("Countries visited").to_string(), 12.5, theme::text(), room);
    painter.galley(Pos2::new(left, rect.top() + 4.0 + count_h), label, theme::text());
    let of = truncated(painter, trf("of {} countries", &[&summary.total]), 11.5, theme::muted(), room);
    painter.galley(Pos2::new(left, rect.top() + 22.0 + count_h), of, theme::muted());

    ui.add_space(8.0);
    match summary.next_milestone() {
        Some(next) => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(trf("Next goal · {} to go", &[&(next - summary.visited())])).size(12.0).color(theme::muted()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} / {}", summary.visited(), next)).size(12.0).color(theme::text()));
                });
            });
            let (bar, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
            meter(ui.painter(), bar, summary.visited() as f32 / next as f32, theme::accent());
        }
        None => {
            ui.label(RichText::new(format!("✓ {}", tr("Every country"))).size(12.5).color(theme::text()));
        }
    }

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let half = (width - 10.0) / 2.0;
        ui.spacing_mut().item_spacing.x = 10.0;
        for (value, label) in [
            (grouped(summary.regions), tr("States and provinces")),
            (format!("{}/{}", summary.continents_started(), CONTINENTS.len()), tr("Continents")),
        ] {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(half, 50.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 10, theme::overlay(10));
            let inner = rect.shrink2(Vec2::new(10.0, 7.0));
            painter.galley(inner.min, painter.layout_no_wrap(value, FontId::proportional(16.0), theme::text()), theme::text());
            let label = truncated(painter, label.to_string(), 11.0, theme::muted(), inner.width());
            painter.galley(Pos2::new(inner.left(), inner.bottom() - label.size().y), label, theme::muted());
        }
    });
}

/// A progress ring around `center`, filled `share` of the way from the top.
fn ring(painter: &egui::Painter, center: Pos2, radius: f32, share: f32) {
    let width = 6.0;
    painter.circle_stroke(center, radius, Stroke::new(width, theme::well()));
    if share <= 0.0 {
        return;
    }
    let sweep = std::f32::consts::TAU * share.clamp(0.01, 1.0);
    let steps = (sweep * 12.0).ceil().max(2.0) as usize;
    let points: Vec<Pos2> = (0..=steps)
        .map(|step| {
            let angle = -std::f32::consts::FRAC_PI_2 + sweep * step as f32 / steps as f32;
            center + Vec2::angled(angle) * radius
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(width, theme::accent())));
}

/// A continent with how many of its countries have photos, and a thin bar
/// for the share. Returns its response.
fn continent_chip(ui: &mut egui::Ui, name: &str, visited: usize, total: usize, active: bool) -> Response {
    let font = FontId::proportional(12.0);
    let title = ui.painter().layout_no_wrap(name.to_string(), font, theme::text());
    let count = ui.painter().layout_no_wrap(format!("{visited}/{total}"), FontId::proportional(11.0), theme::muted());
    let size = Vec2::new(title.size().x + count.size().x + 30.0, 34.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    let (fill, stroke) = if active {
        (theme::accent().gamma_multiply(0.14), Stroke::new(1.2, theme::accent()))
    } else if response.hovered() {
        (theme::hover(), Stroke::new(1.0, theme::hairline()))
    } else {
        (theme::bg(), Stroke::new(1.0, theme::hairline()))
    };
    painter.rect(rect, 10, fill, stroke, StrokeKind::Inside);
    let y = rect.top() + 7.0;
    painter.galley(Pos2::new(rect.left() + 11.0, y), title.clone(), theme::text());
    painter.galley(Pos2::new(rect.right() - 11.0 - count.size().x, y + (title.size().y - count.size().y) / 2.0), count, theme::muted());
    let bar = Rect::from_min_size(Pos2::new(rect.left() + 11.0, rect.bottom() - 9.0), Vec2::new(rect.width() - 22.0, 3.0));
    meter(painter, bar, visited as f32 / total.max(1) as f32, theme::accent());
    response
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).strong().color(theme::muted()));
    ui.add_space(2.0);
}

/// A country in the list: its color, name, the years it was visited, its
/// photos, and a thin bar for its share of the most any country has.
fn country_row(ui: &mut egui::Ui, name: &str, years: &str, color: Color32, photos: usize, share: f32, selected: bool) -> Response {
    let height = if years.is_empty() { 34.0 } else { 46.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 8, theme::select());
    } else if response.hovered() {
        painter.rect_filled(rect, 8, theme::hover());
    }
    let top = rect.top() + 7.0;
    painter.circle_filled(Pos2::new(rect.left() + 12.0, top + 8.0), 5.0, color);
    let figure = painter.layout_no_wrap(grouped(photos), FontId::proportional(11.5), theme::muted());
    let room = rect.width() - 34.0 - figure.size().x - 8.0;
    let title = truncated(painter, name.to_string(), 13.0, theme::text(), room);
    painter.galley(Pos2::new(rect.left() + 26.0, top), title, theme::text());
    painter.galley(Pos2::new(rect.right() - 8.0 - figure.size().x, top + 1.0), figure, theme::muted());
    if !years.is_empty() {
        let line = truncated(painter, years.to_string(), 11.0, theme::muted(), rect.width() - 34.0);
        painter.galley(Pos2::new(rect.left() + 26.0, top + 18.0), line, theme::muted());
    }
    let bar = Rect::from_min_size(Pos2::new(rect.left() + 26.0, rect.bottom() - 6.0), Vec2::new(rect.width() - 34.0, 2.5));
    painter.rect_filled(bar, 1, theme::overlay(12));
    painter.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * share.clamp(0.02, 1.0), bar.height())), 1, color);
    response
}

/// Builds the map: the backdrop, the sea, the graticule, land colored by
/// photos, the globe's shading, borders, and the outlines of the places
/// pointed at and chosen.
fn paint_map(travel: &Travel, view: View, hover: Option<usize>, selected: Option<usize>) -> Vec<Shape> {
    let world = world();
    let dark = theme::is_dark();
    let sea = if dark { Color32::from_rgb(0x0F, 0x2D, 0x35) } else { Color32::from_rgb(0xCF, 0xEA, 0xEC) };
    let rim = if dark { Color32::from_rgb(0x1E, 0x4A, 0x53) } else { Color32::from_rgb(0xA9, 0xD3, 0xD8) };
    let grid = Stroke::new(0.6, if dark { Color32::from_white_alpha(16) } else { Color32::from_rgba_unmultiplied(0x1F, 0x6F, 0x78, 34) });
    let border = if dark { Color32::from_rgb(0x0B, 0x1B, 0x20) } else { Color32::from_rgb(0xFF, 0xFC, 0xF6) };
    let most = travel.tally().map_or(0, |tally| tally.most);
    let mut shapes = Vec::new();

    let (top, bottom) = if dark {
        (Color32::from_rgb(0x0C, 0x17, 0x1B), Color32::from_rgb(0x06, 0x0C, 0x0F))
    } else {
        (Color32::from_rgb(0xF7, 0xF8, 0xF4), Color32::from_rgb(0xE2, 0xEC, 0xEB))
    };
    shapes.push(Shape::Mesh(Arc::new(vertical_gradient(view.rect, top, bottom))));
    match view.globe {
        Some(_) => {
            let glow = if dark { Color32::from_rgb(0x1F, 0xA3, 0xB0) } else { Color32::from_rgb(0x8C, 0xD3, 0xDA) };
            shapes.push(Shape::Mesh(Arc::new(halo(view.origin, view.scale, glow.gamma_multiply(if dark { 0.32 } else { 0.6 })))));
            shapes.push(Shape::circle_filled(view.origin, view.scale, sea));
        }
        None => {
            let mut edge: Vec<Pos2> = (-90..=90).rev().step_by(5).map(|lat| view.at(world::natural_earth(180.0, lat as f32))).collect();
            edge.extend((-90..=90).step_by(5).map(|lat| view.at(world::natural_earth(-180.0, lat as f32))));
            shapes.push(Shape::convex_polygon(edge, sea, Stroke::new(1.0, rim)));
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
            color = if photos > 0 { color.lerp_to_gamma(Color32::WHITE, 0.3) } else { color.lerp_to_gamma(theme::accent(), 0.25) };
        }
        fill_area(&view, area, color, &mut mesh, &mut projected);
    }
    shapes.push(Shape::Mesh(Arc::new(mesh)));
    if view.globe.is_some() {
        shapes.push(Shape::Mesh(Arc::new(shading(view.origin, view.scale, dark))));
    }

    let mut regions = Vec::new();
    let mut countries = Vec::new();
    for area in &world.areas {
        if !visible(&view, area) {
            continue;
        }
        let out = if area.parent.is_some() { &mut regions } else { &mut countries };
        outline(&view, area, out);
    }
    let thin = Stroke::new(0.5, border.gamma_multiply(0.7));
    let thick = Stroke::new(if view.scale > 900.0 { 1.2 } else { 0.8 }, border);
    shapes.extend(regions.into_iter().map(|line| Shape::line(line, thin)));
    shapes.extend(countries.into_iter().map(|line| Shape::line(line, thick)));

    if let Some(area) = hover.filter(|area| Some(*area) != selected) {
        let mut lines = Vec::new();
        outline(&view, &world.areas[area], &mut lines);
        shapes.extend(lines.into_iter().map(|line| Shape::line(line, Stroke::new(1.5, theme::accent().gamma_multiply(0.7)))));
    }
    if let Some(area) = selected {
        let mut lines = Vec::new();
        outline(&view, &world.areas[area], &mut lines);
        let under = Stroke::new(4.5, border.gamma_multiply(0.85));
        shapes.extend(lines.iter().map(|line| Shape::line(line.clone(), under)));
        shapes.extend(lines.into_iter().map(|line| Shape::line(line, Stroke::new(2.2, theme::accent()))));
    }
    if view.globe.is_some() {
        shapes.push(Shape::circle_stroke(view.origin, view.scale, Stroke::new(1.0, rim)));
    }
    shapes
}

/// A rectangle shaded from `top` to `bottom`.
fn vertical_gradient(rect: Rect, top: Color32, bottom: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
}

/// The glow of the atmosphere round a globe of `radius`, fading outward.
fn halo(center: Pos2, radius: f32, color: Color32) -> Mesh {
    const STEPS: u32 = 128;
    let mut mesh = Mesh::default();
    for step in 0..STEPS {
        let direction = Vec2::angled(std::f32::consts::TAU * step as f32 / STEPS as f32);
        mesh.colored_vertex(center + direction * radius * 0.98, color);
        mesh.colored_vertex(center + direction * radius * 1.14, Color32::TRANSPARENT);
    }
    for step in 0..STEPS {
        let (a, b) = (2 * step, 2 * ((step + 1) % STEPS));
        mesh.add_triangle(a, a + 1, b + 1);
        mesh.add_triangle(a, b + 1, b);
    }
    mesh
}

/// Light on the globe from the upper left: a faint shine there and a
/// darker rim, so it reads as a ball.
fn shading(center: Pos2, radius: f32, dark: bool) -> Mesh {
    const STEPS: u32 = 96;
    let light = center + Vec2::new(-0.32, -0.38) * radius;
    let (shine, edge) = if dark { (Color32::from_white_alpha(10), Color32::from_black_alpha(110)) } else { (Color32::from_white_alpha(26), Color32::from_black_alpha(44)) };
    let mut mesh = Mesh::default();
    mesh.colored_vertex(light, shine);
    for step in 0..STEPS {
        let rim = center + Vec2::angled(std::f32::consts::TAU * step as f32 / STEPS as f32) * radius;
        mesh.colored_vertex(light + (rim - light) * 0.55, Color32::TRANSPARENT);
        mesh.colored_vertex(rim, edge);
    }
    for step in 0..STEPS {
        let (mid, rim) = (1 + 2 * step, 2 + 2 * step);
        let (next_mid, next_rim) = (1 + 2 * ((step + 1) % STEPS), 2 + 2 * ((step + 1) % STEPS));
        mesh.add_triangle(0, mid, next_mid);
        mesh.add_triangle(mid, rim, next_rim);
        mesh.add_triangle(mid, next_rim, next_mid);
    }
    mesh
}

/// Adds the visible stretches of the outline of `area` to `out`.
fn outline(view: &View, area: &world::Area, out: &mut Vec<Vec<Pos2>>) {
    for &(start, len) in &area.rings {
        let range = start as usize..(start + len) as usize;
        trace_ring(view, &area.points[range.clone()], &area.sphere[range.clone()], &area.flat[range], out);
    }
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

/// A stone or sage color for a country without photos, the same for its
/// states, so neighbors differ a little.
fn land_color(id: &str, dark: bool) -> Color32 {
    const LIGHT: [(u8, u8, u8); 5] = [(0xEA, 0xE8, 0xE1), (0xE2, 0xE7, 0xDD), (0xE8, 0xE4, 0xE0), (0xDF, 0xE5, 0xE2), (0xE6, 0xE7, 0xDE)];
    const DARK: [(u8, u8, u8); 5] = [(0x33, 0x3A, 0x3C), (0x31, 0x3C, 0x37), (0x3A, 0x37, 0x39), (0x2F, 0x3A, 0x3F), (0x3C, 0x39, 0x31)];
    let hash = id.bytes().fold(0x811C_9DC5u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193));
    let (r, g, b) = if dark { DARK } else { LIGHT }[hash as usize % LIGHT.len()];
    Color32::from_rgb(r, g, b)
}

/// Countries and states sorted by how many photos, for tests.
#[cfg(test)]
fn ranked(tally: &Tally) -> Vec<(&'static str, usize)> {
    let world = world();
    let mut rows: Vec<(&str, usize)> =
        (0..world.areas.len()).filter(|&area| tally.photos[area] > 0).map(|area| (world.areas[area].id.as_str(), tally.photos[area])).collect();
    rows.sort_by_key(|&(id, photos)| (std::cmp::Reverse(photos), id));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn photo(id: i64, lat: f64, lon: f64, year: i32) -> Photo {
        Photo {
            id,
            folder_id: 1,
            path: format!("/p/{id}.jpg"),
            filename: format!("{id}.jpg"),
            width: 1,
            height: 1,
            bytes: 1,
            taken_at: chrono::Local.with_ymd_and_hms(year, 6, 1, 12, 0, 0).unwrap().timestamp(),
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
            photo(1, 25.03, 121.56, 2019),
            photo(2, 25.04, 121.55, 2023),
            photo(3, 22.63, 120.30, 2024),
            photo(4, 34.05, -118.24, 2024),
            photo(5, 40.71, -74.0, 2024),
            photo(6, 0.0, -30.0, 2024),
        ];
        let visits = Visits::compute(&photos, 1);
        assert_eq!(ranked(&visits.all), vec![("TW", 3), ("US", 2), ("US_CA", 1), ("US_NY", 1)]);
        assert_eq!(visits.all.most, 3);
        assert_eq!(visits.all.top_country().map(|country| world().areas[country].id.as_str()), Some("TW"));
        let summary = Summary::of(Some(&visits.all));
        assert_eq!(summary.visited(), 2);
        assert_eq!(summary.regions, 2);
        assert_eq!(summary.continents_started(), 2);
        assert_eq!(summary.next_milestone(), Some(10));
    }

    #[test]
    fn each_year_has_its_own_tally() {
        let photos = vec![photo(1, 25.03, 121.56, 2019), photo(2, 25.04, 121.55, 2023), photo(3, 34.05, -118.24, 2024), photo(4, 22.63, 120.30, 2024)];
        let visits = Visits::compute(&photos, 1);
        let years: Vec<i32> = visits.years.iter().map(|(year, _)| *year).collect();
        assert_eq!(years, [2024, 2023, 2019], "newest first");
        assert_eq!(visits.years[0].1.countries, 2);
        assert_eq!(ranked(&visits.years[1].1), vec![("TW", 1)]);
        let taiwan = world().find_id("TW").unwrap();
        assert_eq!(visits.years_of(taiwan), [2019, 2023, 2024]);
        assert_eq!(years_text(&visits.years_of(taiwan)), "2019, 2023–2024");
        let (first, last) = visits.span[taiwan].unwrap();
        assert!(first < last);
    }

    #[test]
    fn runs_of_years_are_joined() {
        assert_eq!(years_text(&[]), "");
        assert_eq!(years_text(&[2020]), "2020");
        assert_eq!(years_text(&[2018, 2019, 2020, 2023, 2025, 2026]), "2018–2020, 2023, 2025–2026");
    }

    #[test]
    fn flights_ease_and_take_the_short_way_round() {
        let from = Camera { facing: [170.0, 0.0], zoom: 1.0, pan: Vec2::ZERO };
        let to = Camera { facing: [-170.0, 10.0], zoom: 4.0, pan: Vec2::new(100.0, 0.0) };
        let flight = Flight { from, to, start: 0.0, globe: true };
        assert_eq!(flight.at(0.0), from);
        let end = flight.at(1.0);
        assert!((end.facing[0] + 170.0).abs() < 1e-3 && (end.zoom - 4.0).abs() < 1e-3);
        let middle = flight.at(0.5);
        assert!(middle.facing[0].abs() > 175.0, "crosses the date line, not the whole world: {:?}", middle.facing);
    }

    #[test]
    fn more_photos_shade_deeper() {
        assert_eq!(shade(1, 1), MANY);
        assert_eq!(shade(1, 100), FEW);
        assert_eq!(shade(100, 100), MANY);
        assert_eq!(shade(10, 100), theme::GOLD);
        let brightness = |color: Color32| u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b());
        let steps: Vec<u32> = [1, 3, 10, 30, 100].iter().map(|&photos| brightness(shade(photos, 100))).collect();
        assert!(steps.windows(2).all(|pair| pair[0] > pair[1]), "{steps:?}");
    }
}
