//! Statistics: when, where, and with what the photos in view were taken.
//! Years, the busiest day, cameras, and lenses lead to the matching photos
//! in the Library; a place leads to the map.

use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, Timelike};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::geo;
use crate::i18n::{tr, trf};
use crate::track::{format_distance, Bounds};
use crate::types::{format_bytes, format_day, short_month, short_weekday, Nav, Photo};

use super::places::{cluster_points, date_span, folder_summary, grouped, truncated};
use super::theme::{self, ACCENT};
use super::travel::Visits;
use super::{Mode, PicassoApp};

/// Rows a ranked list shows; the rest are summed up below it.
const TOP: usize = 8;
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
const CARD_GAP: f32 = 14.0;

/// Counts over the photos in view, worked out once per view.
pub(super) struct Stats {
    epoch: u64,
    photos: usize,
    videos: usize,
    bytes: u64,
    /// Photos per year, oldest first, with the empty years between.
    years: Vec<(i32, usize)>,
    months: [usize; 12],
    /// Monday first.
    weekdays: [usize; 7],
    hours: [usize; 24],
    /// The day with the most photos, as a timestamp in it, and their count.
    busiest: Option<(i64, usize)>,
    geotagged: usize,
    /// Every place, biggest first.
    places: Vec<Place>,
    cameras: Ranked,
    lenses: Ranked,
    focal: [usize; FOCAL_BANDS.len()],
    /// Most used focal length in whole millimeters, and how often.
    focal_top: Option<(u32, usize)>,
    /// The countries and states the photos were taken in.
    travel: Rc<Visits>,
}

struct Place {
    /// Folder most of its photos are in.
    label: String,
    count: usize,
    first: i64,
    last: i64,
    /// Photo shown for it: id, path, rotation.
    cover: (i64, String, i32),
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

/// Works out the statistics of `photos`.
fn compute(photos: &[Photo], epoch: u64) -> Stats {
    let mut stats = Stats {
        epoch,
        photos: 0,
        videos: 0,
        bytes: 0,
        years: Vec::new(),
        months: [0; 12],
        weekdays: [0; 7],
        hours: [0; 24],
        busiest: None,
        geotagged: 0,
        places: Vec::new(),
        cameras: Ranked::default(),
        lenses: Ranked::default(),
        focal: [0; FOCAL_BANDS.len()],
        focal_top: None,
        travel: Rc::new(Visits::compute(photos, epoch)),
    };
    let mut years: HashMap<i32, usize> = HashMap::new();
    let mut days: HashMap<i32, (i64, usize)> = HashMap::new();
    let mut cameras: HashMap<String, usize> = HashMap::new();
    let mut lenses: HashMap<String, usize> = HashMap::new();
    let mut focals: HashMap<u32, usize> = HashMap::new();
    let (mut no_camera, mut no_lens) = (0, 0);
    let mut located: Vec<usize> = Vec::new();
    for (index, photo) in photos.iter().enumerate() {
        if photo.is_video() {
            stats.videos += 1;
        } else {
            stats.photos += 1;
        }
        stats.bytes += photo.bytes;
        if let Some(time) = chrono::DateTime::from_timestamp(photo.taken_at, 0) {
            let time = time.with_timezone(&chrono::Local);
            *years.entry(time.year()).or_default() += 1;
            stats.months[time.month0() as usize] += 1;
            stats.weekdays[time.weekday().num_days_from_monday() as usize] += 1;
            stats.hours[time.hour() as usize] += 1;
            days.entry(time.num_days_from_ce()).or_insert((photo.taken_at, 0)).1 += 1;
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
    if let (Some(first), Some(last)) = (years.keys().min(), years.keys().max()) {
        stats.years = (*first..=*last).map(|year| (year, years.get(&year).copied().unwrap_or(0))).collect();
    }
    stats.busiest = days.into_values().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
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
            let cover = &photos[located[members[0]]];
            Place {
                label: folder_summary(members.iter().map(|member| photos[located[*member]].path.as_str())),
                count: members.len(),
                first,
                last,
                cover: (cover.id, cover.path.clone(), cover.rotation),
                bounds,
            }
        })
        .collect();
    stats
}

impl PicassoApp {
    pub(super) fn stats_ui(&mut self, ui: &mut egui::Ui) {
        if self.stats.as_ref().is_none_or(|stats| stats.epoch != self.view_epoch) {
            self.stats = Some(Rc::new(compute(&self.photos, self.view_epoch)));
        }
        let Some(stats) = self.stats.clone() else {
            return;
        };
        let mut go: Option<Go> = None;
        egui::ScrollArea::vertical().id_salt("stats").auto_shrink([false, false]).show(ui, |ui| {
            let width = ui.available_width().min(MAX_W);
            ui.set_max_width(width);
            ui.add_space(4.0);
            self.overview(ui, &stats, width);
            ui.add_space(CARD_GAP);
            card(ui, width, tr("Time"), |ui| time_section(ui, &stats, &mut go));
            ui.add_space(CARD_GAP);
            let two = width >= 760.0;
            let half = if two { (width - CARD_GAP) / 2.0 } else { width };
            let mut camera = None;
            pair(ui, two, |ui| card(ui, half, tr("Places"), |ui| self.places_section(ui, &stats, &mut go)), |ui| {
                card(ui, half, tr("Cameras"), |ui| ranked_section(ui, &stats.cameras, "{} without camera info", &mut camera))
            });
            go = go.take().or(camera);
            ui.add_space(CARD_GAP);
            card(ui, width, tr("Travel"), |ui| {
                if self.travel_card(ui, &stats.travel) {
                    go = Some(Go::TravelMap);
                }
            });
            ui.add_space(CARD_GAP);
            pair(ui, two, |ui| card(ui, half, tr("Lenses"), |ui| ranked_section(ui, &stats.lenses, "{} without lens info", &mut go)), |ui| {
                card(ui, half, tr("Focal length"), |ui| focal_section(ui, &stats))
            });
            ui.add_space(24.0);
        });
        match go {
            Some(Go::Search(query)) => self.search_library(query),
            Some(Go::Map(bounds)) => self.show_area_on_map(bounds),
            Some(Go::TravelMap) => self.set_nav(Nav::TravelMap),
            None => {}
        }
    }

