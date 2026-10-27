//! Statistics: when, where, and with what the photos in view were taken,
//! over all years or in one. Years, days, months, cameras, and lenses lead
//! to the matching photos in the Library; a place leads to the map.
//!
//! The page is a grid of cards: the totals, highlights with a photo each, a
//! calendar of the year's days, the years or months, the hours of the week,
//! the places, travel, cameras, lenses, and focal lengths.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::f32::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, TimeZone, Timelike};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Margin, Mesh, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Vec2};

use crate::geo;
use crate::i18n::{tr, trf};
use crate::track::{format_distance, Bounds};
use crate::types::{format_bytes, format_day, format_month, short_month, short_weekday, Nav, Photo};

use super::places::{cluster_points, date_span, folder_summary, grouped, truncated};
use super::theme::{self, ACCENT, WHITE};
use super::travel::Visits;
use super::{Mode, PicassoApp};

/// Names a donut shows on its own; the rest share one slice.
const SLICES: usize = 5;
/// Rows of the rest of a ranked list in view at once when it is opened.
const REST_ROWS: f32 = 10.0;
const BAR_ROW_H: f32 = 40.0;
/// Zoom level whose cluster squares, 64 pixels wide, make a place: about
/// 40 km across.
const PLACE_ZOOM: u8 = 8;
/// Upper end of each focal length band, in 35 mm equivalent millimeters,
/// and its label.
const FOCAL_BANDS: [(f32, &str); 7] = [
    (20.0, "≤20"),
    (30.0, "21–30"),
    (40.0, "31–40"),
    (60.0, "41–60"),
    (105.0, "61–105"),
    (200.0, "106–200"),
    (f32::INFINITY, ">200"),
];
/// Widest the page grows; beyond it the cards would only stretch.
const MAX_W: f32 = 1180.0;
const CARD_GAP: f32 = 16.0;
/// Narrower than this, cards stack one under another.
const NARROW: f32 = 760.0;
/// Space inside a card's edge.
const PAD: f32 = 18.0;
/// Seconds the charts take to grow in.
const GROW: f64 = 0.6;
const GREEN: Color32 = theme::GREEN;
const ROSE: Color32 = Color32::from_rgb(0xE0, 0x60, 0x7E);
const SKY: Color32 = Color32::from_rgb(0x3B, 0x8F, 0xD9);
const RED: Color32 = theme::DANGER;

/// Colors of a donut's slices, in order.
fn slice_colors() -> [Color32; SLICES] {
    [theme::accent(), SKY, theme::GOLD, ROSE, theme::ALBUM]
}

/// A photo shown on a card.
#[derive(Clone)]
struct Cover {
    id: i64,
    path: String,
    rotation: i32,
}

impl Cover {
    fn of(photo: &Photo) -> Cover {
        Cover { id: photo.id, path: photo.path.clone(), rotation: photo.rotation }
    }
}

/// The day or month with the most photos, or the first photo: when, how
/// many, and its first photo.
struct Moment {
    at: i64,
    count: usize,
    cover: Cover,
}

/// The most days in a row with photos.
struct Streak {
    days: usize,
    first: i64,
    last: i64,
    cover: Cover,
}

/// What a year holds.
#[derive(Clone, Copy, Default)]
struct Totals {
    photos: usize,
    videos: usize,
    bytes: u64,
    /// Days with photos.
    days: usize,
}

/// Counts over some photos, worked out once.
struct Stats {
    /// When the page first showed these numbers, for the charts to grow in.
    shown_at: Cell<Option<f64>>,
    photos: usize,
    videos: usize,
    bytes: u64,
    /// First and last time a photo was taken.
    span: Option<(i64, i64)>,
    /// Photos per year, oldest first, with the empty years between.
    years: Vec<(i32, usize)>,
    totals: BTreeMap<i32, Totals>,
    months: [usize; 12],
    /// Photos by weekday, Monday first, and hour.
    week_hours: [[usize; 24]; 7],
    /// Photos on each day, by its number from the common era.
    days: HashMap<i32, usize>,
    busiest_day: Option<Moment>,
    busiest_month: Option<Moment>,
    first: Option<Moment>,
    streak: Option<Streak>,
    geotagged: usize,
    /// Every place, biggest first.
    places: Vec<Place>,
    cameras: Ranked,
    lenses: Ranked,
    focal: [usize; FOCAL_BANDS.len()],
    /// Most used focal length in whole millimeters, and how often.
    focal_top: Option<(u32, usize)>,
}

impl Stats {
    /// How far the charts have grown in, from 0 to 1, eased.
    fn grow(&self, ctx: &egui::Context) -> f32 {
        let now = ctx.input(|input| input.time);
        let start = match self.shown_at.get() {
            Some(start) => start,
            None => {
                self.shown_at.set(Some(now));
                now
            }
        };
        let t = ((now - start) / GROW).clamp(0.0, 1.0) as f32;
        if t < 1.0 {
            ctx.request_repaint();
        }
        1.0 - (1.0 - t).powi(3)
    }
}

struct Place {
    /// Folder most of its photos are in.
    label: String,
    count: usize,
    first: i64,
    last: i64,
    cover: Cover,
    /// Where its photos are, in world pixels at zoom 0.
    bounds: Bounds,
}

#[derive(Default)]
struct Ranked {
    /// Name and count, most first.
    rows: Vec<(String, usize)>,
    /// Photos with no name recorded.
    unknown: usize,
}

impl Ranked {
    fn from_counts(counts: HashMap<String, usize>, unknown: usize) -> Ranked {
        let mut rows: Vec<(String, usize)> = counts.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ranked { rows, unknown }
    }

    fn known(&self) -> usize {
        self.rows.iter().map(|(_, count)| count).sum()
    }
}

/// The Statistics page's numbers: for all the photos in view, for the year
/// chosen, and the countries they were taken in.
pub(super) struct StatsPage {
    epoch: u64,
    all: Rc<Stats>,
    shown: Rc<Stats>,
    year: Option<i32>,
    travel: Rc<Visits>,
}

impl StatsPage {
    fn new(photos: &[Photo], epoch: u64, year: Option<i32>) -> StatsPage {
        let every: Vec<&Photo> = photos.iter().collect();
        let all = Rc::new(compute(&every));
        let year = year.filter(|year| all.totals.contains_key(year));
        let shown = match year {
            Some(year) => Rc::new(compute(&in_year(photos, year))),
            None => Rc::clone(&all),
        };
        StatsPage { epoch, all, shown, year, travel: Rc::new(Visits::compute(photos, epoch)) }
    }
}

fn local(at: i64) -> Option<chrono::DateTime<chrono::Local>> {
    chrono::DateTime::from_timestamp(at, 0).map(|time| time.with_timezone(&chrono::Local))
}

fn in_year(photos: &[Photo], year: i32) -> Vec<&Photo> {
    photos.iter().filter(|photo| local(photo.taken_at).is_some_and(|time| time.year() == year)).collect()
}

/// Works out the statistics of `photos`.
fn compute(photos: &[&Photo]) -> Stats {
    let mut stats = Stats {
        shown_at: Cell::new(None),
        photos: 0,
        videos: 0,
        bytes: 0,
        span: None,
        years: Vec::new(),
        totals: BTreeMap::new(),
        months: [0; 12],
        week_hours: [[0; 24]; 7],
        days: HashMap::new(),
        busiest_day: None,
        busiest_month: None,
        first: None,
        streak: None,
        geotagged: 0,
        places: Vec::new(),
        cameras: Ranked::default(),
        lenses: Ranked::default(),
        focal: [0; FOCAL_BANDS.len()],
        focal_top: None,
    };
    // Each day's and month's photos, and the first of them.
    let mut days: HashMap<i32, (usize, &Photo)> = HashMap::new();
    let mut months: HashMap<(i32, u32), (usize, &Photo)> = HashMap::new();
    let mut first: Option<&Photo> = None;
    let mut cameras: HashMap<String, usize> = HashMap::new();
    let mut lenses: HashMap<String, usize> = HashMap::new();
    let mut focals: HashMap<u32, usize> = HashMap::new();
    let (mut no_camera, mut no_lens) = (0, 0);
    let mut located: Vec<usize> = Vec::new();
    for (index, photo) in photos.iter().copied().enumerate() {
        let video = photo.is_video();
        if video {
            stats.videos += 1;
        } else {
            stats.photos += 1;
        }
        stats.bytes += photo.bytes;
        if let Some(time) = local(photo.taken_at) {
            let totals = stats.totals.entry(time.year()).or_default();
            if video {
                totals.videos += 1;
            } else {
                totals.photos += 1;
            }
            totals.bytes += photo.bytes;
            stats.months[time.month0() as usize] += 1;
            stats.week_hours[time.weekday().num_days_from_monday() as usize][time.hour() as usize] += 1;
            for (count, earliest) in [days.entry(time.num_days_from_ce()).or_insert((0, photo)), months.entry((time.year(), time.month())).or_insert((0, photo))] {
                *count += 1;
                if photo.taken_at < earliest.taken_at {
                    *earliest = photo;
                }
            }
            if first.is_none_or(|first| photo.taken_at < first.taken_at) {
                first = Some(photo);
            }
            let at = photo.taken_at;
            stats.span = Some(stats.span.map_or((at, at), |(start, end)| (start.min(at), end.max(at))));
        }
        if photo.place().is_some() {
            located.push(index);
        }
        match photo.camera.trim() {
            "" => no_camera += 1,
            camera => *cameras.entry(camera.to_string()).or_default() += 1,
        }
        match photo.lens.trim() {
            "" => no_lens += 1,
            lens => *lenses.entry(lens.to_string()).or_default() += 1,
        }
        if photo.focal_mm > 0.0 {
            let band = FOCAL_BANDS.iter().position(|(top, _)| photo.focal_mm <= *top + 0.5).unwrap_or(FOCAL_BANDS.len() - 1);
            stats.focal[band] += 1;
            *focals.entry(photo.focal_mm.round() as u32).or_default() += 1;
        }
    }
    for day in days.keys() {
        if let Some(date) = NaiveDate::from_num_days_from_ce_opt(*day) {
            stats.totals.entry(date.year()).or_default().days += 1;
        }
    }
    if let (Some(first), Some(last)) = (stats.totals.keys().next(), stats.totals.keys().next_back()) {
        stats.years = (*first..=*last).map(|year| (year, stats.totals.get(&year).map_or(0, |totals| totals.photos + totals.videos))).collect();
    }
    // Ties go to the earlier day or month.
    let moment = |(count, photo): &(usize, &Photo)| Moment { at: photo.taken_at, count: *count, cover: Cover::of(photo) };
    stats.busiest_day = days.values().max_by(|a, b| a.0.cmp(&b.0).then(b.1.taken_at.cmp(&a.1.taken_at))).map(moment);
    stats.busiest_month = months.values().max_by(|a, b| a.0.cmp(&b.0).then(b.1.taken_at.cmp(&a.1.taken_at))).map(moment);
    stats.first = first.map(|photo| Moment { at: photo.taken_at, count: 1, cover: Cover::of(photo) });
    stats.streak = longest_run(days.keys().copied().collect()).filter(|(_, length)| *length >= 2).map(|(start, length)| {
        let (_, opening) = days[&start];
        let (_, closing) = days[&(start + length as i32 - 1)];
        Streak { days: length, first: opening.taken_at, last: closing.taken_at, cover: Cover::of(opening) }
    });
    stats.days = days.into_iter().map(|(day, (count, _))| (day, count)).collect();
    stats.focal_top = focals.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
    stats.cameras = Ranked::from_counts(cameras, no_camera);
    stats.lenses = Ranked::from_counts(lenses, no_lens);

    stats.geotagged = located.len();
    let places: Vec<(f64, f64)> = located.iter().filter_map(|index| photos[*index].place()).collect();
    let points: Vec<[f64; 2]> = places
        .iter()
        .map(|(lat, lon)| {
            let (x, y) = geo::latlon_to_world(*lat, *lon, PLACE_ZOOM);
            [x, y]
        })
        .collect();
    stats.places = cluster_points(&points, 64.0)
        .into_iter()
        .map(|(members, _)| {
            let mut bounds = Bounds::EMPTY;
            let (mut first, mut last) = (i64::MAX, i64::MIN);
            for member in &members {
                let (x, y) = geo::latlon_to_world(places[*member].0, places[*member].1, 0);
                bounds.add([x, y]);
                let taken = photos[located[*member]].taken_at;
                first = first.min(taken);
                last = last.max(taken);
            }
            Place {
                label: folder_summary(members.iter().map(|member| photos[located[*member]].path.as_str())),
                count: members.len(),
                first,
                last,
                cover: Cover::of(photos[located[members[0]]]),
                bounds,
            }
        })
        .collect();
    stats
}