    /// Big numbers across the top: how many, how much, how long, where, and
    /// with how many cameras and lenses.
    fn overview(&self, ui: &mut egui::Ui, stats: &Stats, width: f32) {
        let span = match (stats.years.first(), stats.years.last()) {
            (Some((first, _)), Some((last, _))) if first == last => first.to_string(),
            (Some((first, _)), Some((last, _))) => format!("{first}–{last}"),
            _ => "—".into(),
        };
        let share = percent(stats.geotagged, stats.photos + stats.videos);
        let mut tiles = vec![(grouped(stats.photos), tr("Photos").to_string())];
        if stats.videos > 0 {
            tiles.push((grouped(stats.videos), tr("Videos").to_string()));
        }
        tiles.extend([
            (format_bytes(stats.bytes), tr("Total size").to_string()),
            (span, tr("Years").to_string()),
            (format!("{share}%"), tr("With a location").to_string()),
            (grouped(stats.cameras.rows.len()), tr("Cameras").to_string()),
            (grouped(stats.lenses.rows.len()), tr("Lenses").to_string()),
        ]);
        let per_row = ((width + CARD_GAP) / (130.0 + CARD_GAP)).floor().clamp(1.0, tiles.len() as f32) as usize;
        let tile_w = (width - CARD_GAP * (per_row - 1) as f32) / per_row as f32;
        for row in tiles.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = CARD_GAP;
                for (value, label) in row {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(tile_w, 70.0), Sense::hover());
                    let painter = ui.painter();
                    painter.rect(rect, 12, theme::sidebar(), egui::Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
                    let inner = rect.shrink2(Vec2::new(14.0, 12.0));
                    let value = truncated(painter, value.clone(), 22.0, theme::text(), inner.width());
                    painter.galley(inner.min, value, theme::text());
                    let label = truncated(painter, label.clone(), 12.0, theme::muted(), inner.width());
                    painter.galley(Pos2::new(inner.left(), inner.bottom() - label.size().y), label, theme::muted());
                }
            });
            ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
        }
    }

    /// How many photos have a location, the places with the most, and the
    /// GPS tracks.
    fn places_section(&mut self, ui: &mut egui::Ui, stats: &Stats, go: &mut Option<Go>) {
        let total = stats.photos + stats.videos;
        let share = percent(stats.geotagged, total);
        ui.label(egui::RichText::new(trf("{}% of photos have a location", &[&share])).size(13.0).color(theme::text()));
        ui.label(egui::RichText::new(trf("{} of {} photos", &[&grouped(stats.geotagged), &grouped(total)])).size(12.0).color(theme::muted()));
        ui.add_space(4.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
        meter(ui.painter(), rect, stats.geotagged as f32 / total.max(1) as f32, ACCENT);
        if !self.tracks.is_empty() {
            let meters: f64 = self.tracks.iter().map(|track| track.meters).sum();
            let count = trf(if self.tracks.len() == 1 { "{} track" } else { "{} tracks" }, &[&self.tracks.len()]);
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("{count} · {}", format_distance(meters))).size(12.0).color(theme::muted()));
        }
        if stats.places.is_empty() {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(tr("Photos with a location show up here, grouped by place.")).size(12.0).color(theme::muted()));
            return;
        }
        ui.add_space(12.0);
        let heading = trf(if stats.places.len() == 1 { "{} place" } else { "{} places" }, &[&grouped(stats.places.len())]);
        ui.label(egui::RichText::new(heading).size(12.0).strong().color(theme::muted()));
        ui.add_space(2.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        for place in stats.places.iter().take(TOP) {
            if self.place_row(ui, place) {
                *go = Some(Go::Map(place.bounds));
            }
        }
        if stats.places.len() > TOP {
            let rest: usize = stats.places[TOP..].iter().map(|place| place.count).sum();
            more_line(ui, stats.places.len() - TOP, rest);
        }
    }

    /// One place: a photo from it, its folder, when, and how many. Returns
    /// whether it was clicked.
    fn place_row(&mut self, ui: &mut egui::Ui, place: &Place) -> bool {
        const THUMB: f32 = 40.0;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, THUMB + 10.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Show on map"));
        let painter = ui.painter().clone();
        if response.hovered() {
            painter.rect_filled(rect, 8, theme::select());
        }
        let (id, path, rotation) = &place.cover;
        self.ensure_thumb(*id, path);
        let tile = Rect::from_min_size(rect.min + Vec2::new(5.0, 5.0), Vec2::splat(THUMB));
        painter.rect_filled(tile, 6, theme::placeholder());
        if let Some((texture, w, h)) = self.thumb_texture(*id) {
            theme::paint_image_rounded(&painter, &texture, tile, theme::cover_uv(w, h), *rotation, 6.0);
        }
        let left = tile.right() + 10.0;
        let number = painter.layout_no_wrap(grouped(place.count), FontId::proportional(12.0), theme::muted());
        let room = rect.right() - 8.0 - left;
        let title = truncated(&painter, place.label.clone(), 13.0, theme::text(), room - number.size().x - 8.0);
        let span = truncated(&painter, date_span([place.first, place.last].into_iter()), 11.5, theme::muted(), room);
        painter.galley(Pos2::new(rect.right() - 8.0 - number.size().x, rect.top() + 9.0), number, theme::muted());
        painter.galley(Pos2::new(left, rect.top() + 8.0), title, theme::text());
        painter.galley(Pos2::new(left, rect.top() + 27.0), span, theme::muted());
        response.clicked()
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
        self.set_nav(Nav::Places);
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
}