/// The longest run of consecutive day numbers in `days`, as its first day
/// and length. Of runs as long, the latest wins.
fn longest_run(mut days: Vec<i32>) -> Option<(i32, usize)> {
    days.sort_unstable();
    let mut best: Option<(i32, usize)> = None;
    let mut start = 0;
    for index in 0..days.len() {
        if index > 0 && days[index] != days[index - 1] + 1 {
            start = index;
        }
        let length = index - start + 1;
        if best.is_none_or(|(_, most)| length >= most) {
            best = Some((days[start], length));
        }
    }
    best
}

/// What the page shows: the numbers for all years, and for the one chosen.
struct Scope<'a> {
    all: &'a Stats,
    stats: &'a Stats,
    year: Option<i32>,
    travel: &'a Visits,
    width: f32,
    grow: f32,
}

impl PicassoApp {
    pub(super) fn stats_ui(&mut self, ui: &mut egui::Ui) {
        if self.stats.as_ref().is_none_or(|page| page.epoch != self.view_epoch) {
            let year = self.stats.as_ref().and_then(|page| page.year);
            self.stats = Some(StatsPage::new(&self.photos, self.view_epoch, year));
        }
        let Some(page) = self.stats.as_ref() else {
            return;
        };
        let (all, shown, year, travel) = (Rc::clone(&page.all), Rc::clone(&page.shown), page.year, Rc::clone(&page.travel));
        ui.painter().rect_filled(ui.available_rect_before_wrap(), 0, page_fill());
        let grow = shown.grow(ui.ctx());
        let mut go: Option<Go> = None;
        egui::ScrollArea::vertical().id_salt("stats").auto_shrink([false, false]).show(ui, |ui| {
            let room = ui.available_width();
            let width = (room - 40.0).min(MAX_W);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(((room - width) / 2.0).max(0.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.set_width(width);
                    ui.add_space(20.0);
                    let scope = Scope { all: &all, stats: &shown, year, travel: &travel, width, grow };
                    self.stats_page(ui, &scope, &mut go);
                    ui.add_space(32.0);
                });
            });
        });
        match go {
            Some(Go::Search(query)) => self.search_library(query),
            Some(Go::Map(bounds)) => self.show_area_on_map(bounds),
            Some(Go::TravelMap) => self.set_nav(Nav::TravelMap),
            Some(Go::Year(year)) => self.set_stats_year(year),
            None => {}
        }
    }

    fn stats_page(&mut self, ui: &mut egui::Ui, scope: &Scope, go: &mut Option<Go>) {
        let width = scope.width;
        header(ui, scope, go);
        ui.add_space(14.0);
        if let Some(year) = year_chips(ui, scope) {
            *go = Some(Go::Year(year));
        }
        ui.add_space(18.0);
        tiles(ui, scope);
        ui.add_space(CARD_GAP);
        self.highlights(ui, scope, go);
        if let Some(year) = scope.year.or(scope.all.years.last().map(|(year, _)| *year)) {
            bento(ui, "calendar", width, &[1.0], |ui, _| {
                if let Some(query) = calendar_card(ui, scope, year) {
                    *go = Some(Go::Search(query));
                }
            });
            ui.add_space(CARD_GAP);
        }
        bento(ui, "time", width, &[0.6, 0.4], |ui, index| {
            if index == 0 {
                timeline_card(ui, scope, go);
            } else {
                rhythm_card(ui, scope);
            }
        });
        ui.add_space(CARD_GAP);
        bento(ui, "where", width, &[0.6, 0.4], |ui, index| {
            if index == 0 {
                self.places_card(ui, scope, go);
            } else {
                card_title(ui, Mark::Globe, theme::GOLD, tr("Travel"), None);
                if self.travel_card(ui, scope.travel, scope.year) {
                    *go = Some(Go::TravelMap);
                }
            }
        });
        ui.add_space(CARD_GAP);
        bento(ui, "gear", width, &[0.5, 0.5], |ui, index| {
            let (ranked, text) = if index == 0 { (&scope.stats.cameras, &CAMERAS) } else { (&scope.stats.lenses, &LENSES) };
            ranked_card(ui, ranked, text, scope, go);
        });
        ui.add_space(CARD_GAP);
        bento(ui, "mix", width, &[0.4, 0.6], |ui, index| {
            if index == 0 {
                mix_card(ui, scope);
            } else {
                focal_card(ui, scope);
            }
        });
    }

    /// Shows the numbers for `year`, or for all years.
    fn set_stats_year(&mut self, year: Option<i32>) {
        let Some(page) = self.stats.as_mut() else {
            return;
        };
        if page.year == year {
            return;
        }
        page.year = year;
        page.shown = match year {
            Some(year) => Rc::new(compute(&in_year(&self.photos, year))),
            None => {
                page.all.shown_at.set(None);
                Rc::clone(&page.all)
            }
        };
    }

    /// The busiest day, the longest streak, the busiest month, and the first
    /// photo, each on its photo.
    fn highlights(&mut self, ui: &mut egui::Ui, scope: &Scope, go: &mut Option<Go>) {
        let stats = scope.stats;
        let day_query = |at: i64| local(at).map(|time| time.format("%Y-%m-%d").to_string());
        let mut cards = Vec::new();
        if let Some(day) = &stats.busiest_day {
            cards.push(Highlight {
                kicker: tr("Busiest day"),
                title: super::photo_count(day.count),
                sub: format_day(day.at),
                cover: &day.cover,
                tint: theme::accent(),
                query: day_query(day.at),
            });
        }
        if let Some(streak) = &stats.streak {
            cards.push(Highlight {
                kicker: tr("Longest streak"),
                title: trf("{} days in a row", &[&streak.days]),
                sub: format!("{} – {}", format_day(streak.first), format_day(streak.last)),
                cover: &streak.cover,
                tint: GREEN,
                query: None,
            });
        }
        if let Some((month, time)) = stats.busiest_month.as_ref().and_then(|month| Some((month, local(month.at)?))) {
            cards.push(Highlight {
                kicker: tr("Busiest month"),
                title: format_month(time.year(), time.month()),
                sub: super::photo_count(month.count),
                cover: &month.cover,
                tint: theme::GOLD,
                query: Some(time.format("%Y-%m").to_string()),
            });
        }
        if let Some(first) = &stats.first {
            cards.push(Highlight {
                kicker: tr("First photo"),
                title: format_day(first.at),
                sub: folder_summary(std::iter::once(first.cover.path.as_str())),
                cover: &first.cover,
                tint: theme::ALBUM,
                query: day_query(first.at),
            });
        }
        if cards.is_empty() {
            return;
        }
        let per_row = balanced(cards.len(), scope.width, 220.0);
        let card_w = (scope.width - CARD_GAP * (per_row - 1) as f32) / per_row as f32;
        for row in cards.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = CARD_GAP;
                for card in row {
                    if self.highlight(ui, card_w, card) {
                        *go = card.query.clone().map(Go::Search);
                    }
                }
            });
            ui.add_space(CARD_GAP);
        }
    }

    /// One highlight on its photo. Returns whether it was clicked.
    fn highlight(&mut self, ui: &mut egui::Ui, width: f32, card: &Highlight) -> bool {
        const RADIUS: u8 = 16;
        let sense = if card.query.is_some() { Sense::click() } else { Sense::hover() };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 168.0), sense);
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let painter = ui.painter().clone();
        painter.add(soft_shadow().as_shape(rect, RADIUS));
        self.photo_tile(ui, &painter, rect, card.cover, RADIUS, card.tint);
        let kicker = painter.layout_no_wrap(card.kicker.to_string(), FontId::proportional(11.5), WHITE);
        let pill = Rect::from_min_size(rect.min + Vec2::new(12.0, 12.0), kicker.size() + Vec2::new(18.0, 8.0));
        painter.rect_filled(pill, 11, Color32::from_black_alpha(120));
        painter.galley(pill.min + Vec2::new(9.0, 4.0), kicker, WHITE);
        let inner = rect.shrink(16.0);
        let sub = truncated(&painter, card.sub.clone(), 12.0, Color32::from_white_alpha(225), inner.width());
        let title = truncated(&painter, card.title.clone(), 20.0, WHITE, inner.width());
        let sub_y = inner.bottom() - sub.size().y;
        painter.galley(Pos2::new(inner.left(), sub_y - title.size().y - 2.0), title, WHITE);
        painter.galley(Pos2::new(inner.left(), sub_y), sub, WHITE);
        if card.query.is_none() {
            return false;
        }
        if response.hovered() {
            painter.rect_filled(rect, RADIUS, Color32::from_white_alpha(16));
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Show these photos")).clicked()
    }

    /// `cover` filling `rect`, darkened toward the bottom for the words on
    /// it; `tint` until it has loaded.
    fn photo_tile(&mut self, ui: &egui::Ui, painter: &egui::Painter, rect: Rect, cover: &Cover, radius: u8, tint: Color32) {
        self.ensure_thumb(cover.id, &cover.path);
        let held = ui.input(|input| input.pointer.any_down());
        self.sharpen_thumb(cover.id, &cover.path, rect.width().max(rect.height()) * ui.ctx().pixels_per_point(), held);
        match self.thumb_texture(cover.id) {
            Some((texture, w, h)) => theme::paint_image_rounded(painter, &texture, rect, fill_uv(w, h, cover.rotation, rect), cover.rotation, radius as f32),
            None => {
                painter.rect_filled(rect, radius, tint.gamma_multiply(0.75));
            }
        }
        let shade = Color32::from_black_alpha(160);
        let fade = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.3), Pos2::new(rect.right(), rect.bottom() - radius as f32));
        painter.add(Shape::mesh(gradient(fade, Color32::TRANSPARENT, shade)));
        let foot = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - radius as f32), rect.max);
        painter.rect_filled(foot, CornerRadius { nw: 0, ne: 0, sw: radius, se: radius }, shade);
    }

    /// How many photos have a location, the GPS tracks, and the places with
    /// the most photos as a mosaic.
    fn places_card(&mut self, ui: &mut egui::Ui, scope: &Scope, go: &mut Option<Go>) {
        let stats = scope.stats;
        let total = stats.photos + stats.videos;
        let count = (!stats.places.is_empty()).then(|| trf(if stats.places.len() == 1 { "{} place" } else { "{} places" }, &[&grouped(stats.places.len())]));
        card_title(ui, Mark::Pin, theme::PIN, tr("Places"), count.as_deref());
        ui.horizontal(|ui| {
            ui.label(RichText::new(trf("{}% of photos have a location", &[&percent(stats.geotagged, total)])).size(13.0).color(theme::text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(trf("{} of {} photos", &[&grouped(stats.geotagged), &grouped(total)])).size(12.0).color(theme::muted()));
            });
        });
        let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
        meter(ui.painter(), bar, stats.geotagged as f32 / total.max(1) as f32 * scope.grow, theme::PIN);
        if !self.tracks.is_empty() {
            let meters: f64 = self.tracks.iter().map(|track| track.meters).sum();
            let count = trf(if self.tracks.len() == 1 { "{} track" } else { "{} tracks" }, &[&self.tracks.len()]);
            ui.add_space(4.0);
            muted(ui, &format!("{count} · {}", format_distance(meters)));
        }
        if stats.places.is_empty() {
            ui.add_space(10.0);
            muted(ui, tr("Photos with a location show up here, grouped by place."));
            return;
        }
        ui.add_space(14.0);
        const GAP: f32 = 10.0;
        let width = ui.available_width();
        let columns = if width >= 520.0 { 3 } else { 2 };
        let shown = stats.places.len().min(columns * 2);
        let tile_w = (width - GAP * (columns - 1) as f32) / columns as f32;
        let tile = Vec2::new(tile_w, (tile_w * 0.72).round());
        for row in stats.places[..shown].chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for place in row {
                    if self.place_tile(ui, tile, place) {
                        *go = Some(Go::Map(place.bounds));
                    }
                }
            });
            ui.add_space(GAP - ui.spacing().item_spacing.y);
        }
        if stats.places.len() > shown {
            let rest: usize = stats.places[shown..].iter().map(|place| place.count).sum();
            more_line(ui, stats.places.len() - shown, rest);
        }
    }

    /// One place on its photo, with its folder, photos, and dates. Returns
    /// whether it was clicked.
    fn place_tile(&mut self, ui: &mut egui::Ui, size: Vec2, place: &Place) -> bool {
        const RADIUS: u8 = 12;
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let painter = ui.painter().clone();
        self.photo_tile(ui, &painter, rect, &place.cover, RADIUS, theme::PIN);
        let inner = rect.shrink(10.0);
        let line = format!("{} · {}", grouped(place.count), date_span([place.first, place.last].into_iter()));
        let sub = truncated(&painter, line, 11.0, Color32::from_white_alpha(225), inner.width());
        let title = truncated(&painter, place.label.clone(), 13.5, WHITE, inner.width());
        let sub_y = inner.bottom() - sub.size().y;
        painter.galley(Pos2::new(inner.left(), sub_y - title.size().y - 1.0), title, WHITE);
        painter.galley(Pos2::new(inner.left(), sub_y), sub, WHITE);
        if response.hovered() {
            painter.rect_filled(rect, RADIUS, Color32::from_white_alpha(18));
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Show on map")).clicked()
    }

    /// The Library with the photos that match `query`.
    fn search_library(&mut self, query: String) {
        self.nav = Nav::Library;
        self.mode = Mode::Browse;
        self.selection.clear();
        self.search = query;
        self.search_due = None;
        self.reload_view();
    }

    /// The Places map framed on `bounds`, in world pixels at zoom 0.
    pub(super) fn show_area_on_map(&mut self, bounds: Bounds) {
        self.open_places_to_show();
        self.frame_after_load = false;
        self.frame_bounds(bounds);
        self.map_zoom = self.map_zoom.min(14);
        self.map_card = None;
    }
}