/// A titled card `width` wide.
fn card(ui: &mut egui::Ui, width: f32, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::sidebar())
        .stroke(egui::Stroke::new(1.0, theme::hairline()))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(width - 32.0);
                ui.label(egui::RichText::new(title).size(16.0).strong().color(theme::text()));
                ui.add_space(8.0);
                add(ui);
            });
        });
}

/// Two cards side by side, or one under the other when `two` is false.
fn pair(ui: &mut egui::Ui, two: bool, left: impl FnOnce(&mut egui::Ui), right: impl FnOnce(&mut egui::Ui)) {
    if two {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP;
            left(ui);
            right(ui);
        });
    } else {
        left(ui);
        ui.add_space(CARD_GAP);
        right(ui);
    }
}

fn time_section(ui: &mut egui::Ui, stats: &Stats, go: &mut Option<Go>) {
    if stats.years.is_empty() {
        ui.label(egui::RichText::new(tr("Unknown date")).color(theme::muted()));
        return;
    }
    subheading(ui, tr("Photos per year"));
    let years: Vec<(String, usize)> = stats.years.iter().map(|(year, count)| (year.to_string(), *count)).collect();
    if let Some(index) = columns(ui, &years, 140.0, true) {
        *go = Some(Go::Search(stats.years[index].0.to_string()));
    }
    if let Some((day, count)) = stats.busiest {
        ui.add_space(6.0);
        let text = trf("Busiest day: {} · {}", &[&format_day(day), &super::photo_count(count)]);
        let link = ui
            .add(egui::Label::new(egui::RichText::new(text).size(12.0).color(theme::muted())).sense(Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Show these photos"));
        if link.clicked() {
            if let Some(date) = chrono::DateTime::from_timestamp(day, 0) {
                *go = Some(Go::Search(date.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string()));
            }
        }
    }
    ui.add_space(14.0);
    let months: Vec<(String, usize)> = (1..=12).map(|month| (short_month(month), stats.months[month as usize - 1])).collect();
    let weekdays: Vec<(String, usize)> = (0..7).map(|day| (short_weekday(day).to_string(), stats.weekdays[day])).collect();
    let hours: Vec<(String, usize)> = (0..24).map(|hour| (hour.to_string(), stats.hours[hour])).collect();
    let charts: [(&str, &[(String, usize)]); 3] = [("By month", &months), ("By weekday", &weekdays), ("By hour of day", &hours)];
    let wide = ui.available_width() >= 600.0;
    if wide {
        let third = (ui.available_width() - 2.0 * 24.0) / 3.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            for (title, bars) in charts {
                ui.vertical(|ui| {
                    ui.set_width(third);
                    subheading(ui, tr(title));
                    columns(ui, bars, 90.0, false);
                });
            }
        });
    } else {
        for (title, bars) in charts {
            subheading(ui, tr(title));
            columns(ui, bars, 90.0, false);
            ui.add_space(10.0);
        }
    }
}