/// Where a click on the page leads.
enum Go {
    Search(String),
    Map(Bounds),
    TravelMap,
    Year(Option<i32>),
}

/// A highlight card: what it is, its numbers, its photo, and the search it
/// leads to.
struct Highlight<'a> {
    kicker: &'a str,
    title: String,
    sub: String,
    cover: &'a Cover,
    tint: Color32,
    query: Option<String>,
}

/// How many of `count` cards fit in a row of `width`, each at least
/// `least` wide, with the rows as even as can be.
fn balanced(count: usize, width: f32, least: f32) -> usize {
    let most = ((width + CARD_GAP) / (least + CARD_GAP)).floor().clamp(1.0, count.max(1) as f32) as usize;
    let rows = count.div_ceil(most);
    count.div_ceil(rows.max(1)).max(1)
}

/// The summary line, the span and size, and photos per year as a line that
/// picks a year.
fn header(ui: &mut egui::Ui, scope: &Scope, go: &mut Option<Go>) {
    let stats = scope.stats;
    let total = super::photo_count(stats.photos + stats.videos);
    let title = match (scope.year, scope.all.years.first(), scope.all.years.last()) {
        (Some(year), _, _) => trf("{} in {}", &[&total, &year]),
        (None, Some((first, _)), Some((last, _))) if first != last => trf("{} over {} years", &[&total, &(last - first + 1)]),
        (None, Some((first, _)), _) => trf("{} in {}", &[&total, first]),
        _ => total,
    };
    let mut details = Vec::new();
    if let Some((first, last)) = stats.span {
        details.push(date_span([first, last].into_iter()));
    }
    details.push(format_bytes(stats.bytes));
    let width = scope.width;
    let spark = scope.all.years.len() >= 2 && width >= 560.0;
    let spark_w = if spark { (width * 0.34).min(340.0) } else { 0.0 };
    let room = width - spark_w - if spark { 24.0 } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 64.0), Sense::hover());
    let painter = ui.painter();
    let title = truncated(painter, title, 26.0, theme::text(), room);
    let title_h = title.size().y;
    painter.galley(rect.min, title, theme::text());
    let details = truncated(painter, details.join(" · "), 13.0, theme::muted(), room);
    painter.galley(Pos2::new(rect.left(), rect.top() + title_h + 6.0), details, theme::muted());
    if spark {
        let area = Rect::from_min_max(Pos2::new(rect.right() - spark_w, rect.top()), rect.max);
        if let Some(year) = sparkline(ui, area, &scope.all.years, scope.year, scope.grow) {
            *go = Some(Go::Year(if scope.year == Some(year) { None } else { Some(year) }));
        }
    }
}

/// Photos per year as a line, the chosen year marked. Returns the year
/// clicked.
fn sparkline(ui: &mut egui::Ui, rect: Rect, years: &[(i32, usize)], chosen: Option<i32>, grow: f32) -> Option<i32> {
    let response = ui.interact(rect, egui::Id::unique("stats-sparkline"), Sense::click());
    let plot = rect.shrink2(Vec2::new(6.0, 6.0));
    let most = years.iter().map(|(_, count)| *count).max().unwrap_or(0).max(1) as f32;
    let step = plot.width() / (years.len().max(2) - 1) as f32;
    let points: Vec<Pos2> = years
        .iter()
        .enumerate()
        .map(|(index, (_, count))| Pos2::new(plot.left() + index as f32 * step, plot.bottom() - plot.height() * *count as f32 / most * grow))
        .collect();
    let painter = ui.painter();
    let mut area = Mesh::default();
    for (index, point) in points.iter().enumerate() {
        area.colored_vertex(*point, theme::accent().gamma_multiply(0.22));
        area.colored_vertex(Pos2::new(point.x, plot.bottom()), Color32::TRANSPARENT);
        if index > 0 {
            let at = 2 * index as u32 - 2;
            area.add_triangle(at, at + 1, at + 3);
            area.add_triangle(at, at + 3, at + 2);
        }
    }
    painter.add(Shape::mesh(area));
    painter.add(Shape::line(points.clone(), Stroke::new(2.0, theme::accent())));
    let hovered = response.hover_pos().map(|pos| (((pos.x - plot.left()) / step).round().max(0.0) as usize).min(years.len() - 1));
    if let Some(index) = hovered {
        painter.vline(points[index].x, plot.y_range(), Stroke::new(1.0, theme::hairline()));
    }
    for (index, (year, _)) in years.iter().enumerate() {
        if chosen == Some(*year) || hovered == Some(index) {
            painter.circle(points[index], 4.5, card_fill(), Stroke::new(2.0, theme::accent()));
        }
    }
    let index = hovered?;
    let (year, count) = years[index];
    let foot = if chosen == Some(year) { tr("All years") } else { tr("Show this year") };
    hover_card(ui, points[index], &year.to_string(), &super::photo_count(count), Some(foot));
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked().then_some(year)
}

/// All years and each year with photos, newest first. Returns the choice.
fn year_chips(ui: &mut egui::Ui, scope: &Scope) -> Option<Option<i32>> {
    let years: Vec<(i32, usize)> = scope.all.years.iter().rev().filter(|(_, count)| *count > 0).copied().collect();
    if years.len() < 2 {
        return None;
    }
    let mut chosen = None;
    egui::ScrollArea::horizontal()
        .id_salt("stats-years")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let total = scope.all.photos + scope.all.videos;
                if chip(ui, tr("All years"), scope.year.is_none()).on_hover_text(super::photo_count(total)).clicked() {
                    chosen = Some(None);
                }
                for (year, count) in &years {
                    if chip(ui, &year.to_string(), scope.year == Some(*year)).on_hover_text(super::photo_count(*count)).clicked() {
                        chosen = Some(Some(*year));
                    }
                }
            });
        });
    chosen
}

fn chip(ui: &mut egui::Ui, label: &str, active: bool) -> Response {
    let font = FontId::proportional(13.0);
    let size = ui.painter().layout_no_wrap(label.to_string(), font.clone(), theme::text()).size();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(size.x + 28.0, 30.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    let (fill, color, stroke) = if active {
        (ACCENT, WHITE, Stroke::NONE)
    } else if response.hovered() {
        (theme::hover(), theme::text(), Stroke::new(1.0, theme::hairline()))
    } else {
        (card_fill(), theme::text(), Stroke::new(1.0, theme::hairline()))
    };
    painter.rect(rect, 15, fill, stroke, StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    response
}

/// A big number with an icon, a label, and a note under it.
struct Tile {
    mark: Mark,
    tint: Color32,
    value: String,
    label: String,
    note: Note,
}

enum Note {
    None,
    Text(String),
    /// The change on a year, as a fraction.
    Change(f64, i32),
}

/// The big numbers. With a year chosen, each tells how it changed on the
/// year before.
fn tiles(ui: &mut egui::Ui, scope: &Scope) {
    let stats = scope.stats;
    let total = stats.photos + stats.videos;
    let previous = scope.year.and_then(|year| Some((year - 1, *scope.all.totals.get(&(year - 1))?)));
    let change = |now: f64, then: fn(&Totals) -> f64| match previous {
        Some((year, totals)) if then(&totals) > 0.0 => Note::Change(now / then(&totals) - 1.0, year),
        _ => Note::None,
    };
    let first_year = scope.all.years.first().map(|(year, _)| *year);
    let mut list = vec![Tile {
        mark: Mark::Photo,
        tint: theme::accent(),
        value: grouped(stats.photos),
        label: tr("Photos").into(),
        note: match (scope.year, first_year) {
            (Some(_), _) => change(stats.photos as f64, |totals| totals.photos as f64),
            (None, Some(year)) => Note::Text(trf("since {}", &[&year])),
            (None, None) => Note::None,
        },
    }];
    if stats.videos > 0 {
        list.push(Tile { mark: Mark::Video, tint: ROSE, value: grouped(stats.videos), label: tr("Videos").into(), note: change(stats.videos as f64, |totals| totals.videos as f64) });
    }
    list.extend([
        Tile { mark: Mark::Calendar, tint: GREEN, value: grouped(stats.days.len()), label: tr("Days with photos").into(), note: change(stats.days.len() as f64, |totals| totals.days as f64) },
        Tile { mark: Mark::Storage, tint: theme::ALBUM, value: format_bytes(stats.bytes), label: tr("Total size").into(), note: change(stats.bytes as f64, |totals| totals.bytes as f64) },
        Tile {
            mark: Mark::Pin,
            tint: theme::PIN,
            value: format!("{}%", percent(stats.geotagged, total)),
            label: tr("With a location").into(),
            note: Note::Text(trf("{} of {} photos", &[&grouped(stats.geotagged), &grouped(total)])),
        },
        Tile {
            mark: Mark::Camera,
            tint: theme::GOLD,
            value: grouped(stats.cameras.rows.len()),
            label: tr("Cameras").into(),
            note: stats.cameras.rows.first().map_or(Note::None, |(name, _)| Note::Text(name.clone())),
        },
        Tile {
            mark: Mark::Lens,
            tint: SKY,
            value: grouped(stats.lenses.rows.len()),
            label: tr("Lenses").into(),
            note: stats.lenses.rows.first().map_or(Note::None, |(name, _)| Note::Text(name.clone())),
        },
    ]);
    let per_row = balanced(list.len(), scope.width, 168.0);
    for row in list.chunks(per_row) {
        let tile_w = (scope.width - CARD_GAP * (row.len() - 1) as f32) / row.len() as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP;
            for each in row {
                tile(ui, tile_w, each);
            }
        });
        ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
    }
}

fn tile(ui: &mut egui::Ui, width: f32, tile: &Tile) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 118.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    paint_card(painter, rect, 16);
    let inner = rect.shrink(16.0);
    badge(painter, Rect::from_min_size(inner.min, Vec2::splat(30.0)), tile.mark, tile.tint);
    let label = truncated(painter, tile.label.clone(), 12.5, theme::muted(), inner.width() - 40.0);
    painter.galley(Pos2::new(inner.left() + 40.0, inner.top() + 15.0 - label.size().y / 2.0), label, theme::muted());
    let value = truncated(painter, tile.value.clone(), 24.0, theme::text(), inner.width());
    painter.galley(Pos2::new(inner.left(), inner.top() + 36.0), value, theme::text());
    match &tile.note {
        Note::None => {}
        Note::Text(text) => {
            let note = truncated(painter, text.clone(), 11.5, theme::muted(), inner.width());
            painter.galley(Pos2::new(inner.left(), inner.bottom() - note.size().y), note, theme::muted());
        }
        Note::Change(change, year) => {
            let up = *change >= 0.0;
            let color = if up { GREEN } else { RED };
            let text = format!("{}%  {}", (change.abs() * 100.0).round(), trf("vs {}", &[year]));
            let note = truncated(painter, text, 11.5, color, inner.width() - 14.0);
            let top = inner.bottom() - note.size().y;
            let mid = Pos2::new(inner.left() + 4.5, top + note.size().y / 2.0);
            let (tip, base) = if up { (-4.0, 3.0) } else { (4.0, -3.0) };
            painter.add(Shape::convex_polygon(vec![mid + Vec2::new(-4.5, base), mid + Vec2::new(4.5, base), mid + Vec2::new(0.0, tip)], color, Stroke::NONE));
            painter.galley(Pos2::new(inner.left() + 13.0, top), note, color);
        }
    }
}

/// The days of `year` with photos, as a calendar. Returns the search for
/// the day clicked.
fn calendar_card(ui: &mut egui::Ui, scope: &Scope, year: i32) -> Option<String> {
    let days = &scope.stats.days;
    let active = days.keys().filter(|day| NaiveDate::from_num_days_from_ce_opt(**day).is_some_and(|date| date.year() == year)).count();
    card_title(ui, Mark::Calendar, GREEN, &trf("Activity in {}", &[&year]), Some(&trf("{} days with photos", &[&grouped(active)])));
    calendar(ui, days, year, scope.grow)
}

/// The days of `year` in weeks, a cell a day, deeper with more photos.
/// Returns the search for the day clicked.
fn calendar(ui: &mut egui::Ui, days: &HashMap<i32, usize>, year: i32, grow: f32) -> Option<String> {
    const LABEL_W: f32 = 34.0;
    const HEAD: f32 = 18.0;
    const FOOT: f32 = 30.0;
    let first = NaiveDate::from_ymd_opt(year, 1, 1)?;
    let last = NaiveDate::from_ymd_opt(year, 12, 31)?;
    let lead = i64::from(first.weekday().num_days_from_monday());
    let length = (last - first).num_days() + 1;
    let weeks = (lead + length + 6) / 7;
    let width = ui.available_width();
    let cell = ((width - LABEL_W) / weeks as f32).clamp(6.0, 20.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, HEAD + 7.0 * cell + FOOT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let painter = ui.painter();
    let origin = Pos2::new(rect.left() + LABEL_W, rect.top() + HEAD);
    let gap = (cell * 0.18).clamp(1.5, 3.0);
    let corner = (cell * 0.22) as u8;
    let count_of = |offset: i64| days.get(&(first.num_days_from_ce() + offset as i32)).copied().unwrap_or(0);
    let slot = |offset: i64| (((lead + offset) / 7) as f32, ((lead + offset) % 7) as f32);
    let most = (0..length).map(count_of).max().unwrap_or(0);
    let hovered = response.hover_pos().and_then(|pos| {
        let column = ((pos.x - origin.x) / cell).floor();
        let row = ((pos.y - origin.y) / cell).floor();
        if column < 0.0 || !(0.0..7.0).contains(&row) {
            return None;
        }
        let offset = column as i64 * 7 + row as i64 - lead;
        (0..length).contains(&offset).then_some(offset)
    });
    let font = FontId::proportional(10.5);
    for row in [0, 2, 4] {
        painter.text(Pos2::new(rect.left(), origin.y + (row as f32 + 0.5) * cell), Align2::LEFT_CENTER, short_weekday(row), font.clone(), theme::muted());
    }
    for month in 1..=12 {
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, 1) {
            let (column, _) = slot((date - first).num_days());
            painter.text(Pos2::new(origin.x + column * cell, rect.top()), Align2::LEFT_TOP, short_month(month), font.clone(), theme::muted());
        }
    }
    for offset in 0..length {
        let (column, row) = slot(offset);
        let square = Rect::from_min_size(origin + Vec2::new(column * cell, row * cell), Vec2::splat(cell)).shrink(gap / 2.0);
        painter.rect_filled(square, corner, heat(count_of(offset), most).gamma_multiply(wave(grow, column / weeks as f32)));
        if hovered == Some(offset) {
            painter.rect_stroke(square, corner, Stroke::new(1.5, theme::text()), StrokeKind::Outside);
        }
    }
    let foot = origin.y + 7.0 * cell + 12.0;
    let swatch = 11.0;
    let more = painter.layout_no_wrap(tr("More photos").to_string(), font.clone(), theme::muted());
    let less = painter.layout_no_wrap(tr("Fewer photos").to_string(), font, theme::muted());
    let mut x = rect.right() - more.size().x;
    painter.galley(Pos2::new(x, foot - 1.0), more, theme::muted());
    x -= 6.0;
    for level in (0..5).rev() {
        x -= swatch;
        let color = if level == 0 { theme::well() } else { theme::accent().gamma_multiply(heat_alpha((level - 1) as f32 / 3.0)) };
        painter.rect_filled(Rect::from_min_size(Pos2::new(x, foot), Vec2::splat(swatch)), 2, color);
        x -= 3.0;
    }
    x -= 3.0 + less.size().x;
    painter.galley(Pos2::new(x, foot - 1.0), less, theme::muted());

    let offset = hovered?;
    let date = first + chrono::Duration::days(offset);
    let count = count_of(offset);
    let (column, row) = slot(offset);
    let at = chrono::Local.from_local_datetime(&date.and_hms_opt(12, 0, 0)?).earliest()?.timestamp();
    let foot = (count > 0).then_some(tr("Show these photos"));
    hover_card(ui, origin + Vec2::new((column + 0.5) * cell, row * cell), &format_day(at), &photos_or_none(count), foot);
    if count > 0 && response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        return Some(date.format("%Y-%m-%d").to_string());
    }
    None
}