/// Names with how many photos each, most first, then how many more there
/// are and how many photos have no name recorded.
fn ranked_section(ui: &mut egui::Ui, ranked: &Ranked, unknown: &'static str, go: &mut Option<Go>) {
    let known = ranked.known();
    if ranked.rows.is_empty() {
        ui.label(egui::RichText::new(tr("Photos taken with a camera that records it show up here.")).size(12.0).color(theme::muted()));
    }
    let most = ranked.rows.first().map_or(1, |(_, count)| *count);
    ui.spacing_mut().item_spacing.y = 2.0;
    for (name, count) in ranked.rows.iter().take(TOP) {
        if bar_row(ui, name, *count, most, known) {
            *go = Some(Go::Search(name.clone()));
        }
    }
    if ranked.rows.len() > TOP {
        let rest: usize = ranked.rows[TOP..].iter().map(|(_, count)| count).sum();
        more_line(ui, ranked.rows.len() - TOP, rest);
    }
    if ranked.unknown > 0 {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(trf(unknown, &[&grouped(ranked.unknown)])).size(11.5).color(theme::muted()));
    }
}

fn focal_section(ui: &mut egui::Ui, stats: &Stats) {
    let known: usize = stats.focal.iter().sum();
    if known == 0 {
        ui.label(egui::RichText::new(tr("Photos with a recorded focal length show up here.")).size(12.0).color(theme::muted()));
        return;
    }
    let bands: Vec<(String, usize)> = FOCAL_BANDS.iter().zip(stats.focal).map(|((_, label), count)| (label.to_string(), count)).collect();
    columns(ui, &bands, 150.0, false);
    ui.add_space(6.0);
    let mut note = tr("Millimeters, 35 mm equivalent").to_string();
    if let Some((mm, count)) = stats.focal_top {
        note = format!("{note} · {}", trf("Most used: {} mm ({})", &[&mm, &super::photo_count(count)]));
    }
    ui.label(egui::RichText::new(note).size(11.5).color(theme::muted()));
}

fn subheading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).strong().color(theme::muted()));
    ui.add_space(2.0);
}