/// Photos per year, which picks a year, and by month; or the chosen year's
/// photos per month, which lead to them.
fn timeline_card(ui: &mut egui::Ui, scope: &Scope, go: &mut Option<Go>) {
    let stats = scope.stats;
    match scope.year {
        None => {
            card_title(ui, Mark::Chart, theme::accent(), tr("Photos per year"), None);
            if stats.years.is_empty() {
                muted(ui, tr("Unknown date"));
                return;
            }
            let years: Vec<(String, usize)> = stats.years.iter().map(|(year, count)| (year.to_string(), *count)).collect();
            if let Some(index) = columns(ui, &years, 150.0, scope.grow, Some(tr("Show this year"))) {
                *go = Some(Go::Year(Some(stats.years[index].0)));
            }
            ui.add_space(16.0);
            subheading(ui, tr("By month"));
            columns(ui, &month_bars(&stats.months), 90.0, scope.grow, None);
        }
        Some(year) => {
            card_title(ui, Mark::Chart, theme::accent(), tr("Photos per month"), Some(&year.to_string()));
            if let Some(index) = columns(ui, &month_bars(&stats.months), 220.0, scope.grow, Some(tr("Show these photos"))) {
                *go = Some(Go::Search(format!("{year}-{:02}", index + 1)));
            }
        }
    }
}

fn month_bars(months: &[usize; 12]) -> Vec<(String, usize)> {
    (1..=12).map(|month| (short_month(month), months[month as usize - 1])).collect()
}

/// When in the week photos are taken, by weekday and hour.
fn rhythm_card(ui: &mut egui::Ui, scope: &Scope) {
    card_title(ui, Mark::Clock, ROSE, tr("When you shoot"), None);
    let grid = &scope.stats.week_hours;
    let busiest = (0..7).flat_map(|day| (0..24).map(move |hour| (day, hour, grid[day][hour]))).max_by_key(|(_, _, count)| *count);
    let Some((day, hour, count)) = busiest.filter(|(_, _, count)| *count > 0) else {
        muted(ui, tr("Unknown date"));
        return;
    };
    punch_card(ui, grid, count, scope.grow);
    ui.add_space(10.0);
    muted(ui, &trf("Busiest hour: {}", &[&format!("{} {hour:02}:00", short_weekday(day))]));
}

/// Photos by weekday and hour as a grid of cells, deeper with more.
fn punch_card(ui: &mut egui::Ui, grid: &[[usize; 24]; 7], most: usize, grow: f32) {
    const LABEL_W: f32 = 36.0;
    const FOOT: f32 = 18.0;
    let width = ui.available_width();
    let cell_w = (width - LABEL_W) / 24.0;
    let cell_h = cell_w.clamp(12.0, 24.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 7.0 * cell_h + FOOT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let origin = Pos2::new(rect.left() + LABEL_W, rect.top());
    let gap = (cell_w.min(cell_h) * 0.16).clamp(1.5, 3.0);
    let hovered = response.hover_pos().and_then(|pos| {
        let column = ((pos.x - origin.x) / cell_w).floor();
        let row = ((pos.y - origin.y) / cell_h).floor();
        ((0.0..24.0).contains(&column) && (0.0..7.0).contains(&row)).then_some((row as usize, column as usize))
    });
    let font = FontId::proportional(10.5);
    for (day, hours) in grid.iter().enumerate() {
        painter.text(Pos2::new(rect.left(), origin.y + (day as f32 + 0.5) * cell_h), Align2::LEFT_CENTER, short_weekday(day), font.clone(), theme::muted());
        for (hour, count) in hours.iter().enumerate() {
            let cell = Rect::from_min_size(origin + Vec2::new(hour as f32 * cell_w, day as f32 * cell_h), Vec2::new(cell_w, cell_h)).shrink(gap / 2.0);
            painter.rect_filled(cell, 3, heat(*count, most).gamma_multiply(wave(grow, hour as f32 / 24.0)));
            if hovered == Some((day, hour)) {
                painter.rect_stroke(cell, 3, Stroke::new(1.5, theme::text()), StrokeKind::Outside);
            }
        }
    }
    for hour in [0, 6, 12, 18] {
        painter.text(Pos2::new(origin.x + hour as f32 * cell_w + 1.0, origin.y + 7.0 * cell_h + 4.0), Align2::LEFT_TOP, format!("{hour:02}:00"), font.clone(), theme::muted());
    }
    if let Some((day, hour)) = hovered {
        let anchor = origin + Vec2::new((hour as f32 + 0.5) * cell_w, day as f32 * cell_h);
        hover_card(ui, anchor, &format!("{} {hour:02}:00", short_weekday(day)), &photos_or_none(grid[day][hour]), None);
    }
}

/// Text for one of the ranked lists: its key for the state kept between
/// frames, its title and icon, the line counting photos without a name, and
/// the line shown when there are no names at all.
struct RankedText {
    key: &'static str,
    title: &'static str,
    mark: Mark,
    tint: Color32,
    unknown: &'static str,
    empty: &'static str,
}

const CAMERAS: RankedText = RankedText {
    key: "cameras",
    title: "Cameras",
    mark: Mark::Camera,
    tint: theme::GOLD,
    unknown: "{} without camera info",
    empty: "Photos taken with a camera that records it show up here.",
};

const LENSES: RankedText = RankedText {
    key: "lenses",
    title: "Lenses",
    mark: Mark::Lens,
    tint: SKY,
    unknown: "{} without lens info",
    empty: "Photos with a recorded lens show up here.",
};

/// The names with the most photos as a donut and a legend, the rest as one
/// slice that opens the full list, and how many photos have no name.
fn ranked_card(ui: &mut egui::Ui, ranked: &Ranked, text: &RankedText, scope: &Scope, go: &mut Option<Go>) {
    let count = (!ranked.rows.is_empty()).then(|| grouped(ranked.rows.len()));
    card_title(ui, text.mark, text.tint, tr(text.title), count.as_deref());
    let unknown = |ui: &mut egui::Ui| {
        if ranked.unknown > 0 {
            ui.add_space(6.0);
            ui.label(RichText::new(trf(text.unknown, &[&grouped(ranked.unknown)])).size(11.5).color(theme::muted()));
        }
    };
    if ranked.rows.is_empty() {
        muted(ui, tr(text.empty));
        unknown(ui);
        return;
    }
    let search = |name: &str| match scope.year {
        Some(year) => format!("{year} {name}"),
        None => name.to_string(),
    };
    let known = ranked.known();
    let id = egui::Id::unique(("stats-ranked-all", text.key));
    let all = ui.ctx().data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    let rest: usize = ranked.rows.iter().skip(SLICES).map(|(_, count)| count).sum();
    let mut parts: Vec<(f32, Color32)> = ranked.rows.iter().take(SLICES).zip(slice_colors()).map(|((_, count), color)| (*count as f32 / known.max(1) as f32, color)).collect();
    if rest > 0 {
        parts.push((rest as f32 / known.max(1) as f32, other_color()));
    }
    let mut toggle = false;
    let size = if ui.available_width() >= 380.0 { 136.0 } else { 112.0 };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        let (donut_response, slice) = donut(ui, size, &parts, scope.grow, &grouped(known), tr("Photos"));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (index, ((name, count), color)) in ranked.rows.iter().take(SLICES).zip(slice_colors()).enumerate() {
                if legend_row(ui, color, name, *count, known, slice == Some(index)).clicked() {
                    *go = Some(Go::Search(search(name)));
                }
            }
            if rest > 0 {
                let label = format!("{} ({})", tr("Other"), grouped(ranked.rows.len() - SLICES));
                if legend_row(ui, other_color(), &label, rest, known, slice == Some(SLICES)).clicked() {
                    toggle = true;
                }
            }
        });
        if donut_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            match slice {
                Some(index) if index < SLICES => *go = Some(Go::Search(search(&ranked.rows[index].0))),
                Some(_) => toggle = true,
                None => {}
            }
        }
    });
    if all && rest > 0 {
        ui.add_space(10.0);
        let most = ranked.rows[SLICES].1;
        egui::ScrollArea::vertical()
            .id_salt(("stats-ranked-rest", text.key))
            .max_height(REST_ROWS * (BAR_ROW_H + 2.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (name, count) in &ranked.rows[SLICES..] {
                    if bar_row(ui, name, *count, most, known) {
                        *go = Some(Go::Search(search(name)));
                    }
                }
            });
    }
    if rest > 0 {
        ui.add_space(4.0);
        let toggled = if all {
            more_link(ui, tr("Show fewer").to_string(), tr("Show only the top names"))
        } else {
            let line = trf("and {} more · {}", &[&grouped(ranked.rows.len() - SLICES), &super::photo_count(rest)]);
            more_link(ui, line, tr("Show the full list"))
        };
        toggle |= toggled;
    }
    if toggle {
        ui.ctx().data_mut(|data| data.insert_temp(id, !all));
    }
    unknown(ui);
}

/// Photos against videos, and photos with a location against those
/// without, as two donuts.
fn mix_card(ui: &mut egui::Ui, scope: &Scope) {
    let stats = scope.stats;
    let total = stats.photos + stats.videos;
    let share = |part: usize| part as f32 / total.max(1) as f32;
    card_title(ui, Mark::Pie, theme::ALBUM, tr("Library"), None);
    let size = ((ui.available_width() - 28.0) / 2.0).clamp(90.0, 136.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 28.0;
        mix_chart(ui, size, scope.grow, &grouped(total), [(theme::accent(), tr("Photos"), stats.photos), (ROSE, tr("Videos"), stats.videos)].map(|(color, name, count)| (share(count), color, name, count)));
        let without = total - stats.geotagged;
        mix_chart(
            ui,
            size,
            scope.grow,
            &format!("{}%", percent(stats.geotagged, total)),
            [(theme::PIN, tr("With a location"), stats.geotagged), (other_color(), tr("Without a location"), without)].map(|(color, name, count)| (share(count), color, name, count)),
        );
    });
}

/// A donut of two `parts`, each a share, color, name, and count, with its
/// legend under it.
fn mix_chart(ui: &mut egui::Ui, size: f32, grow: f32, value: &str, parts: [(f32, Color32, &str, usize); 2]) {
    ui.vertical(|ui| {
        ui.set_width(size);
        let slices: Vec<(f32, Color32)> = parts.iter().map(|(share, color, _, _)| (*share, *color)).collect();
        let (_, slice) = donut(ui, size, &slices, grow, value, "");
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        for (index, (_, color, name, count)) in parts.iter().enumerate() {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, *color);
                let tint = if slice == Some(index) { theme::text() } else { theme::muted() };
                ui.label(RichText::new(format!("{name} · {}", grouped(*count))).size(11.5).color(tint));
            });
        }
    });
}

fn focal_card(ui: &mut egui::Ui, scope: &Scope) {
    let stats = scope.stats;
    card_title(ui, Mark::Focal, SKY, tr("Focal length"), None);
    let known: usize = stats.focal.iter().sum();
    if known == 0 {
        muted(ui, tr("Photos with a recorded focal length show up here."));
        return;
    }
    let bands: Vec<(String, usize)> = FOCAL_BANDS.iter().zip(stats.focal).map(|((_, label), count)| (label.to_string(), count)).collect();
    columns(ui, &bands, 150.0, scope.grow, None);
    ui.add_space(6.0);
    let mut note = tr("Millimeters, 35 mm equivalent").to_string();
    if let Some((mm, count)) = stats.focal_top {
        note = format!("{note} · {}", trf("Most used: {} mm ({})", &[&mm, &super::photo_count(count)]));
    }
    ui.label(RichText::new(note).size(11.5).color(theme::muted()));
}

/// Cards side by side, each `shares` of `width`, as tall as the tallest
/// one; one under the other on a narrow page.
fn bento(ui: &mut egui::Ui, key: &str, width: f32, shares: &[f32], mut add: impl FnMut(&mut egui::Ui, usize)) {
    if width < NARROW || shares.len() == 1 {
        for index in 0..shares.len() {
            if index > 0 {
                ui.add_space(CARD_GAP);
            }
            shell(ui, width, 0.0, |ui| add(ui, index));
        }
        return;
    }
    // Content heights are known only once drawn, so a row is as tall as its
    // tallest card was the frame before.
    let id = egui::Id::unique(("stats-bento", key));
    let tallest = ui.ctx().data(|data| data.get_temp::<f32>(id)).unwrap_or(0.0);
    let room = width - CARD_GAP * (shares.len() - 1) as f32;
    let mut natural = 0.0f32;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = CARD_GAP;
        for (index, share) in shares.iter().enumerate() {
            natural = natural.max(shell(ui, room * share, tallest, |ui| add(ui, index)));
        }
    });
    if (natural - tallest).abs() > 0.5 {
        ui.ctx().data_mut(|data| data.insert_temp(id, natural));
        ui.ctx().request_repaint();
    }
}

/// A card `width` wide and at least `least` tall. Returns how tall its
/// content needs it to be.
fn shell(ui: &mut egui::Ui, width: f32, least: f32, add: impl FnOnce(&mut egui::Ui)) -> f32 {
    // The margin on both sides and the outline.
    let edge = 2.0 * PAD + 2.0;
    let mut natural = 0.0;
    egui::Frame::new()
        .fill(card_fill())
        .stroke(Stroke::new(1.0, theme::hairline()))
        .corner_radius(16)
        .shadow(soft_shadow())
        .inner_margin(Margin::same(PAD as i8))
        .show(ui, |ui| {
            let content = ui.vertical(|ui| {
                ui.set_width(width - edge);
                add(ui);
            });
            natural = content.response.rect.height() + edge;
            ui.set_min_height(least - edge);
        });
    natural
}

/// A card's icon, title, and a note on the right.
fn card_title(ui: &mut egui::Ui, mark: Mark, tint: Color32, title: &str, aside: Option<&str>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
    let painter = ui.painter();
    badge(painter, Rect::from_min_size(rect.min, Vec2::splat(30.0)), mark, tint);
    let aside = aside.map(|text| painter.layout_no_wrap(text.to_string(), FontId::proportional(12.0), theme::muted()));
    let aside_w = aside.as_ref().map_or(0.0, |aside| aside.size().x + 12.0);
    let title = truncated(painter, title.to_string(), 15.5, theme::text(), width - 40.0 - aside_w);
    painter.galley(Pos2::new(rect.left() + 40.0, rect.center().y - title.size().y / 2.0), title, theme::text());
    if let Some(aside) = aside {
        painter.galley(Pos2::new(rect.right() - aside.size().x, rect.center().y - aside.size().y / 2.0), aside, theme::muted());
    }
    ui.add_space(12.0);
}

/// Small drawings for the cards and tiles.
#[derive(Clone, Copy)]
enum Mark {
    Photo,
    Video,
    Calendar,
    Storage,
    Pin,
    Camera,
    Lens,
    Clock,
    Chart,
    Globe,
    Focal,
    Pie,
}

/// `mark` on a tinted square.
fn badge(painter: &egui::Painter, rect: Rect, mark: Mark, tint: Color32) {
    painter.rect_filled(rect, 9, tint.gamma_multiply(if theme::is_dark() { 0.22 } else { 0.12 }));
    paint_mark(painter, rect.center(), mark, tint);
}

fn paint_mark(painter: &egui::Painter, center: Pos2, mark: Mark, color: Color32) {
    let line = Stroke::new(1.6, color);
    match mark {
        Mark::Photo => {
            let frame = Rect::from_center_size(center, Vec2::new(15.0, 12.0));
            painter.rect_stroke(frame, 2, line, StrokeKind::Middle);
            let corner = frame.left_bottom();
            painter.add(Shape::convex_polygon(vec![corner + Vec2::new(2.0, -2.0), corner + Vec2::new(10.0, -2.0), corner + Vec2::new(6.0, -7.0)], color, Stroke::NONE));
            painter.circle_filled(frame.right_top() + Vec2::new(-4.0, 3.5), 1.5, color);
        }
        Mark::Video => {
            painter.rect_stroke(Rect::from_center_size(center, Vec2::new(15.0, 12.0)), 2, line, StrokeKind::Middle);
            painter.add(Shape::convex_polygon(vec![center + Vec2::new(-2.0, -3.5), center + Vec2::new(3.5, 0.0), center + Vec2::new(-2.0, 3.5)], color, Stroke::NONE));
        }
        Mark::Calendar => {
            let frame = Rect::from_center_size(center + Vec2::new(0.0, 1.0), Vec2::new(14.0, 13.0));
            painter.rect_stroke(frame, 2, line, StrokeKind::Middle);
            painter.hline(frame.x_range(), frame.top() + 4.0, line);
            for dx in [-3.5, 3.5] {
                painter.vline(center.x + dx, (frame.top() - 2.5)..=(frame.top() + 1.5), line);
            }
        }
        Mark::Storage => {
            for dy in [-4.5, 0.0, 4.5] {
                painter.rect_filled(Rect::from_center_size(center + Vec2::new(0.0, dy), Vec2::new(14.0, 3.2)), 1, color);
            }
        }
        Mark::Pin => theme::paint_pin(painter, center + Vec2::new(0.0, -1.0), 5.5, color, card_fill()),
        Mark::Camera => {
            let body = Rect::from_center_size(center + Vec2::new(0.0, 1.0), Vec2::new(16.0, 11.0));
            painter.rect_stroke(body, 2, line, StrokeKind::Middle);
            painter.rect_filled(Rect::from_center_size(Pos2::new(center.x - 3.0, body.top() - 1.0), Vec2::new(5.0, 2.5)), 1, color);
            painter.circle_stroke(body.center(), 3.0, line);
        }
        Mark::Lens => {
            painter.circle_stroke(center, 6.5, line);
            painter.circle_stroke(center, 3.2, line);
            painter.circle_filled(center, 1.2, color);
        }
        Mark::Clock => {
            painter.circle_stroke(center, 7.0, line);
            painter.line_segment([center, center + Vec2::new(0.0, -4.5)], line);
            painter.line_segment([center, center + Vec2::new(3.5, 0.0)], line);
        }
        Mark::Chart => {
            for (n, h) in [6.0, 12.0, 9.0].into_iter().enumerate() {
                let left = center.x - 6.5 + n as f32 * 5.0;
                painter.rect_filled(Rect::from_min_max(Pos2::new(left, center.y + 6.0 - h), Pos2::new(left + 3.5, center.y + 6.0)), 1, color);
            }
        }
        Mark::Globe => {
            painter.circle_stroke(center, 7.0, line);
            painter.add(Shape::ellipse_stroke(center, Vec2::new(3.0, 7.0), line));
            painter.hline((center.x - 7.0)..=(center.x + 7.0), center.y, line);
        }
        Mark::Focal => {
            let frame = Rect::from_center_size(center, Vec2::splat(14.0));
            for (corner, dx, dy) in [(frame.left_top(), 1.0, 1.0), (frame.right_top(), -1.0, 1.0), (frame.left_bottom(), 1.0, -1.0), (frame.right_bottom(), -1.0, -1.0)] {
                painter.add(Shape::line(vec![corner + Vec2::new(4.0 * dx, 0.0), corner, corner + Vec2::new(0.0, 4.0 * dy)], line));
            }
            painter.circle_filled(center, 2.0, color);
        }
        Mark::Pie => {
            painter.circle_stroke(center, 7.0, line);
            painter.line_segment([center, center + Vec2::new(0.0, -7.0)], line);
            painter.line_segment([center, center + Vec2::angled(0.6) * 7.0], line);
        }
    }
}