/// A column chart of `bars`, `height` tall with the labels under it. The
/// column under the pointer is lit and says its count. Returns the column
/// clicked, when `clickable`.
fn columns(ui: &mut egui::Ui, bars: &[(String, usize)], height: f32, clickable: bool) -> Option<usize> {
    const LABEL_H: f32 = 18.0;
    const COUNT_H: f32 = 16.0;
    let width = ui.available_width();
    let sense = if clickable { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height + LABEL_H), sense);
    if bars.is_empty() || !ui.is_rect_visible(rect) {
        return None;
    }
    let painter = ui.painter();
    let plot = Rect::from_min_max(rect.min + Vec2::new(0.0, COUNT_H), Pos2::new(rect.right(), rect.top() + height));
    let slot = plot.width() / bars.len() as f32;
    let gap = (slot * 0.25).clamp(1.0, 10.0);
    let most = bars.iter().map(|(_, count)| *count).max().unwrap_or(0).max(1);
    let hovered = response
        .hover_pos()
        .filter(|pos| rect.contains(*pos))
        .map(|pos| (((pos.x - plot.left()) / slot).floor() as usize).min(bars.len() - 1));
    let label_font = FontId::proportional(11.0);
    let widest = bars.iter().map(|(label, _)| painter.layout_no_wrap(label.clone(), label_font.clone(), theme::muted()).size().x).fold(0.0, f32::max);
    let every = ((widest + 6.0) / slot).ceil().max(1.0) as usize;
    painter.line_segment([plot.left_bottom(), plot.right_bottom()], egui::Stroke::new(1.0, theme::hairline()));
    for (index, (label, count)) in bars.iter().enumerate() {
        let left = plot.left() + index as f32 * slot + gap / 2.0;
        let h = if *count == 0 { 0.0 } else { (plot.height() * *count as f32 / most as f32).max(2.0) };
        let bar = Rect::from_min_max(Pos2::new(left, plot.bottom() - h), Pos2::new(left + slot - gap, plot.bottom()));
        let lit = hovered == Some(index);
        let color = if lit { ACCENT } else { ACCENT.gamma_multiply(if theme::is_dark() { 0.75 } else { 0.6 }) };
        let radius = ((slot - gap) / 2.0).min(4.0) as u8;
        painter.rect_filled(bar, egui::CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 }, color);
        if lit {
            painter.text(Pos2::new(bar.center().x, bar.top() - 2.0), Align2::CENTER_BOTTOM, grouped(*count), FontId::proportional(11.5), theme::text());
        }
        if index % every == 0 || lit {
            let color = if lit { theme::text() } else { theme::muted() };
            painter.text(Pos2::new(bar.center().x, plot.bottom() + 3.0), Align2::CENTER_TOP, label, label_font.clone(), color);
        }
    }
    if clickable {
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            return hovered;
        }
    }
    None
}

/// One name with its count and share of `total`, and a bar as long as its
/// part of `most`. Returns whether it was clicked.
fn bar_row(ui: &mut egui::Ui, name: &str, count: usize, most: usize, total: usize) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Show these photos"));
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 8, theme::select());
    }
    let inner = rect.shrink2(Vec2::new(8.0, 6.0));
    let figure = format!("{}  {}%", grouped(count), percent(count, total));
    let figure = painter.layout_no_wrap(figure, FontId::proportional(12.0), theme::muted());
    let title = truncated(painter, name.to_string(), 13.0, theme::text(), inner.width() - figure.size().x - 10.0);
    painter.galley(Pos2::new(inner.right() - figure.size().x, inner.top() + 1.0), figure, theme::muted());
    painter.galley(inner.min, title, theme::text());
    let track = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - 6.0), inner.max);
    meter(painter, track, count as f32 / most.max(1) as f32, ACCENT);
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
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(text).size(11.5).color(theme::muted()));
    });
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

    #[test]
    fn counts_time_cameras_lenses_and_places() {
        let taipei = Some((25.03, 121.56));
        let paris = Some((48.85, 2.35));
        let photos = vec![
            photo(1, at(2020, 3, 2, 9), "Canon EOS R5", "RF24-105mm F4 L IS USM", 24.0, taipei),
            photo(2, at(2020, 3, 2, 10), "Canon EOS R5", "RF24-105mm F4 L IS USM", 105.0, taipei),
            photo(3, at(2022, 7, 9, 18), "iPhone 13", "", 26.0, paris),
            photo(4, at(2022, 7, 10, 18), " ", "", 0.0, None),
        ];
        let stats = compute(&photos, 7);
        assert_eq!(stats.photos, 4);
        assert_eq!(stats.years, vec![(2020, 2), (2021, 0), (2022, 2)]);
        assert_eq!(stats.months[2], 2);
        assert_eq!(stats.months[6], 2);
        assert_eq!(stats.weekdays[0], 2, "2020-03-02 was a Monday");
        assert_eq!(stats.hours[18], 2);
        assert_eq!(stats.busiest.map(|(_, count)| count), Some(2));
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
    fn percentages_round_but_never_hide_a_few() {
        assert_eq!(percent(0, 10), 0);
        assert_eq!(percent(1, 1000), 1);
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(5, 0), 0);
    }
}