/// A column chart of `bars`, `height` tall with the labels under it and
/// light lines at round counts, grown `grow` of the way. The column under
/// the pointer is lit and tells its count. Returns the column clicked, when
/// there is a `hint` to say what a click does.
fn columns(ui: &mut egui::Ui, bars: &[(String, usize)], height: f32, grow: f32, hint: Option<&str>) -> Option<usize> {
    const LABEL_H: f32 = 20.0;
    const SCALE_W: f32 = 34.0;
    let width = ui.available_width();
    let sense = if hint.is_some() { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height + LABEL_H), sense);
    if bars.is_empty() || !ui.is_rect_visible(rect) {
        return None;
    }
    let painter = ui.painter();
    let plot = Rect::from_min_max(Pos2::new(rect.left() + SCALE_W, rect.top() + 8.0), Pos2::new(rect.right(), rect.top() + height));
    let most = bars.iter().map(|(_, count)| *count).max().unwrap_or(0).max(1);
    let step = nice_step(most as f32 / 3.0);
    let top = (most as f32 / step).ceil() * step;
    let small = FontId::proportional(10.5);
    painter.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, theme::hairline()));
    let mut mark = step;
    while mark <= top + 0.5 {
        let y = plot.bottom() - plot.height() * mark / top;
        painter.hline(plot.x_range(), y, Stroke::new(1.0, theme::hairline().gamma_multiply(0.6)));
        painter.text(Pos2::new(plot.left() - 8.0, y), Align2::RIGHT_CENTER, grouped(mark as usize), small.clone(), theme::muted());
        mark += step;
    }
    let slot = plot.width() / bars.len() as f32;
    let gap = (slot * 0.28).clamp(1.0, 14.0);
    let hovered = response
        .hover_pos()
        .filter(|pos| plot.x_range().contains(pos.x))
        .map(|pos| (((pos.x - plot.left()) / slot).floor() as usize).min(bars.len() - 1));
    let label_font = FontId::proportional(11.0);
    let widest = bars.iter().map(|(label, _)| painter.layout_no_wrap(label.clone(), label_font.clone(), theme::muted()).size().x).fold(0.0, f32::max);
    let every = ((widest + 6.0) / slot).ceil().max(1.0) as usize;
    for (index, (label, count)) in bars.iter().enumerate() {
        let left = plot.left() + index as f32 * slot + gap / 2.0;
        let full = plot.height() * *count as f32 / top;
        let h = if *count == 0 { 0.0 } else { (full * wave(grow, index as f32 / bars.len() as f32)).max(2.0) };
        let bar = Rect::from_min_max(Pos2::new(left, plot.bottom() - h), Pos2::new(left + slot - gap, plot.bottom()));
        let lit = hovered == Some(index);
        let color = match hovered {
            None => theme::accent().gamma_multiply(0.85),
            Some(_) if lit => theme::accent(),
            Some(_) => theme::accent().gamma_multiply(0.35),
        };
        let radius = ((slot - gap) / 2.0).min(5.0) as u8;
        painter.rect_filled(bar, CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 }, color);
        if index % every == 0 || lit {
            let color = if lit { theme::text() } else { theme::muted() };
            painter.text(Pos2::new(bar.center().x, plot.bottom() + 4.0), Align2::CENTER_TOP, label, label_font.clone(), color);
        }
    }
    let index = hovered?;
    let (label, count) = &bars[index];
    let anchor = Pos2::new(plot.left() + (index as f32 + 0.5) * slot, plot.bottom() - plot.height() * *count as f32 / top - 4.0);
    hover_card(ui, anchor, label, &super::photo_count(*count), hint);
    if hint.is_some() && response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        return Some(index);
    }
    None
}

/// A round step near `raw`: 1, 2, or 5 times a power of ten, at least 1.
fn nice_step(raw: f32) -> f32 {
    if raw <= 1.0 {
        return 1.0;
    }
    let magnitude = 10f32.powf(raw.log10().floor());
    let scaled = raw / magnitude;
    let nice = if scaled <= 1.0 {
        1.0
    } else if scaled <= 2.0 {
        2.0
    } else if scaled <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

/// A ring cut into `parts`, each a share and a color, drawn `grow` of the
/// way round, with `value` and `label` in the middle. Returns its response
/// and the part under the pointer, which stands out.
fn donut(ui: &mut egui::Ui, size: f32, parts: &[(f32, Color32)], grow: f32, value: &str, label: &str) -> (Response, Option<usize>) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let center = rect.center();
    let thick = size * 0.13;
    let radius = size / 2.0 - thick / 2.0 - 4.0;
    let hovered = response.hover_pos().and_then(|pos| {
        let offset = pos - center;
        if (offset.length() - radius).abs() > thick / 2.0 + 4.0 {
            return None;
        }
        let turn = (offset.y.atan2(offset.x) + FRAC_PI_2).rem_euclid(TAU) / TAU;
        let mut end = 0.0;
        parts.iter().position(|(share, _)| {
            end += share * grow;
            turn < end
        })
    });
    let painter = ui.painter();
    painter.circle_stroke(center, radius, Stroke::new(thick, theme::well()));
    let gap = if parts.iter().filter(|(share, _)| *share > 0.0).count() > 1 { 0.004 } else { 0.0 };
    let mut start = 0.0;
    for (index, (share, color)) in parts.iter().enumerate() {
        let sweep = share * grow;
        let (width, color) = match hovered {
            Some(lit) if lit == index => (thick + 6.0, *color),
            Some(_) => (thick, color.gamma_multiply(0.5)),
            None => (thick, *color),
        };
        arc(painter, center, radius, start + gap, start + sweep - gap, width, color);
        start += sweep;
    }
    if label.is_empty() {
        painter.text(center, Align2::CENTER_CENTER, value, FontId::proportional(size * 0.16), theme::text());
    } else {
        painter.text(center - Vec2::new(0.0, size * 0.05), Align2::CENTER_CENTER, value, FontId::proportional(size * 0.15), theme::text());
        painter.text(center + Vec2::new(0.0, size * 0.1), Align2::CENTER_CENTER, label, FontId::proportional(11.0), theme::muted());
    }
    (response, hovered)
}

/// A stretch of a circle from `from` to `to`, in turns from the top.
fn arc(painter: &egui::Painter, center: Pos2, radius: f32, from: f32, to: f32, width: f32, color: Color32) {
    if to <= from {
        return;
    }
    let steps = ((to - from) * 120.0).ceil().max(2.0) as usize;
    let points: Vec<Pos2> = (0..=steps)
        .map(|step| center + Vec2::angled((from + (to - from) * step as f32 / steps as f32) * TAU - FRAC_PI_2) * radius)
        .collect();
    painter.add(Shape::line(points, Stroke::new(width, color)));
}

/// A name in a donut's legend with its color and share. Returns its
/// response.
fn legend_row(ui: &mut egui::Ui, color: Color32, name: &str, count: usize, total: usize, lit: bool) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if lit || response.hovered() {
        painter.rect_filled(rect, 6, theme::hover());
    }
    painter.circle_filled(Pos2::new(rect.left() + 10.0, rect.center().y), 4.5, color);
    let figure = painter.layout_no_wrap(format!("{}%", percent(count, total)), FontId::proportional(12.0), theme::text());
    let figure_w = figure.size().x;
    painter.galley(Pos2::new(rect.right() - 6.0 - figure_w, rect.center().y - figure.size().y / 2.0), figure, theme::text());
    let title = truncated(painter, name.to_string(), 12.5, theme::text(), width - 22.0 - figure_w - 14.0);
    painter.galley(Pos2::new(rect.left() + 22.0, rect.center().y - title.size().y / 2.0), title, theme::text());
    response.on_hover_text(super::photo_count(count))
}

/// A small card above `anchor` naming what is pointed at, drawn over
/// everything else and kept on the page.
fn hover_card(ui: &egui::Ui, anchor: Pos2, title: &str, body: &str, foot: Option<&str>) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("stats-hover")));
    let title = painter.layout_no_wrap(title.to_string(), FontId::proportional(12.5), theme::text());
    let body = painter.layout_no_wrap(body.to_string(), FontId::proportional(11.5), theme::muted());
    let foot = foot.map(|text| painter.layout_no_wrap(text.to_string(), FontId::proportional(11.0), theme::accent()));
    let inner_w = title.size().x.max(body.size().x).max(foot.as_ref().map_or(0.0, |foot| foot.size().x));
    let inner_h = title.size().y + 2.0 + body.size().y + foot.as_ref().map_or(0.0, |foot| foot.size().y + 2.0);
    let size = Vec2::new(inner_w + 20.0, inner_h + 14.0);
    let bounds = ui.clip_rect();
    let mut min = Pos2::new(anchor.x - size.x / 2.0, anchor.y - size.y - 8.0);
    if min.y < bounds.top() + 2.0 {
        min.y = anchor.y + 22.0;
    }
    min.x = min.x.clamp(bounds.left() + 2.0, (bounds.right() - size.x - 2.0).max(bounds.left() + 2.0));
    let rect = Rect::from_min_size(min, size);
    painter.add(theme::card_shadow().as_shape(rect, 8));
    painter.rect(rect, 8, popup_fill(), Stroke::new(1.0, theme::hairline()), StrokeKind::Inside);
    let mut y = rect.top() + 7.0;
    for (galley, color) in [Some((title, theme::text())), Some((body, theme::muted())), foot.map(|foot| (foot, theme::accent()))].into_iter().flatten() {
        let height = galley.size().y;
        painter.galley(Pos2::new(rect.left() + 10.0, y), galley, color);
        y += height + 2.0;
    }
}

fn photos_or_none(count: usize) -> String {
    if count == 0 {
        tr("No photos here").to_string()
    } else {
        super::photo_count(count)
    }
}

/// Fill of a cell with `count` of the `most` any cell has: empty, then
/// four steps of the accent color.
fn heat(count: usize, most: usize) -> Color32 {
    if count == 0 {
        return theme::well();
    }
    let t = if most <= 1 { 1.0 } else { ((count as f32).ln() / (most as f32).ln()).clamp(0.0, 1.0) };
    theme::accent().gamma_multiply(heat_alpha((t * 3.0).round() / 3.0))
}

fn heat_alpha(level: f32) -> f32 {
    0.28 + 0.72 * level
}

/// How far something `at` a fraction across has grown in when the whole
/// has grown `grow`: the left first, sweeping right.
fn wave(grow: f32, at: f32) -> f32 {
    (grow * 1.5 - at * 0.5).clamp(0.0, 1.0)
}

/// Texture coordinates that fill `rect` with the middle of a `width` by
/// `height` texture turned by `rotation` degrees.
fn fill_uv(width: usize, height: usize, rotation: i32, rect: Rect) -> Rect {
    let turned = rotation.div_euclid(90).rem_euclid(2) == 1;
    let (w, h) = if turned { (height as f32, width as f32) } else { (width as f32, height as f32) };
    let image = w / h.max(1.0);
    let target = rect.width() / rect.height().max(1.0);
    let (keep_x, keep_y) = if image > target { (target / image, 1.0) } else { (1.0, image / target) };
    let (keep_u, keep_v) = if turned { (keep_y, keep_x) } else { (keep_x, keep_y) };
    Rect::from_min_max(Pos2::new((1.0 - keep_u) / 2.0, (1.0 - keep_v) / 2.0), Pos2::new((1.0 + keep_u) / 2.0, (1.0 + keep_v) / 2.0))
}

/// A rectangle shaded from `top` to `bottom`.
fn gradient(rect: Rect, top: Color32, bottom: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
}

/// Behind the cards: a shade off the cards in the light theme.
fn page_fill() -> Color32 {
    if theme::is_dark() {
        theme::bg()
    } else {
        theme::sidebar()
    }
}

fn card_fill() -> Color32 {
    if theme::is_dark() {
        theme::sidebar()
    } else {
        WHITE
    }
}

fn popup_fill() -> Color32 {
    if theme::is_dark() {
        Color32::from_rgb(0x24, 0x2C, 0x2F)
    } else {
        WHITE
    }
}

/// The slice of everything else in a donut.
fn other_color() -> Color32 {
    if theme::is_dark() {
        Color32::from_rgb(0x4A, 0x54, 0x58)
    } else {
        Color32::from_rgb(0xCB, 0xC8, 0xC0)
    }
}

fn soft_shadow() -> egui::Shadow {
    egui::Shadow { offset: [0, 2], blur: 12, spread: 0, color: Color32::from_black_alpha(if theme::is_dark() { 70 } else { 14 }) }
}

fn paint_card(painter: &egui::Painter, rect: Rect, radius: u8) {
    painter.add(soft_shadow().as_shape(rect, radius));
    painter.rect(rect, radius, card_fill(), Stroke::new(1.0, theme::hairline()), StrokeKind::Inside);
}

fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::muted()));
}

fn subheading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).strong().color(theme::muted()));
    ui.add_space(2.0);
}

/// One name with its count and share of `total`, and a bar as long as its
/// part of `most`. Returns whether it was clicked.
fn bar_row(ui: &mut egui::Ui, name: &str, count: usize, most: usize, total: usize) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, BAR_ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Show these photos"));
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 8, theme::hover());
    }
    let inner = rect.shrink2(Vec2::new(8.0, 6.0));
    let figure = format!("{}  {}%", grouped(count), percent(count, total));
    let figure = painter.layout_no_wrap(figure, FontId::proportional(12.0), theme::muted());
    let title = truncated(painter, name.to_string(), 13.0, theme::text(), inner.width() - figure.size().x - 10.0);
    painter.galley(Pos2::new(inner.right() - figure.size().x, inner.top() + 1.0), figure, theme::muted());
    painter.galley(inner.min, title, theme::text());
    let track = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - 6.0), inner.max);
    meter(painter, track, count as f32 / most.max(1) as f32, theme::accent());
    response.clicked()
}

/// A rounded bar filled `share` of the way.
pub(super) fn meter(painter: &egui::Painter, rect: Rect, share: f32, color: Color32) {
    let radius = (rect.height() / 2.0) as u8;
    painter.rect_filled(rect, radius, theme::well());
    if share > 0.0 {
        let fill = Rect::from_min_size(rect.min, Vec2::new((rect.width() * share.min(1.0)).max(rect.height()), rect.height()));
        painter.rect_filled(fill, radius, color);
    }
}

/// "And {} more, {} photos" under a list cut short.
fn more_line(ui: &mut egui::Ui, more: usize, photos: usize) {
    ui.add_space(2.0);
    let text = trf("and {} more · {}", &[&grouped(more), &super::photo_count(photos)]);
    ui.label(RichText::new(text).size(11.5).color(theme::muted()));
}

/// The line under a ranked list that shows all of it or cuts it short
/// again. Returns whether it was clicked.
fn more_link(ui: &mut egui::Ui, text: String, hint: &str) -> bool {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(theme::accent())).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(hint)
        .clicked()
}

/// `part` of `total` as a whole percentage, at least 1 when `part` isn't 0.
pub(super) fn percent(part: usize, total: usize) -> usize {
    if part == 0 || total == 0 {
        return 0;
    }
    ((part as f64 * 100.0 / total as f64).round() as usize).clamp(1, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(id: i64, taken_at: i64, camera: &str, lens: &str, focal_mm: f32, place: Option<(f64, f64)>) -> Photo {
        Photo {
            id,
            folder_id: 1,
            path: format!("/photos/trip/{id}.jpg"),
            filename: format!("{id}.jpg"),
            width: 10,
            height: 10,
            bytes: 100,
            taken_at,
            modified_at: taken_at,
            starred: false,
            caption: String::new(),
            camera: camera.into(),
            lens: lens.into(),
            focal_mm,
            gps_lat: place.map(|(lat, _)| lat),
            gps_lon: place.map(|(_, lon)| lon),
            pin_lat: None,
            pin_lon: None,
            rotation: 0,
            duration: 0.0,
        }
    }

    fn at(year: i32, month: u32, day: u32, hour: u32) -> i64 {
        use chrono::TimeZone;
        chrono::Local.with_ymd_and_hms(year, month, day, hour, 0, 0).earliest().unwrap().timestamp()
    }

    fn sample() -> Vec<Photo> {
        let taipei = Some((25.03, 121.56));
        let paris = Some((48.85, 2.35));
        vec![
            photo(1, at(2020, 3, 2, 9), "Canon EOS R5", "RF24-105mm F4 L IS USM", 24.0, taipei),
            photo(2, at(2020, 3, 2, 10), "Canon EOS R5", "RF24-105mm F4 L IS USM", 105.0, taipei),
            photo(3, at(2022, 7, 9, 18), "iPhone 13", "", 26.0, paris),
            photo(4, at(2022, 7, 10, 18), " ", "", 0.0, None),
        ]
    }

    #[test]
    fn counts_time_cameras_lenses_and_places() {
        let photos = sample();
        let stats = compute(&photos.iter().collect::<Vec<_>>());
        assert_eq!(stats.photos, 4);
        assert_eq!(stats.years, vec![(2020, 2), (2021, 0), (2022, 2)]);
        assert_eq!(stats.months[2], 2);
        assert_eq!(stats.months[6], 2);
        assert_eq!(stats.week_hours[0].iter().sum::<usize>(), 2, "2020-03-02 was a Monday");
        assert_eq!(stats.week_hours.iter().map(|hours| hours[18]).sum::<usize>(), 2);
        assert_eq!(stats.cameras.rows, vec![("Canon EOS R5".to_string(), 2), ("iPhone 13".to_string(), 1)]);
        assert_eq!(stats.cameras.unknown, 1);
        assert_eq!(stats.lenses.rows.len(), 1);
        assert_eq!(stats.lenses.unknown, 2);
        assert_eq!(stats.focal, [0, 2, 0, 0, 1, 0, 0]);
        assert_eq!(stats.geotagged, 3);
        let places: Vec<usize> = stats.places.iter().map(|place| place.count).collect();
        assert_eq!(places, vec![2, 1]);
        assert_eq!(stats.places[0].label, "trip");
    }

    #[test]
    fn finds_the_highlights() {
        let photos = sample();
        let stats = compute(&photos.iter().collect::<Vec<_>>());
        assert_eq!(stats.days.len(), 3);
        let day = stats.busiest_day.as_ref().unwrap();
        assert_eq!((day.count, day.cover.id), (2, 1));
        let month = stats.busiest_month.as_ref().unwrap();
        assert_eq!((month.count, month.at), (2, photos[0].taken_at), "ties go to the earlier month");
        let streak = stats.streak.as_ref().unwrap();
        assert_eq!((streak.days, streak.cover.id, streak.last), (2, 3, photos[3].taken_at));
        assert_eq!(stats.first.as_ref().map(|first| first.cover.id), Some(1));
        assert_eq!(stats.totals[&2022].days, 2);
        assert_eq!(stats.totals[&2020].photos, 2);
        assert_eq!(stats.span, Some((photos[0].taken_at, photos[3].taken_at)));
    }

    #[test]
    fn one_year_counts_only_its_photos() {
        let photos = sample();
        let stats = compute(&in_year(&photos, 2022));
        assert_eq!(stats.photos, 2);
        assert_eq!(stats.years, vec![(2022, 2)]);
        assert_eq!(stats.cameras.rows, vec![("iPhone 13".to_string(), 1)]);
    }

    #[test]
    fn the_latest_of_the_longest_runs_wins() {
        assert_eq!(longest_run(vec![10, 1, 2, 3, 11, 12, 20]), Some((10, 3)));
        assert_eq!(longest_run(vec![5]), Some((5, 1)));
        assert_eq!(longest_run(Vec::new()), None);
    }

    #[test]
    fn chart_lines_fall_on_round_counts() {
        assert_eq!(nice_step(0.4), 1.0);
        assert_eq!(nice_step(13.3), 20.0);
        assert_eq!(nice_step(40.0), 50.0);
        assert_eq!(nice_step(700.0), 1000.0);
    }

    #[test]
    fn covers_fill_their_box() {
        let wide = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
        let uv = fill_uv(100, 100, 0, wide);
        assert!((uv.width() - 1.0).abs() < 1e-4 && (uv.height() - 0.5).abs() < 1e-4);
        let uv = fill_uv(400, 100, 90, wide);
        assert!((uv.width() - 0.125).abs() < 1e-4 && (uv.height() - 1.0).abs() < 1e-4, "a turned photo is cropped along its other side");
    }

    #[test]
    fn percentages_round_but_never_hide_a_few() {
        assert_eq!(percent(0, 10), 0);
        assert_eq!(percent(1, 1000), 1);
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(5, 0), 0);
    }
}
