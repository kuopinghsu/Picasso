//! The track editor: a GPS track on the map, cleaned up as GPXViewer does,
//! with points to move, add, and remove, saved as GPX or KML.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use eframe::egui::{self, Align2, Color32, FontId, Key, Margin, Modifiers, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::geo;
use crate::i18n::{tr, trf};
use crate::track::format_distance;
use crate::track_edit::{self, EditPoint, Format, Loaded, Settings};
use crate::types::{folder_label, format_duration, format_timestamp};

use super::places::{
    grouped, pinch_steps, truncated, wheel_steps, zoom_control, zoom_step, MapView, MAP_BACKDROP, MAX_ZOOM, MIN_ZOOM, ZOOM_BUTTON,
};
use super::theme::{self, ACCENT, DANGER, WHITE};
use super::PicassoApp;

const PANEL_W: f32 = 320.0;
/// Space between the map's edge and the cards floating over it.
const INSET: f32 = 12.0;
/// Side of a tool button.
const TOOL: f32 = 32.0;
const PROFILE_H: f32 = 96.0;
/// Seconds a message stays over the map, and of its fade in and out.
const TOAST_SECONDS: f64 = 3.5;
const TOAST_FADE: f64 = 0.25;
/// How close, in points, the pointer must come to pick a point or a line.
const POINT_REACH: f32 = 8.0;
const LINE_REACH: f32 = 6.0;
/// Points are drawn one by one while no more than this many are in view.
const MAX_DRAWN_POINTS: usize = 4000;
/// Least distance, in points, between two drawn point handles.
const HANDLE_GAP: f32 = 9.0;
/// Edits kept for Undo.
const HISTORY: usize = 200;
/// Points kept for Undo and Redo together, about 60 MB, though the last
/// edit can always be undone.
const HISTORY_POINTS: usize = 1_000_000;
/// File in the library folder that keeps the unsaved edits of the open
/// track, for the next start after a quit or a crash.
const RECOVERY_FILE: &str = "track-recovery.json";
/// Seconds the edits rest before they are kept in [`RECOVERY_FILE`].
const KEEP_AFTER: f64 = 3.0;

/// Looked-up heights, one for each point and then each point as read.
type Heights = Vec<Option<f64>>;
/// Zoom level of the Start, End, and Highest views.
const CLOSE_ZOOM: u8 = 15;
const LINE: Color32 = Color32::from_rgb(0x1E, 0x6F, 0xD9);
const FLIGHT: Color32 = Color32::from_rgb(0xF0, 0x7C, 0x00);
const STAY: Color32 = Color32::from_rgb(0xF5, 0xB4, 0x00);
const START: Color32 = Color32::from_rgb(0x2E, 0x9D, 0x44);

/// Numbers worked out from the points, kept until the points or the
/// settings change rather than worked out on every frame.
struct Measures {
    revision: u64,
    settings: Settings,
    stats: track_edit::Stats,
    /// Whether each step from one point to the next is a flight.
    flights: Vec<bool>,
    /// Distance along the track to each point, in meters.
    along: Vec<f64>,
    /// Lowest and highest heights, when the track has heights.
    heights: Option<(f64, f64)>,
    /// Points without a height.
    unknown: usize,
}

impl Measures {
    fn of(points: &[EditPoint], settings: Settings, revision: u64) -> Measures {
        let mut along = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                total += track_edit::distance_m(&points[index - 1], point);
            }
            along.push(total);
        }
        let heights = has_heights(points).then(|| {
            points.iter().filter_map(|point| point.ele).fold((f64::MAX, f64::MIN), |(low, high), ele| (low.min(ele), high.max(ele)))
        });
        Measures {
            revision,
            settings,
            stats: track_edit::stats(points),
            flights: track_edit::flights(points, &settings),
            along,
            heights,
            unknown: points.iter().filter(|point| point.ele.is_none()).count(),
        }
    }
}

/// A file read and cleaned up on a thread.
struct Opened {
    loaded: Loaded,
    points: Vec<EditPoint>,
    settings: Settings,
}

#[derive(Clone, Copy, PartialEq)]
enum Preset {
    Full,
    Start,
    End,
    Highest,
}

/// The views, with their names and the keys that show them.
const PRESETS: [(Preset, &str, &str); 4] = [
    (Preset::Full, "Whole track", "1"),
    (Preset::Start, "Start", "2"),
    (Preset::End, "End", "3"),
    (Preset::Highest, "Highest point", "4"),
];

const TIPS: [&str; 5] = [
    "Drag a point to move it.",
    "Click the line to add a point.",
    "Right-click a point to remove it.",
    "Press R, drag a box round points, then press Delete.",
    "Ctrl+Z undoes, Ctrl+Y redoes.",
];

pub(super) struct TrackEditor {
    path: PathBuf,
    name: String,
    format: Format,
    /// The points as read, before the cleanup.
    raw: Vec<EditPoint>,
    points: Vec<EditPoint>,
    /// The settings in the panel, for the next cleanup.
    settings: Settings,
    /// The settings `points` were cleaned up with, which a save keeps.
    points_settings: Settings,
    /// The settings were saved in the file.
    settings_from_file: bool,
    /// The settings were changed and become the defaults once the pointer
    /// lets go.
    settings_unsaved: bool,
    /// Earlier points, each with the settings they were cleaned up with.
    undo: Vec<(Vec<EditPoint>, Settings)>,
    redo: Vec<(Vec<EditPoint>, Settings)>,
    /// Revision of the points last kept in [`RECOVERY_FILE`], and the
    /// revision and time of the last change not kept yet.
    kept_revision: Option<u64>,
    changed: Option<(u64, f64)>,
    /// Counts the changes to `points`, so `measures` knows when to update.
    revision: u64,
    measures: Option<Rc<Measures>>,
    /// Edited since it was opened or last saved.
    dirty: bool,
    selection: HashSet<usize>,
    /// Point whose details the panel shows.
    focus: Option<usize>,
    /// Point being dragged and where to.
    dragging: Option<(usize, f64, f64)>,
    /// Esc ended the drag of a point or a box before the button let go, so
    /// the rest of it doesn't pan the map.
    drag_cancelled: bool,
    /// Select mode, in which dragging draws a box round points.
    box_mode: bool,
    box_from: Option<Pos2>,
    show_raw: bool,
    view: MapView,
    wheel: f32,
    pinch: f32,
    /// The map waits for its size to fit the track.
    framed: bool,
    opening: Option<Receiver<Result<Opened, String>>>,
    cleaning: Option<Receiver<Vec<EditPoint>>>,
    /// Heights being looked up from terrain tiles, for `points` and then
    /// `raw`, with how far the download is.
    looking_up: Option<(Receiver<Heights>, Arc<geo::TerrainProgress>)>,
    /// The heights came from terrain data, whose sources are named under
    /// the profile.
    terrain_heights: bool,
    error: Option<String>,
    /// Message shown over the map for a few seconds.
    note: String,
    /// When the message appeared, or NaN until it is first drawn.
    note_at: f64,
    asking_close: bool,
    /// The window was asked to close, and closes once the edits are
    /// discarded.
    quit_after_discard: bool,
    /// Edits kept when Picasso quit, put in place of the points once the
    /// track is read.
    restored: Option<Loaded>,
    /// Point the context menu was opened on.
    menu_point: Option<usize>,
    /// Point under the pointer on the map, marked on the elevation profile.
    map_hover: Option<usize>,
    /// Point under the pointer on the elevation profile, marked on the map.
    profile_hover: Option<usize>,
}

impl TrackEditor {
    fn new(path: PathBuf, base: Settings, ctx: &egui::Context) -> TrackEditor {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (file, ctx) = (path.clone(), ctx.clone());
        let _ = std::thread::Builder::new().name("picasso-track".into()).spawn(move || {
            let opened = track_edit::load(&file, base).map(|loaded| {
                let settings = loaded.settings.unwrap_or(base);
                let points = if loaded.cleaned { loaded.points.clone() } else { track_edit::optimize(&loaded.points, &settings) };
                Opened { loaded, points, settings }
            });
            let _ = sender.send(opened);
            ctx.request_repaint();
        });
        TrackEditor {
            name: path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default(),
            path,
            format: Format::Gpx,
            raw: Vec::new(),
            points: Vec::new(),
            settings: base,
            points_settings: base,
            settings_from_file: false,
            settings_unsaved: false,
            undo: Vec::new(),
            redo: Vec::new(),
            kept_revision: None,
            changed: None,
            revision: 0,
            measures: None,
            dirty: false,
            selection: HashSet::new(),
            focus: None,
            dragging: None,
            drag_cancelled: false,
            box_mode: false,
            box_from: None,
            show_raw: false,
            view: MapView { lat: 0.0, lon: 0.0, zoom: MIN_ZOOM, pan: Vec2::ZERO },
            wheel: 0.0,
            pinch: 0.0,
            framed: false,
            opening: Some(receiver),
            cleaning: None,
            looking_up: None,
            terrain_heights: false,
            error: None,
            note: String::new(),
            note_at: f64::NAN,
            asking_close: false,
            quit_after_discard: false,
            restored: None,
            menu_point: None,
            map_hover: None,
            profile_hover: None,
        }
    }

    fn tell(&mut self, text: impl Into<String>) {
        self.note = text.into();
        self.note_at = f64::NAN;
    }

    /// Selects the point `index` and centers the map on it.
    fn show_point(&mut self, index: usize) {
        let point = self.points[index];
        self.selection = HashSet::from([index]);
        self.focus = Some(index);
        self.view = MapView { lat: point.lat, lon: point.lon, zoom: self.view.zoom, pan: Vec2::ZERO };
    }

    /// Moves the focus `by` points along the track, from either end when no
    /// point has it.
    fn step(&mut self, by: isize) {
        let Some(last) = self.points.len().checked_sub(1) else { return };
        let next = match self.focus {
            Some(index) => index.saturating_add_signed(by).min(last),
            None if by > 0 => 0,
            None => last,
        };
        self.show_point(next);
    }

    fn busy(&self) -> bool {
        self.opening.is_some() || self.cleaning.is_some() || self.looking_up.is_some()
    }

    /// Looks the heights of the points, and of the points as read, up from
    /// terrain tiles on a thread, so a later cleanup keeps them. Undo takes
    /// them off the points again.
    fn look_up_heights(&mut self, cache: &Path, ctx: &egui::Context) {
        let places: Vec<(f64, f64)> = self.points.iter().chain(&self.raw).map(|point| (point.lat, point.lon)).collect();
        let progress = Arc::new(geo::TerrainProgress::default());
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (cache, ctx, shared) = (cache.to_path_buf(), ctx.clone(), Arc::clone(&progress));
        let spawned = std::thread::Builder::new().name("picasso-terrain".into()).spawn(move || {
            let _ = sender.send(geo::terrain_heights(&places, &cache, &shared));
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.looking_up = Some((receiver, progress));
        }
    }

    /// Puts the looked-up `heights`, for `points` and then `raw`, in place:
    /// on every point when the track had no heights, else on the points
    /// without one, so heights the track came with are kept.
    fn take_heights(&mut self, heights: &[Option<f64>]) {
        let tidy = |height: f64| (height * 10.0).round() / 10.0;
        let (ours, raw) = heights.split_at(self.points.len().min(heights.len()));
        let all = !has_heights(&self.points);
        let found: Vec<(usize, f64)> = self
            .points
            .iter()
            .zip(ours)
            .enumerate()
            .filter(|(_, (point, _))| all || point.ele.is_none())
            .filter_map(|(index, (_, height))| height.map(|height| (index, tidy(height))))
            .collect();
        if found.is_empty() {
            self.tell(tr("Couldn't download the terrain data. Check the internet connection."));
            return;
        }
        if all && found.iter().all(|&(_, height)| height == 0.0) {
            self.tell(tr("The terrain data puts the whole track at sea level."));
            return;
        }
        self.push_undo();
        if all {
            for point in &mut self.points {
                point.ele = None;
            }
        }
        for &(index, height) in &found {
            self.points[index].ele = Some(height);
        }
        let all_raw = !has_heights(&self.raw);
        for (point, height) in self.raw.iter_mut().zip(raw) {
            if all_raw || point.ele.is_none() {
                point.ele = height.map(tidy);
            }
        }
        self.terrain_heights = true;
        let added = if found.len() == 1 { "Added a height to {} point." } else { "Added heights to {} points." };
        let mut text = trf(added, &[&grouped(found.len())]);
        let missing = self.points.iter().filter(|point| point.ele.is_none()).count();
        if missing > 0 {
            let still = if missing == 1 { "{} point still has no height." } else { "{} points still have no height." };
            text = format!("{text} {}", trf(still, &[&grouped(missing)]));
        }
        self.tell(text);
    }

    fn pump(&mut self) {
        // A thread that ended without an answer stopped with a panic.
        let opened = match self.opening.as_ref().map(Receiver::try_recv) {
            Some(Ok(opened)) => Some(opened),
            Some(Err(TryRecvError::Disconnected)) => Some(Err(tr("Reading the track stopped unexpectedly.").to_string())),
            _ => None,
        };
        if let Some(opened) = opened {
            self.opening = None;
            let restored = self.restored.take();
            match opened {
                Ok(Opened { loaded, points, settings }) => {
                    let mut note = if restored.is_some() {
                        tr("Restored the unsaved edits.").to_string()
                    } else if loaded.cleaned {
                        tr("Opened as saved, without a new cleanup.").to_string()
                    } else {
                        trf("Cleaned up from {} points to {}.", &[&loaded.points.len(), &points.len()])
                    };
                    if loaded.untimed > 0 {
                        let left = if loaded.untimed == 1 { "Left out {} point without a time." } else { "Left out {} points without a time." };
                        note = format!("{note} {}", trf(left, &[&grouped(loaded.untimed)]));
                    }
                    self.tell(note);
                    self.name = loaded.name;
                    self.format = loaded.format;
                    self.settings_from_file = loaded.settings.is_some();
                    self.settings = settings;
                    self.points_settings = settings;
                    self.raw = loaded.points;
                    self.points = points;
                    if let Some(restored) = restored {
                        self.take_restored(restored);
                    }
                    self.revision += 1;
                    self.framed = false;
                }
                Err(err) => match restored {
                    // The edits are kept even when the track itself is gone.
                    Some(restored) => {
                        self.raw = restored.points.clone();
                        self.take_restored(restored);
                        self.tell(tr("Restored the unsaved edits."));
                        self.revision += 1;
                        self.framed = false;
                    }
                    None => self.error = Some(err),
                },
            }
        }
        let cleaned = match self.cleaning.as_ref().map(Receiver::try_recv) {
            Some(Ok(points)) => Some(Some(points)),
            Some(Err(TryRecvError::Disconnected)) => Some(None),
            _ => None,
        };
        match cleaned {
            Some(Some(points)) if points.len() >= 2 => {
                self.cleaning = None;
                self.tell(trf("Cleaned up from {} points to {}.", &[&self.raw.len(), &points.len()]));
                self.push_undo();
                self.points = points;
                // The panel can't change while the cleanup runs.
                self.points_settings = self.settings;
                self.forget_picks();
            }
            Some(Some(_)) => {
                self.cleaning = None;
                self.tell(tr("The cleanup left fewer than two points. Try other settings."));
            }
            Some(None) => {
                self.cleaning = None;
                self.tell(tr("The cleanup stopped unexpectedly."));
            }
            None => {}
        }
        let heights = match self.looking_up.as_ref().map(|(receiver, _)| receiver.try_recv()) {
            Some(Ok(heights)) => Some(heights),
            Some(Err(TryRecvError::Disconnected)) => Some(Vec::new()),
            _ => None,
        };
        if let Some(heights) = heights {
            self.looking_up = None;
            self.take_heights(&heights);
        }
    }

    /// Puts the edits kept when Picasso quit in place of the points read.
    fn take_restored(&mut self, restored: Loaded) {
        self.name = restored.name;
        self.settings = restored.settings.unwrap_or(self.settings);
        self.points_settings = self.settings;
        self.settings_from_file = false;
        self.points = restored.points;
        self.dirty = true;
    }

    /// Keeps the points before an edit for Undo. Every change to `points`
    /// comes after this, or after Undo, Redo, or opening the file.
    fn push_undo(&mut self) {
        self.undo.push((self.points.clone(), self.points_settings));
        self.redo.clear();
        let mut kept: usize = self.undo.iter().map(|(points, _)| points.len()).sum();
        while self.undo.len() > 1 && (self.undo.len() > HISTORY || kept > HISTORY_POINTS) {
            kept -= self.undo.remove(0).0.len();
        }
        self.dirty = true;
        self.revision += 1;
    }

    fn undo(&mut self) {
        if let Some(earlier) = self.undo.pop() {
            let now = self.swap_points(earlier);
            self.redo.push(now);
            self.tell(tr("Undo"));
        }
    }

    fn redo(&mut self) {
        if let Some(later) = self.redo.pop() {
            let now = self.swap_points(later);
            self.undo.push(now);
            self.tell(tr("Redo"));
        }
    }

    /// Puts `(points, settings)` from Undo or Redo in place, and returns the
    /// ones they replace. Going back over a cleanup puts its settings back
    /// in the panel; other steps leave changes there not applied yet.
    fn swap_points(&mut self, (points, settings): (Vec<EditPoint>, Settings)) -> (Vec<EditPoint>, Settings) {
        let now = (std::mem::replace(&mut self.points, points), self.points_settings);
        if settings != self.points_settings {
            self.settings = settings;
        }
        self.points_settings = settings;
        self.dirty = true;
        self.revision += 1;
        self.forget_picks();
        now
    }

    /// Forgets the picked points after the points were replaced as a whole,
    /// since their numbers now stand for other points.
    fn forget_picks(&mut self) {
        self.selection.clear();
        self.focus = None;
        self.dragging = None;
        self.menu_point = None;
    }

    fn measures(&mut self) -> Rc<Measures> {
        match &self.measures {
            Some(measures) if measures.revision == self.revision && measures.settings == self.settings => Rc::clone(measures),
            _ => {
                let measures = Rc::new(Measures::of(&self.points, self.settings, self.revision));
                self.measures = Some(Rc::clone(&measures));
                measures
            }
        }
    }

    fn remove(&mut self, indices: HashSet<usize>) {
        let count = indices.len();
        let mut points = self.points.clone();
        if !track_edit::remove_points(&mut points, &indices) {
            self.tell(tr("A track keeps at least two points."));
            return;
        }
        self.push_undo();
        self.points = points;
        self.selection.clear();
        self.focus = None;
        self.menu_point = None;
        self.tell(trf(if count == 1 { "Removed {} point" } else { "Removed {} points" }, &[&count]));
    }

    /// Cleans the points as read up again with the current settings, on a
    /// thread. Undo brings back the points before.
    fn clean_up(&mut self, ctx: &egui::Context) {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (raw, settings, ctx) = (self.raw.clone(), self.settings, ctx.clone());
        let spawned = std::thread::Builder::new().name("picasso-track-cleanup".into()).spawn(move || {
            let _ = sender.send(track_edit::optimize(&raw, &settings));
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.cleaning = Some(receiver);
        }
    }

    /// The point `index`, where it is being dragged to while it is.
    fn point(&self, index: usize) -> EditPoint {
        let mut point = self.points[index];
        if let Some((dragged, lat, lon)) = self.dragging {
            if dragged == index {
                point.lat = lat;
                point.lon = lon;
            }
        }
        point
    }

    fn to_screen(&self, rect: Rect, lat: f64, lon: f64) -> Pos2 {
        let (cx, cy) = geo::latlon_to_world(self.view.lat, self.view.lon, self.view.zoom);
        let (x, y) = geo::latlon_to_world(lat, lon, self.view.zoom);
        rect.center() + self.view.pan + Vec2::new((x - cx) as f32, (y - cy) as f32)
    }

    fn to_latlon(&self, rect: Rect, pos: Pos2) -> (f64, f64) {
        let (cx, cy) = geo::latlon_to_world(self.view.lat, self.view.lon, self.view.zoom);
        let offset = pos - rect.center() - self.view.pan;
        geo::world_to_latlon(cx + f64::from(offset.x), cy + f64::from(offset.y), self.view.zoom)
    }

    fn bake_pan(&mut self) {
        let (cx, cy) = geo::latlon_to_world(self.view.lat, self.view.lon, self.view.zoom);
        let (lat, lon) = geo::world_to_latlon(cx - f64::from(self.view.pan.x), cy - f64::from(self.view.pan.y), self.view.zoom);
        self.view = MapView { lat, lon, zoom: self.view.zoom, pan: Vec2::ZERO };
    }

    /// Zooms by `steps` levels, keeping the place under `pointer` there.
    /// Returns whether the zoom changed.
    fn zoom_at(&mut self, steps: i32, pointer: Pos2, rect: Rect) -> bool {
        let next = (i32::from(self.view.zoom) + steps).clamp(i32::from(MIN_ZOOM), i32::from(MAX_ZOOM)) as u8;
        if next == self.view.zoom {
            return false;
        }
        let (lat, lon) = self.to_latlon(rect, pointer);
        let (x, y) = geo::latlon_to_world(lat, lon, next);
        let offset = pointer - rect.center();
        let (lat, lon) = geo::world_to_latlon(x - f64::from(offset.x), y - f64::from(offset.y), next);
        self.view = MapView { lat, lon, zoom: next, pan: Vec2::ZERO };
        true
    }

    /// Centers the map on `points` at the closest zoom that shows them all
    /// in `size`.
    fn fit(&mut self, points: &[EditPoint], size: Vec2) {
        let Some(first) = points.first() else { return };
        let world = |point: &EditPoint| geo::latlon_to_world(point.lat, point.lon, 0);
        let (mut min, mut max) = (world(first), world(first));
        for (x, y) in points.iter().map(world) {
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        let room = size * 0.85;
        let mut zoom = MAX_ZOOM.min(17);
        while zoom > MIN_ZOOM
            && ((max.0 - min.0) * f64::from(1u32 << zoom) > f64::from(room.x) || (max.1 - min.1) * f64::from(1u32 << zoom) > f64::from(room.y))
        {
            zoom -= 1;
        }
        let (lat, lon) = geo::world_to_latlon((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0, 0);
        self.view = MapView { lat, lon, zoom, pan: Vec2::ZERO };
    }

    fn go_to(&mut self, preset: Preset, size: Vec2) {
        let close = |editor: &mut TrackEditor, point: Option<EditPoint>| {
            if let Some(point) = point {
                editor.view = MapView { lat: point.lat, lon: point.lon, zoom: CLOSE_ZOOM, pan: Vec2::ZERO };
            }
        };
        match preset {
            Preset::Full => {
                let points = self.points.clone();
                self.fit(&points, size);
            }
            Preset::Start => close(self, self.points.first().copied()),
            Preset::End => close(self, self.points.last().copied()),
            Preset::Highest => {
                let top = if self.measures().heights.is_some() {
                    self.points.iter().filter(|point| point.ele.is_some()).max_by(|a, b| a.ele.partial_cmp(&b.ele).unwrap_or(std::cmp::Ordering::Equal))
                } else {
                    None
                };
                match top.copied() {
                    Some(point) => close(self, Some(point)),
                    None => self.tell(tr("The track has no heights.")),
                }
            }
        }
    }

    /// Writes the points as `format`, picked in a save dialog that starts
    /// beside the track. The track itself is never written over, so the
    /// points as read stay for the next cleanup.
    fn save(&mut self, format: Format) -> Option<PathBuf> {
        let (label, extension) = if format == Format::Kml { ("KML", "kml") } else { ("GPX", "gpx") };
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr("Save the track"))
            .set_file_name(track_edit::saved_name(&self.path, extension))
            .add_filter(label, &[extension]);
        if let Some(dir) = self.path.parent() {
            dialog = dialog.set_directory(dir);
        }
        let target = dialog.save_file()?;
        if same_file(&target, &self.path) {
            self.tell(tr("Save under another name; the original stays as it is."));
            return None;
        }
        let text = if format == Format::Kml {
            track_edit::to_kml(&self.points, &self.name, &self.points_settings)
        } else {
            track_edit::to_gpx(&self.points, &self.name, &self.points_settings)
        };
        match crate::image_io::write_whole(&target, text.as_bytes()) {
            Ok(()) => {
                self.dirty = false;
                self.tell(trf("Saved {}", &[&folder_label(&target.to_string_lossy())]));
                Some(target)
            }
            Err(err) => {
                self.tell(trf("Couldn't save the track: {}", &[&err]));
                None
            }
        }
    }

    /// The format Save writes without asking: the track's own, or GPX for
    /// a Polarsteps file.
    fn own_format(&self) -> Format {
        if self.format == Format::Kml { Format::Kml } else { Format::Gpx }
    }
}

/// Whether `a` and `b` name one file, through links and, on a disk that
/// ignores case, names that differ only in case.
fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        matches!((std::fs::metadata(a), std::fs::metadata(b)), (Ok(a), Ok(b)) if a.dev() == b.dev() && a.ino() == b.ino())
    }
    #[cfg(not(unix))]
    {
        matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
    }
}

/// Unsaved edits of a track, kept in [`RECOVERY_FILE`] when Picasso quit.
pub(super) struct Recovery {
    path: PathBuf,
    loaded: Loaded,
}

/// The unsaved edits of a track, as [`RECOVERY_FILE`] keeps them.
struct Kept {
    path: PathBuf,
    name: String,
    points: Vec<EditPoint>,
    settings: Settings,
}

impl Kept {
    fn of(editor: &TrackEditor) -> Kept {
        Kept { path: editor.path.clone(), name: editor.name.clone(), points: editor.points.clone(), settings: editor.points_settings }
    }
}

/// What [`RECOVERY_FILE`] holds: the track's path, and its points as GPX.
fn recovery_text(kept: &Kept) -> String {
    let gpx = track_edit::to_gpx(&kept.points, &kept.name, &kept.settings);
    serde_json::json!({ "path": kept.path.to_string_lossy(), "gpx": gpx }).to_string()
}

enum KeepJob {
    Keep(Kept),
    Forget,
    Flush(Sender<()>),
}

/// Writes and removes [`RECOVERY_FILE`] on a thread of its own, so a long
/// track doesn't hold the window up. The changes go one after another, so
/// a removal after a save can't be undone by a write still on its way.
pub(super) struct Keeper {
    jobs: Sender<KeepJob>,
}

impl Keeper {
    pub(super) fn new(data_dir: &Path) -> Keeper {
        let (jobs, inbox) = crossbeam_channel::unbounded::<KeepJob>();
        let file = data_dir.join(RECOVERY_FILE);
        let _ = std::thread::Builder::new().name("picasso-track-keeper".into()).spawn(move || {
            while let Ok(job) = inbox.recv() {
                // Of the changes waiting, only the last one counts.
                let (mut change, mut flushed) = (None, Vec::new());
                for job in std::iter::once(job).chain(inbox.try_iter()) {
                    match job {
                        KeepJob::Flush(done) => flushed.push(done),
                        other => change = Some(other),
                    }
                }
                match change {
                    Some(KeepJob::Keep(kept)) => {
                        if let Err(err) = crate::image_io::write_whole(&file, recovery_text(&kept).as_bytes()) {
                            eprintln!("picasso track: {err}");
                        }
                    }
                    Some(KeepJob::Forget) => {
                        let _ = std::fs::remove_file(&file);
                    }
                    _ => {}
                }
                for done in flushed {
                    let _ = done.send(());
                }
            }
        });
        Keeper { jobs }
    }

    fn keep(&self, editor: &TrackEditor) {
        let _ = self.jobs.send(KeepJob::Keep(Kept::of(editor)));
    }

    fn forget(&self) {
        let _ = self.jobs.send(KeepJob::Forget);
    }

    /// Waits, up to a few seconds, for the changes sent to reach the disk.
    fn flush(&self) {
        let (done, wait) = crossbeam_channel::bounded(1);
        if self.jobs.send(KeepJob::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(std::time::Duration::from_secs(10));
        }
    }
}

/// The edits kept when Picasso last quit, if any.
pub(super) fn read_recovery(data_dir: &Path) -> Option<Recovery> {
    let text = std::fs::read_to_string(data_dir.join(RECOVERY_FILE)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let path = PathBuf::from(value.get("path")?.as_str()?);
    let loaded = track_edit::parse_gpx(value.get("gpx")?.as_str()?, Settings::default()).ok()?;
    Some(Recovery { path, loaded })
}

impl PicassoApp {
    /// Opens the track file at `path` in the editor.
    pub(super) fn open_track(&mut self, path: &str, ctx: &egui::Context) {
        let base = Settings::load(&self.config);
        self.track_editor = Some(TrackEditor::new(PathBuf::from(path), base, ctx));
    }

    /// Keeps the open track's unsaved edits for the next start, and waits
    /// for the changes on their way to reach the disk. Closing the window
    /// asks first, but Cmd+Q on a Mac quits without asking.
    pub(super) fn keep_track_edits(&self) {
        if let Some(editor) = self.track_editor.as_ref().filter(|editor| editor.dirty && editor.points.len() >= 2) {
            self.track_keeper.keep(editor);
        }
        self.track_keeper.flush();
    }

    /// Keeps the unsaved edits once they have rested [`KEEP_AFTER`]
    /// seconds, so a crash doesn't lose them either, and forgets them once
    /// they are saved.
    fn keep_track_edits_soon(&self, editor: &mut TrackEditor, ctx: &egui::Context) {
        if !editor.dirty {
            self.forget_track_edits(editor);
            return;
        }
        if editor.kept_revision == Some(editor.revision) || editor.points.len() < 2 {
            return;
        }
        let now = ctx.input(|input| input.time);
        match editor.changed {
            Some((revision, at)) if revision == editor.revision && now - at >= KEEP_AFTER => {
                self.track_keeper.keep(editor);
                editor.kept_revision = Some(editor.revision);
                editor.changed = None;
            }
            Some((revision, at)) if revision == editor.revision => {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(KEEP_AFTER - (now - at)));
            }
            _ => {
                editor.changed = Some((editor.revision, now));
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(KEEP_AFTER));
            }
        }
    }

    /// Removes the edits this editor kept, once saved or let go.
    fn forget_track_edits(&self, editor: &mut TrackEditor) {
        editor.changed = None;
        if editor.kept_revision.take().is_some() {
            self.track_keeper.forget();
        }
    }

    /// Asks whether to restore the track edits kept when Picasso quit.
    pub(super) fn track_recovery_ui(&mut self, ctx: &egui::Context) {
        let Some(recovery) = &self.track_recovery else { return };
        let name = folder_label(&recovery.path.to_string_lossy());
        let mut choice = None;
        // Esc and clicks beside it don't close it: until it is answered,
        // another track's edits would be kept in place of these.
        egui::Modal::new(egui::Id::unique("track-recovery")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(RichText::new(tr("Restore the unsaved track edits?")).size(16.0).strong());
            ui.add_space(6.0);
            ui.label(RichText::new(trf("Picasso quit before the edits to {} were saved.", &[&name])).color(theme::muted()));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button(tr("Restore"))).clicked() {
                    choice = Some(true);
                }
                if ui.add(theme::danger_button(tr("Discard"))).clicked() {
                    choice = Some(false);
                }
            });
        });
        match (choice, self.track_recovery.take()) {
            (Some(restore), Some(recovery)) => {
                self.track_keeper.forget();
                if restore {
                    let mut editor = TrackEditor::new(recovery.path, Settings::load(&self.config), ctx);
                    editor.restored = Some(recovery.loaded);
                    self.track_editor = Some(editor);
                }
            }
            (_, recovery) => self.track_recovery = recovery,
        }
    }

    pub(super) fn track_editor_ui(&mut self, ui: &mut egui::Ui) {
        let Some(mut editor) = self.track_editor.take() else { return };
        editor.pump();
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut close = false;
        track_header(ui, &mut editor, &mut close);
        let full = ui.available_rect_before_wrap();
        ui.painter().hline(full.x_range(), full.top(), Stroke::new(1.0, theme::hairline()));
        let ready = editor.error.is_none() && editor.opening.is_none();
        let map_rect = Rect::from_min_max(full.min, Pos2::new(full.max.x - PANEL_W, full.max.y));
        if let Some(err) = editor.error.clone() {
            close |= track_message(ui, full, &trf("Couldn't open the track: {}", &[&err]), false);
        } else if !ready {
            track_message(ui, full, tr("Loading…"), true);
        } else {
            if !editor.framed {
                let points = editor.points.clone();
                editor.fit(&points, map_rect.size());
                editor.framed = true;
            }
            self.track_map(ui, &mut editor, map_rect);
            self.track_overlays(ui, &mut editor, map_rect);
            let panel_rect = Rect::from_min_max(Pos2::new(map_rect.max.x, full.min.y), full.max);
            track_panel(ui, panel_rect, &mut editor, &mut self.config, &self.cache_dir);
        }
        ui.advance_cursor_after_rect(full);
        if ready {
            track_keys(ui.ctx(), &mut editor, map_rect.size(), &mut close);
        } else if ui.ctx().input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }
        if editor.dirty && ui.ctx().input(|input| input.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            editor.asking_close = true;
            editor.quit_after_discard = true;
        }
        if close {
            if editor.dirty {
                editor.asking_close = true;
            } else {
                self.forget_track_edits(&mut editor);
                return;
            }
        }
        if editor.asking_close && ask_discard(ui.ctx(), &mut editor) {
            self.forget_track_edits(&mut editor);
            if editor.quit_after_discard {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        self.keep_track_edits_soon(&mut editor, ui.ctx());
        self.track_editor = Some(editor);
    }

    fn track_map(&mut self, ui: &mut egui::Ui, editor: &mut TrackEditor, rect: Rect) {
        let response = ui.interact(rect, egui::Id::unique("track-map"), Sense::click_and_drag());
        let pointer = response.hover_pos();
        if response.hovered() {
            let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
            let steps = wheel_steps(&mut editor.wheel, scroll) + pinch_steps(&mut editor.pinch, pinch);
            if let Some(pointer) = pointer.filter(|_| steps != 0) {
                if editor.zoom_at(steps, pointer, rect) {
                    self.tiles.clear();
                }
            }
        }

        let screen: Vec<Pos2> = (0..editor.points.len()).map(|index| {
            let point = editor.point(index);
            editor.to_screen(rect, point.lat, point.lon)
        }).collect();
        let in_view: Vec<usize> = (0..screen.len()).filter(|index| rect.expand(POINT_REACH).contains(screen[*index])).collect();
        let near_point = |pos: Pos2| {
            in_view
                .iter()
                .map(|index| (*index, screen[*index].distance(pos)))
                .filter(|(_, distance)| *distance <= POINT_REACH)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(index, _)| index)
        };
        let near_line = |pos: Pos2| {
            screen
                .windows(2)
                .enumerate()
                .filter(|(_, pair)| Rect::from_two_pos(pair[0], pair[1]).expand(LINE_REACH).contains(pos))
                .map(|(index, pair)| (index, distance_to_segment(pos, pair[0], pair[1])))
                .filter(|(_, distance)| *distance <= LINE_REACH)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(index, _)| index)
        };
        // A cleanup on its way replaces the points, so they aren't edited
        // meanwhile.
        let locked = editor.busy();
        let hovered_point = pointer.filter(|_| editor.dragging.is_none()).and_then(near_point);
        let hovered_line = if hovered_point.is_none() && !editor.box_mode && !locked { pointer.and_then(near_line) } else { None };
        editor.map_hover = hovered_point;

        if response.drag_started() {
            editor.drag_cancelled = false;
            let origin = ui.input(|input| input.pointer.press_origin()).unwrap_or_default();
            if editor.box_mode {
                editor.box_from = Some(origin);
            } else if let Some(index) = near_point(origin).filter(|_| !locked) {
                let point = editor.points[index];
                editor.dragging = Some((index, point.lat, point.lon));
                editor.focus = Some(index);
            }
        }
        if response.dragged() {
            if let Some((index, _, _)) = editor.dragging {
                if let Some(pos) = response.interact_pointer_pos() {
                    let (lat, lon) = editor.to_latlon(rect, pos);
                    editor.dragging = Some((index, lat, lon));
                }
            } else if editor.box_from.is_none() && !editor.drag_cancelled {
                editor.view.pan += response.drag_delta();
            }
        }
        if response.drag_stopped() {
            editor.drag_cancelled = false;
            if let Some((index, lat, lon)) = editor.dragging.take() {
                editor.push_undo();
                editor.points[index].lat = lat;
                editor.points[index].lon = lon;
                editor.tell(trf("Moved point {}", &[&(index + 1)]));
            } else if let Some(from) = editor.box_from.take() {
                let area = Rect::from_two_pos(from, response.interact_pointer_pos().unwrap_or(from));
                let picked: Vec<usize> = in_view.iter().copied().filter(|index| area.contains(screen[*index])).collect();
                if !ui.input(|input| input.modifiers.shift) {
                    editor.selection.clear();
                }
                editor.selection.extend(picked);
            } else {
                editor.bake_pan();
            }
        }
        if response.clicked() {
            let modifiers = ui.input(|input| input.modifiers);
            if let Some(index) = hovered_point {
                if modifiers.command || modifiers.shift {
                    if !editor.selection.remove(&index) {
                        editor.selection.insert(index);
                    }
                } else {
                    editor.selection = HashSet::from([index]);
                }
                editor.focus = Some(index);
            } else if let (Some(segment), Some(pos)) = (hovered_line, pointer) {
                let (a, b) = (screen[segment], screen[segment + 1]);
                let (lat, lon) = editor.to_latlon(rect, closest_on_segment(pos, a, b));
                editor.push_undo();
                let index = track_edit::insert_point(&mut editor.points, segment, lat, lon);
                editor.selection = HashSet::from([index]);
                editor.focus = Some(index);
                editor.tell(trf("Added point {}", &[&(index + 1)]));
            } else {
                editor.selection.clear();
                editor.focus = None;
            }
        }
        if response.secondary_clicked() {
            editor.menu_point = hovered_point.filter(|_| !locked);
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, MAP_BACKDROP);
        self.paint_tiles(&painter, rect, editor.view);
        if editor.show_raw {
            let raw: Vec<Pos2> = editor.raw.iter().map(|point| editor.to_screen(rect, point.lat, point.lon)).collect();
            painter.add(egui::Shape::line(raw, Stroke::new(2.0, Color32::from_rgba_unmultiplied(80, 80, 80, 150))));
        }
        let measures = editor.measures();
        let flights = &measures.flights;
        let shown = |index: usize, pair: &[Pos2]| !flights[index] && Rect::from_two_pos(pair[0], pair[1]).intersects(rect);
        for (index, pair) in screen.windows(2).enumerate() {
            if flights[index] && Rect::from_two_pos(pair[0], pair[1]).intersects(rect) {
                painter.add(egui::Shape::dashed_line(&[pair[0], pair[1]], Stroke::new(2.5, FLIGHT), 8.0, 6.0));
                paint_plane(&painter, pair[0].lerp(pair[1], 0.5), pair[1] - pair[0]);
            } else if shown(index, pair) {
                painter.line_segment([pair[0], pair[1]], Stroke::new(7.0, Color32::from_white_alpha(225)));
            }
        }
        for (index, pair) in screen.windows(2).enumerate() {
            if shown(index, pair) {
                let lit = hovered_line == Some(index);
                painter.line_segment([pair[0], pair[1]], Stroke::new(if lit { 5.5 } else { 3.5 }, LINE));
            }
        }
        if let (Some(segment), Some(pos)) = (hovered_line, pointer) {
            let at = closest_on_segment(pos, screen[segment], screen[segment + 1]);
            painter.circle(at, 7.0, WHITE, Stroke::new(2.0, LINE));
            painter.hline((at.x - 3.0)..=(at.x + 3.0), at.y, Stroke::new(1.6, LINE));
            painter.vline(at.x, (at.y - 3.0)..=(at.y + 3.0), Stroke::new(1.6, LINE));
        }
        let every = in_view.len() <= MAX_DRAWN_POINTS;
        let mut last_handle: Option<Pos2> = None;
        for &index in &in_view {
            let point = editor.points[index];
            let pos = screen[index];
            if point.dwell_ms > 0 {
                let hours = (point.dwell_ms as f32 / 3_600_000.0).min(24.0);
                painter.circle(pos, 8.0 + 9.0 * hours / 24.0, STAY.gamma_multiply(0.4), Stroke::new(1.5, STAY));
            }
            let selected = editor.selection.contains(&index);
            let lit = hovered_point == Some(index) || editor.dragging.is_some_and(|(dragged, _, _)| dragged == index);
            if lit {
                painter.circle_filled(pos, 12.0, LINE.gamma_multiply(0.22));
            }
            // Handles closer together than this would bury the line.
            let spaced = last_handle.is_none_or(|last| last.distance(pos) >= HANDLE_GAP);
            if selected {
                painter.circle(pos, if lit { 6.5 } else { 5.5 }, ACCENT, Stroke::new(2.0, WHITE));
            } else if lit || (every && spaced) {
                painter.circle(pos, if lit { 6.0 } else { 3.0 }, WHITE, Stroke::new(1.5, LINE));
                last_handle = Some(pos);
            }
        }
        if let Some(pos) = editor.profile_hover.and_then(|index| screen.get(index)).filter(|pos| rect.contains(**pos)) {
            painter.circle_filled(*pos, 13.0, ACCENT.gamma_multiply(0.25));
            painter.circle(*pos, 6.5, WHITE, Stroke::new(3.0, ACCENT));
        }
        for (index, color) in [(screen.len().saturating_sub(1), DANGER), (0, START)] {
            if let Some(pos) = screen.get(index).filter(|pos| rect.contains(**pos)) {
                paint_marker(&painter, *pos, color);
            }
        }
        if let (Some(from), Some(to)) = (editor.box_from, response.interact_pointer_pos()) {
            let area = Rect::from_two_pos(from, to);
            painter.rect_filled(area, 4, ACCENT.gamma_multiply(0.12));
            painter.rect_stroke(area, 4, Stroke::new(1.5, ACCENT), egui::StrokeKind::Inside);
        }
        if let (Some(index), Some(pos)) = (hovered_point, pointer) {
            paint_point_card(&painter, rect, pos, index, &editor.points[index], measures.heights.is_some());
        }
        self.paint_map_notes(&painter, rect, editor.view);

        let cursor = if editor.dragging.is_some() {
            egui::CursorIcon::Grabbing
        } else if editor.box_mode || hovered_line.is_some() {
            egui::CursorIcon::Crosshair
        } else if hovered_point.is_some() {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Default
        };
        if response.hovered() || editor.dragging.is_some() {
            ui.ctx().set_cursor_icon(cursor);
        }
        response.context_menu(|ui| {
            if let Some(index) = editor.menu_point.filter(|index| *index < editor.points.len()) {
                if ui.button(tr("Remove point")).clicked() {
                    editor.remove(HashSet::from([index]));
                    ui.close();
                }
                ui.separator();
            }
            if ui.button(tr("Whole track")).clicked() {
                editor.go_to(Preset::Full, rect.size());
                ui.close();
            }
        });
    }

    /// The tools, the views, the zoom, the selection, and messages, floating
    /// over the map.
    fn track_overlays(&mut self, ui: &mut egui::Ui, editor: &mut TrackEditor, rect: Rect) {
        let ctx = ui.ctx().clone();
        floating(&ctx, "track-tools", rect.left_top() + Vec2::splat(INSET), Align2::LEFT_TOP, rect, |ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
            if tool_button(ui, Glyph::Move, !editor.box_mode, tr("Move and add points (V)")).clicked() {
                editor.box_mode = false;
            }
            if tool_button(ui, Glyph::Box, editor.box_mode, tr("Drag a box round points to select them (R)")).clicked() {
                editor.box_mode = true;
            }
            let (line, _) = ui.allocate_exact_size(Vec2::new(TOOL, 7.0), Sense::hover());
            ui.painter().hline(line.x_range().shrink(6.0), line.center().y, Stroke::new(1.0, theme::hairline()));
            let hint = format!("{}\n{}", tr("Show original"), tr("Draw the track as read, before the cleanup"));
            if tool_button(ui, Glyph::Original, editor.show_raw, &hint).clicked() {
                editor.show_raw = !editor.show_raw;
            }
        });

        let size = rect.size();
        floating(&ctx, "track-views", Pos2::new(rect.center().x, rect.top() + INSET), Align2::CENTER_TOP, rect, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (preset, label, key) in PRESETS {
                    let button = egui::Button::selectable(false, RichText::new(tr(label)).size(12.5)).min_size(Vec2::new(0.0, 28.0));
                    if ui.add(button).on_hover_text(format!("{} ({key})", tr(label))).clicked() {
                        editor.go_to(preset, size);
                    }
                }
            });
        });

        let step = zoom_step((rect.height() - 2.0 * INSET).min(240.0));
        let corner = Pos2::new(rect.right() - INSET - ZOOM_BUTTON, rect.top() + INSET);
        if let Some(next) = zoom_control(ui, corner, editor.view.zoom, step, "track-zoom") {
            if editor.zoom_at(i32::from(next) - i32::from(editor.view.zoom), rect.center(), rect) {
                self.tiles.clear();
            }
        }

        let bottom = Pos2::new(rect.center().x, rect.bottom() - 30.0);
        let mut raised = 0.0;
        let count = editor.selection.len();
        if count > 0 {
            let bar = floating(&ctx, "track-selection", bottom, Align2::CENTER_BOTTOM, rect, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let text = trf(if count == 1 { "{} point selected" } else { "{} points selected" }, &[&grouped(count)]);
                    ui.label(RichText::new(text).strong().color(theme::text()));
                    ui.add_space(10.0);
                    let remove = ui.add_enabled(!editor.busy(), theme::danger_button(tr("Remove")));
                    if remove.on_hover_text(tr("Remove the selected points (Delete)")).clicked() {
                        editor.remove(editor.selection.clone());
                    }
                    if ui.button(tr("Clear selection")).on_hover_text("Esc").clicked() {
                        editor.selection.clear();
                        editor.focus = None;
                    }
                });
            });
            raised = bar.response.rect.height() + 10.0;
        } else if editor.box_mode {
            let hint = floating(&ctx, "track-box-hint", bottom, Align2::CENTER_BOTTOM, rect, |ui| {
                egui::Frame::new().inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.label(RichText::new(tr("Drag a box round points to select them (R)")).color(theme::muted()));
                });
            });
            raised = hint.response.rect.height() + 10.0;
        }

        if editor.note.is_empty() {
            return;
        }
        let now = ui.input(|input| input.time);
        if editor.note_at.is_nan() {
            editor.note_at = now;
        }
        let age = now - editor.note_at;
        if age >= TOAST_SECONDS {
            return;
        }
        let presence = (age / TOAST_FADE).min((TOAST_SECONDS - age) / TOAST_FADE).min(1.0) as f32;
        egui::Area::new(egui::Id::unique("track-toast"))
            .order(egui::Order::Middle)
            .interactable(false)
            .fixed_pos(bottom - Vec2::new(0.0, raised))
            .pivot(Align2::CENTER_BOTTOM)
            .constrain_to(rect)
            .show(&ctx, |ui| {
                ui.set_opacity(presence);
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(0x1F, 0x21, 0x25, 235))
                    .corner_radius(16)
                    .inner_margin(Margin::symmetric(14, 7))
                    .shadow(theme::card_shadow())
                    .show(ui, |ui| ui.label(RichText::new(&editor.note).size(12.5).color(WHITE)));
            });
        let wait = if presence < 1.0 { 0.016 } else { TOAST_SECONDS - TOAST_FADE - age };
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.016)));
    }
}

/// Back, the track's name and folder, Undo and Redo, and Save.
fn track_header(ui: &mut egui::Ui, editor: &mut TrackEditor, close: &mut bool) {
    theme::toolbar_frame(theme::toolbar()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let back = egui::Button::new(RichText::new(tr("‹  Back")).color(theme::text())).frame_when_inactive(false);
            if ui.add(back).on_hover_text(tr("Close the track editor (Esc)")).clicked() {
                *close = true;
            }
            ui.add(egui::Separator::default().vertical().spacing(14.0));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(&editor.name).size(15.0).strong().color(theme::text()));
                    badge(ui, &format_badge(&editor.path.to_string_lossy()));
                    if editor.dirty {
                        let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.5, ACCENT);
                    }
                });
                let folder = editor.path.parent().map(|dir| folder_label(&dir.to_string_lossy())).unwrap_or_default();
                ui.label(RichText::new(folder).size(11.5).color(theme::muted()));
            });
            if editor.busy() {
                ui.add(egui::Spinner::new().size(14.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ready = !editor.busy() && editor.error.is_none();
                ui.add_enabled_ui(ready, |ui| {
                    save_buttons(ui, editor);
                    ui.add_space(10.0);
                    let redo = theme::icon_button(ui, theme::Icon::Redo, !editor.redo.is_empty(), false);
                    if redo.on_hover_text(tr("Redo (Ctrl+Y)")).clicked() {
                        editor.redo();
                    }
                    let undo = theme::icon_button(ui, theme::Icon::Undo, !editor.undo.is_empty(), false);
                    if undo.on_hover_text(tr("Undo (Ctrl+Z)")).clicked() {
                        editor.undo();
                    }
                });
            });
        });
    });
}

/// Save in the track's own format, with the other format under the arrow
/// beside it.
fn save_buttons(ui: &mut egui::Ui, editor: &mut TrackEditor) {
    let label = |format: Format| if format == Format::Kml { tr("Save as KML…") } else { tr("Save as GPX…") };
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        ui.spacing_mut().button_padding.x = 7.0;
        let more = ui.add(theme::primary_button(" "));
        theme_glyph(ui.painter(), Glyph::Chevron, more.rect.center(), WHITE);
        egui::Popup::menu(&more).show(|ui| {
            for format in [Format::Gpx, Format::Kml] {
                if ui.button(label(format)).clicked() {
                    editor.save(format);
                }
            }
        });
        ui.spacing_mut().button_padding.x = 12.0;
        let own = editor.own_format();
        if ui.add(theme::primary_button(label(own))).on_hover_text(tr("Save a copy beside the track (Ctrl+S)")).clicked() {
            editor.save(own);
        }
    });
}

/// A spinner or an error in the middle of `rect`. Returns true when Back
/// is clicked.
fn track_message(ui: &mut egui::Ui, rect: Rect, text: &str, loading: bool) -> bool {
    let area = Rect::from_center_size(rect.center(), Vec2::new(rect.width().min(420.0), 180.0));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
    ui.spacing_mut().item_spacing.y = 10.0;
    if loading {
        ui.add(egui::Spinner::new().size(28.0));
        ui.label(RichText::new(text).color(theme::muted()));
        return false;
    }
    ui.add(egui::Label::new(RichText::new(text).size(14.0).color(theme::text())).wrap());
    ui.button(tr("‹  Back")).clicked()
}

/// Undo, Redo, Select, Remove, the views, the points, Save, and Esc.
fn track_keys(ctx: &egui::Context, editor: &mut TrackEditor, size: Vec2, close: &mut bool) {
    let modal = ctx.memory(|memory| memory.top_modal_layer().is_some());
    if ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) || modal || editor.asking_close {
        return;
    }
    let command = |key| ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, key));
    let plain = |key| ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key));
    // Edits wait while a cleanup on its way would replace them.
    let busy = editor.busy();
    if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)) || command(Key::Y) {
        if !busy {
            editor.redo();
        }
    } else if command(Key::Z) {
        if !busy {
            editor.undo();
        }
    } else if command(Key::S) {
        if !busy {
            editor.save(editor.own_format());
        }
    } else if plain(Key::Delete) || plain(Key::Backspace) {
        if !busy && !editor.selection.is_empty() {
            editor.remove(editor.selection.clone());
        }
    } else if plain(Key::R) {
        editor.box_mode = !editor.box_mode;
    } else if plain(Key::V) {
        editor.box_mode = false;
    } else if plain(Key::ArrowLeft) {
        editor.step(-1);
    } else if plain(Key::ArrowRight) {
        editor.step(1);
    } else if plain(Key::Escape) {
        if editor.box_from.is_some() || editor.dragging.is_some() {
            editor.box_from = None;
            editor.dragging = None;
            editor.drag_cancelled = true;
        } else if !editor.selection.is_empty() {
            editor.selection.clear();
            editor.focus = None;
        } else if editor.box_mode {
            editor.box_mode = false;
        } else {
            *close = true;
        }
    } else {
        for (key, (preset, _, _)) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4].into_iter().zip(PRESETS) {
            if plain(key) {
                editor.go_to(preset, size);
            }
        }
    }
}

/// Asks before closing a track with edits that weren't saved. Returns true
/// to close it.
fn ask_discard(ctx: &egui::Context, editor: &mut TrackEditor) -> bool {
    let mut discard = false;
    let response = egui::Modal::new(egui::Id::unique("track-discard")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new(tr("Discard the changes to this track?")).size(16.0).strong());
        ui.add_space(6.0);
        ui.label(RichText::new(tr("The file stays as it is. Save a copy first to keep your edits.")).color(theme::muted()));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add(theme::danger_button(tr("Discard"))).clicked() {
                discard = true;
            }
            if ui.button(tr("Keep editing")).clicked() {
                editor.asking_close = false;
            }
        });
    });
    if response.should_close() {
        editor.asking_close = false;
    }
    if !editor.asking_close {
        editor.quit_after_discard = false;
    }
    discard
}

/// The inspector beside the map: statistics, the elevation profile, the
/// focused point, and the cleanup settings.
fn track_panel(ui: &mut egui::Ui, rect: Rect, editor: &mut TrackEditor, config: &mut crate::config::Config, cache: &Path) {
    ui.painter().rect_filled(rect, 0, theme::sidebar());
    ui.painter().vline(rect.left(), rect.y_range(), Stroke::new(1.0, theme::hairline()));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    egui::ScrollArea::vertical().id_salt("track-panel").auto_shrink([false, false]).show(&mut ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 6, bottom: 18 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let measures = editor.measures();
            let stats = measures.stats;
            section(ui, tr("Statistics"), |_| {});
            let mut tiles = vec![(tr("Distance").to_string(), format_distance(stats.meters), None)];
            if let Some(ms) = stats.duration_ms {
                tiles.push((tr("Elapsed time").to_string(), format_duration(ms as f64 / 1000.0), None));
            }
            if let Some(kmh) = stats.average_kmh {
                tiles.push((tr("Average speed").to_string(), format!("{kmh:.1} km/h"), None));
            }
            tiles.push((tr("Top speed").to_string(), format!("{:.1} km/h", stats.top_kmh), None));
            let read = trf("{} of {} read", &[&grouped(editor.points.len()), &grouped(editor.raw.len())]);
            tiles.push((tr("Points").to_string(), grouped(editor.points.len()), Some(read)));
            tiles.push((trf("Stays over {} h", &[&format_hours(editor.settings.dwell_hours)]), stats.stays.to_string(), None));
            stat_tiles(ui, &tiles);

            if let Some((low, high)) = measures.heights {
                section(ui, tr("Elevation"), |ui| {
                    ui.label(RichText::new(format!("{low:.0} – {high:.0} m")).size(11.5).color(theme::muted()));
                });
                let (hovered, picked) = elevation_profile(ui, &editor.points, &measures.along, (low, high), editor.focus, editor.map_hover);
                editor.profile_hover = hovered;
                if let Some(index) = picked {
                    editor.show_point(index);
                }
                if editor.terrain_heights {
                    note(ui, tr("Heights from Terrain Tiles on AWS: USGS, NASA SRTM, Copernicus EU-DEM, and others."));
                }
                if (editor.terrain_heights && measures.unknown > 0) || editor.looking_up.is_some() {
                    height_lookup(ui, editor, cache, false);
                }
            } else {
                editor.profile_hover = None;
                section(ui, tr("Elevation"), |_| {});
                panel_card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if editor.looking_up.is_none() {
                        note(ui, tr("The track has no heights."));
                        ui.add_space(4.0);
                    }
                    height_lookup(ui, editor, cache, true);
                });
            }

            point_section(ui, editor, measures.heights.is_some());
            cleanup_section(ui, editor, config);

            ui.add_space(8.0);
            egui::CollapsingHeader::new(RichText::new(tr("Editing")).strong().size(12.5).color(theme::text()))
                .id_salt("track-tips")
                .default_open(false)
                .show(ui, |ui| {
                    for tip in TIPS {
                        note(ui, tr(tip));
                    }
                });
        });
    });
}

fn point_section(ui: &mut egui::Ui, editor: &mut TrackEditor, heights: bool) {
    let count = editor.points.len();
    let Some(index) = editor.focus.filter(|index| *index < count) else {
        section(ui, tr("Point"), |_| {});
        panel_card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            note(ui, tr("Click a point to see its details."));
        });
        return;
    };
    let (mut step, mut remove) = (None, false);
    section(ui, tr("Point"), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        remove = ui.add_enabled_ui(!editor.busy(), |ui| glyph_button(ui, Glyph::Trash, tr("Remove point")).clicked()).inner;
        ui.add_space(6.0);
        if nav_button(ui, "›", index + 1 < count, tr("Next point (→)")) {
            step = Some(1);
        }
        ui.label(RichText::new(format!("{} / {}", grouped(index + 1), grouped(count))).size(11.5).color(theme::muted()));
        if nav_button(ui, "‹", index > 0, tr("Previous point (←)")) {
            step = Some(-1);
        }
    });
    let point = editor.points[index];
    panel_card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Grid::new("track-point").num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
            if let Some(ms) = point.time {
                row(ui, tr("Time"), format_timestamp(ms.div_euclid(1000)));
            }
            row(ui, tr("Latitude"), format!("{:.6}", point.lat));
            row(ui, tr("Longitude"), format!("{:.6}", point.lon));
            if let Some(ele) = point.ele.filter(|_| heights) {
                row(ui, tr("Elevation"), format!("{ele:.1} m"));
            }
            if index > 0 {
                let before = editor.points[index - 1];
                row(ui, tr("Speed"), format!("{:.1} km/h", track_edit::speed_kmh(&before, &point)));
                if track_edit::flights(&editor.points[index - 1..=index], &editor.settings)[0] {
                    row(ui, tr("Arrived by"), tr("Flight").to_string());
                }
            }
            if point.dwell_ms > 0 {
                row(ui, tr("Stay"), format_duration(point.dwell_ms as f64 / 1000.0));
            }
        });
    });
    if remove {
        editor.remove(HashSet::from([index]));
    } else if let Some(by) = step {
        editor.step(by);
    }
}

fn cleanup_section(ui: &mut egui::Ui, editor: &mut TrackEditor, config: &mut crate::config::Config) {
    let mut defaults = false;
    section(ui, tr("Cleanup"), |ui| {
        let link = RichText::new(tr("Defaults")).size(12.0).color(ui.visuals().hyperlink_color);
        defaults = ui.add(egui::Button::new(link).frame_when_inactive(false)).clicked();
    });
    if defaults {
        editor.settings = Settings::default();
        editor.settings_unsaved = true;
    }
    if editor.settings_from_file {
        ui.label(RichText::new(tr("These settings were saved in the file.")).size(12.0).color(ACCENT));
    }
    panel_card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        editor.settings_unsaved |= settings_form(ui, "track-settings", &mut editor.settings);
    });
    let enabled = !editor.busy() && !editor.raw.is_empty();
    let button = egui::Button::new(tr("Clean up again")).min_size(Vec2::new(ui.available_width(), 30.0));
    if ui.add_enabled(enabled, button).on_hover_text(tr("Clean the track as read up with these settings. Undo brings your edits back.")).clicked() {
        editor.clean_up(ui.ctx());
    }
    // Saved once a drag on a number ends rather than on every step.
    if editor.settings_unsaved && !ui.input(|input| input.pointer.any_down()) {
        editor.settings.save(config);
        editor.settings_unsaved = false;
    }
}

/// The button that looks the missing heights up, or how far the lookup is.
fn height_lookup(ui: &mut egui::Ui, editor: &mut TrackEditor, cache: &Path, primary: bool) {
    if let Some((_, progress)) = &editor.looking_up {
        let (done, total) = (progress.done.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed));
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(13.0));
            note(ui, &trf("Looking up heights… {} of {} tiles", &[&grouped(done), &grouped(total)]));
        });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        return;
    }
    let label = tr("Look up heights");
    let button = if primary { theme::primary_button(label) } else { egui::Button::new(label) };
    let hint = tr("Fills in each point's height from free terrain data. Only the numbers of the map tiles along the track are sent, as when showing the map; the points stay on this computer.");
    if ui.add_enabled(!editor.busy(), button).on_hover_text(hint).clicked() {
        editor.look_up_heights(cache, ui.ctx());
    }
}

/// The elevation along the track, with the point under the pointer here or
/// on the map and the focused one marked. `along` is the distance to each
/// point, and `range` the lowest and highest heights. Returns the point
/// under the pointer, and the one clicked or dragged to.
fn elevation_profile(
    ui: &mut egui::Ui,
    points: &[EditPoint],
    along: &[f64],
    range: (f64, f64),
    focus: Option<usize>,
    map_hover: Option<usize>,
) -> (Option<usize>, Option<usize>) {
    let total = along.last().copied().unwrap_or_default();
    let heights: Vec<(usize, f64)> = points.iter().enumerate().filter_map(|(index, point)| point.ele.map(|ele| (index, ele))).collect();
    let (low, high) = range;
    let span = (high - low).max(10.0);

    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), PROFILE_H), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(1.0));
    painter.rect(rect, 8, theme::bg(), Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
    let plot = rect.shrink2(Vec2::new(8.0, 12.0));
    let x_of = |index: usize| plot.left() + (along[index] / total.max(1.0)) as f32 * plot.width();
    let y_of = |ele: f64| plot.bottom() - ((ele - low) / span) as f32 * plot.height();

    let mut line: Vec<Pos2> = Vec::with_capacity(heights.len().min(4000));
    for &(index, ele) in &heights {
        let pos = Pos2::new(x_of(index), y_of(ele));
        if line.last().is_none_or(|last| pos.x - last.x >= 0.5) {
            line.push(pos);
        }
    }
    let (top, foot) = (LINE.gamma_multiply(0.35), LINE.gamma_multiply(0.03));
    let mut mesh = egui::Mesh::default();
    for pair in line.windows(2) {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], top);
        mesh.colored_vertex(pair[1], top);
        mesh.colored_vertex(Pos2::new(pair[1].x, plot.bottom()), foot);
        mesh.colored_vertex(Pos2::new(pair[0].x, plot.bottom()), foot);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::line(line, Stroke::new(1.8, LINE)));

    let pick = |pos: Pos2| {
        let target = f64::from(((pos.x - plot.left()) / plot.width()).clamp(0.0, 1.0)) * total;
        let at = heights.partition_point(|(index, _)| along[*index] < target);
        [at.checked_sub(1), Some(at)]
            .into_iter()
            .flatten()
            .filter_map(|slot| heights.get(slot))
            .min_by(|a, b| (along[a.0] - target).abs().total_cmp(&(along[b.0] - target).abs()))
            .map(|(index, _)| *index)
    };
    let hovered = response.hover_pos().and_then(pick);
    let picked = if response.clicked() || response.dragged() { response.interact_pointer_pos().and_then(pick) } else { None };
    if let Some(ele) = focus.and_then(|index| points.get(index).and_then(|point| point.ele.map(|ele| (index, ele)))) {
        painter.circle(Pos2::new(x_of(ele.0), y_of(ele.1)), 4.5, ACCENT, Stroke::new(2.0, WHITE));
    }
    if let Some((index, ele)) = hovered.or(map_hover).and_then(|index| points.get(index).and_then(|point| point.ele.map(|ele| (index, ele)))) {
        let pos = Pos2::new(x_of(index), y_of(ele));
        painter.vline(pos.x, rect.y_range().shrink(4.0), Stroke::new(1.0, theme::muted().gamma_multiply(0.6)));
        painter.circle(pos, 4.5, WHITE, Stroke::new(2.0, LINE));
        let text = format!("{ele:.0} m · {}", format_distance(along[index]));
        let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), theme::text());
        let width = galley.size().x + 10.0;
        let left = (pos.x - width / 2.0).clamp(rect.left() + 4.0, rect.right() - 4.0 - width);
        let label = Rect::from_min_size(Pos2::new(left, rect.top() + 3.0), Vec2::new(width, galley.size().y + 4.0));
        painter.rect(label, 4, theme::bg(), Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
        painter.galley(label.min + Vec2::new(5.0, 2.0), galley, theme::text());
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    (hovered, picked)
}

/// Tiles of two a row, each a value over its name. `hint`, or the name,
/// shows on hover, since long names are cut short.
fn stat_tiles(ui: &mut egui::Ui, tiles: &[(String, String, Option<String>)]) {
    let gap = 8.0;
    let width = ((ui.available_width() - gap) / 2.0).floor();
    for pair in tiles.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, value, hint) in pair {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 54.0), Sense::hover());
                let painter = ui.painter();
                painter.rect(rect, 8, theme::bg(), Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
                let value = truncated(painter, value.clone(), 17.0, theme::text(), width - 20.0);
                painter.galley(rect.left_top() + Vec2::new(10.0, 8.0), value, theme::text());
                let name = truncated(painter, label.clone(), 11.0, theme::muted(), width - 20.0);
                painter.galley(rect.left_bottom() + Vec2::new(10.0, -9.0 - name.size().y), name, theme::muted());
                response.on_hover_text(hint.as_deref().unwrap_or(label));
            }
        });
    }
}

/// A small upper-case title, with `right` laid out from the right edge.
fn section(ui: &mut egui::Ui, title: &str, right: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        ui.label(RichText::new(title.to_uppercase()).size(10.5).strong().color(theme::muted()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

fn panel_card() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::bg())
        .stroke(Stroke::new(1.0, theme::hairline()))
        .corner_radius(8)
        .inner_margin(Margin::same(10))
}

fn badge(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::well())
        .corner_radius(4)
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(10.5).strong().color(theme::muted())));
}

/// A card floating over the map in `within`, placed by its `pivot` at `at`.
fn floating<R>(ctx: &egui::Context, id: &str, at: Pos2, pivot: Align2, within: Rect, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Area::new(egui::Id::unique(id))
        .order(egui::Order::Middle)
        .fixed_pos(at)
        .pivot(pivot)
        .constrain_to(within.shrink(4.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::bg())
                .stroke(Stroke::new(1.0, theme::hairline()))
                .corner_radius(10)
                .shadow(theme::card_shadow())
                .inner_margin(Margin::same(4))
                .show(ui, add)
                .inner
        })
}

#[derive(Clone, Copy)]
enum Glyph {
    Move,
    Box,
    Original,
    Trash,
    Chevron,
}

fn tool_button(ui: &mut egui::Ui, glyph: Glyph, selected: bool, hint: &str) -> egui::Response {
    let response = ui.add(egui::Button::selectable(selected, "").min_size(Vec2::splat(TOOL))).on_hover_text(hint);
    theme_glyph(ui.painter(), glyph, response.rect.center(), if selected { ACCENT } else { theme::text() });
    response
}

fn glyph_button(ui: &mut egui::Ui, glyph: Glyph, hint: &str) -> egui::Response {
    let response = ui.add(egui::Button::new("").frame_when_inactive(false).min_size(Vec2::new(26.0, 24.0))).on_hover_text(hint);
    theme_glyph(ui.painter(), glyph, response.rect.center(), theme::muted());
    response
}

fn nav_button(ui: &mut egui::Ui, text: &str, enabled: bool, hint: &str) -> bool {
    let button = egui::Button::new(RichText::new(text).size(15.0)).frame_when_inactive(false).min_size(Vec2::new(24.0, 24.0));
    ui.add_enabled(enabled, button).on_hover_text(hint).clicked()
}

/// Line glyphs about 14 points wide, in the style of the theme's icons.
fn theme_glyph(painter: &egui::Painter, glyph: Glyph, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    let at = |x: f32, y: f32| center + Vec2::new(x, y);
    match glyph {
        Glyph::Move => {
            let arrow = [(-4.0, -6.5), (-4.0, 5.0), (-1.1, 2.4), (1.1, 7.0), (2.9, 6.1), (0.8, 1.6), (4.5, 1.4)];
            painter.add(egui::Shape::closed_line(arrow.iter().map(|(x, y)| at(*x, *y)).collect(), stroke));
        }
        Glyph::Box => {
            let corners = [at(-6.5, -5.5), at(6.5, -5.5), at(6.5, 5.5), at(-6.5, 5.5), at(-6.5, -5.5)];
            painter.add(egui::Shape::dashed_line(&corners, stroke, 2.5, 2.0));
        }
        Glyph::Original => {
            let wave = |dy: f32| (0..=12).map(|step| {
                let x = -7.0 + 14.0 * step as f32 / 12.0;
                at(x, dy + 2.2 * (x * 0.55).sin())
            }).collect::<Vec<_>>();
            painter.add(egui::Shape::line(wave(-2.5), stroke));
            painter.add(egui::Shape::line(wave(2.5), Stroke::new(1.4, color.gamma_multiply(0.4))));
        }
        Glyph::Trash => {
            painter.hline((center.x - 5.5)..=(center.x + 5.5), center.y - 4.0, stroke);
            painter.add(egui::Shape::line(vec![at(-2.0, -4.0), at(-2.0, -6.0), at(2.0, -6.0), at(2.0, -4.0)], stroke));
            painter.add(egui::Shape::line(vec![at(-4.2, -4.0), at(-3.4, 6.0), at(3.4, 6.0), at(4.2, -4.0)], stroke));
            for x in [-1.3, 1.3] {
                painter.vline(center.x + x, (center.y - 1.5)..=(center.y + 3.5), stroke);
            }
        }
        Glyph::Chevron => {
            painter.add(egui::Shape::line(vec![at(-3.5, -1.5), at(0.0, 2.0), at(3.5, -1.5)], Stroke::new(1.6, color)));
        }
    }
}

/// A plane on a white disc at `center`, flying along `heading`.
fn paint_plane(painter: &egui::Painter, center: Pos2, heading: Vec2) {
    painter.circle(center, 10.0, WHITE, Stroke::new(1.5, FLIGHT));
    let along = heading.normalized();
    let side = along.rot90();
    let at = |x: f32, y: f32| center + along * x + side * y;
    painter.line_segment([at(-6.0, 0.0), at(6.0, 0.0)], Stroke::new(2.2, FLIGHT));
    for flip in [-1.0, 1.0] {
        let mut wing = vec![at(1.5, 0.0), at(-1.5, 6.0 * flip), at(-3.2, 6.0 * flip), at(-1.2, 0.0)];
        let mut tail = vec![at(-4.4, 0.0), at(-5.8, 2.8 * flip), at(-6.8, 2.8 * flip), at(-6.0, 0.0)];
        // Clockwise on screen, as the tessellator's edge smoothing expects.
        if flip < 0.0 {
            wing.reverse();
            tail.reverse();
        }
        painter.add(egui::Shape::convex_polygon(wing, FLIGHT, Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(tail, FLIGHT, Stroke::NONE));
    }
}

/// A map pin with its tip at `pos`, outlined in white.
fn paint_marker(painter: &egui::Painter, pos: Pos2, color: Color32) {
    painter.circle_filled(pos, 3.0, Color32::from_black_alpha(60));
    theme::paint_pin(painter, pos - Vec2::new(0.0, 1.45 * 9.5), 9.5, WHITE, WHITE);
    theme::paint_pin(painter, pos - Vec2::new(0.0, 1.45 * 9.5 + 0.5), 8.0, color, WHITE);
}

/// The number, time, and height (when the track has `heights`) of the
/// point under the pointer, in a card beside it.
fn paint_point_card(painter: &egui::Painter, rect: Rect, pointer: Pos2, index: usize, point: &EditPoint, heights: bool) {
    let title = painter.layout_no_wrap(format!("{} {}", tr("Point"), grouped(index + 1)), FontId::proportional(12.5), theme::text());
    let mut details = Vec::new();
    if let Some(ms) = point.time {
        details.push(format_timestamp(ms.div_euclid(1000)));
    }
    if let Some(ele) = point.ele.filter(|_| heights) {
        details.push(format!("{ele:.0} m"));
    }
    let detail = (!details.is_empty()).then(|| painter.layout_no_wrap(details.join("  ·  "), FontId::proportional(11.5), theme::muted()));
    let width = title.size().x.max(detail.as_ref().map_or(0.0, |galley| galley.size().x)) + 20.0;
    let height = title.size().y + detail.as_ref().map_or(0.0, |galley| galley.size().y + 2.0) + 14.0;
    let mut at = pointer + Vec2::splat(16.0);
    if at.x + width > rect.right() - 8.0 {
        at.x = pointer.x - 16.0 - width;
    }
    if at.y + height > rect.bottom() - 8.0 {
        at.y = pointer.y - 16.0 - height;
    }
    let card = Rect::from_min_size(at, Vec2::new(width, height));
    painter.add(theme::card_shadow().as_shape(card, 8));
    painter.rect(card, 8, theme::bg(), Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
    let title_height = title.size().y;
    painter.galley(at + Vec2::new(10.0, 7.0), title, theme::text());
    if let Some(detail) = detail {
        painter.galley(at + Vec2::new(10.0, 9.0 + title_height), detail, theme::muted());
    }
}

/// The cleanup and flight thresholds, as in GPXViewer's Settings dialog.
/// Returns whether one changed.
pub(super) fn settings_form(ui: &mut egui::Ui, id: &str, settings: &mut Settings) -> bool {
    let before = *settings;
    egui::Grid::new(id).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        number(ui, tr("Speed limit"), &mut settings.max_speed_kmh, 1.0..=2000.0, " km/h", tr("Points reached faster than this are dropped, unless they are a long gap away."));
        number(ui, tr("Cluster radius"), &mut settings.cluster_radius_m, 1.0..=10_000.0, " m", tr("Points that stay within this distance are merged into one."));
        number(ui, tr("Long gap"), &mut settings.long_gap_m, 1.0..=100_000.0, " m", tr("Steps longer than this are kept at any speed, and can be bridged with a curve."));
        number(ui, tr("Tolerance"), &mut settings.tolerance_m, 0.1..=1000.0, " m", tr("A point closer than this to the curve through its neighbors is redundant. Smaller keeps more points."));
        number(ui, tr("Stay"), &mut settings.dwell_hours, 0.1..=240.0, " h", tr("Merged points that lasted this long count as a stay."));
        number(ui, tr("Flight distance"), &mut settings.flight_km, 1.0..=20_000.0, " km", tr("Steps longer than this are flights."));
        number(ui, tr("Flight speed"), &mut settings.flight_kmh, 1.0..=2000.0, " km/h", tr("Steps faster than this are flights."));
    });
    ui.checkbox(&mut settings.remove_redundant, tr("Remove redundant points"));
    ui.checkbox(&mut settings.fill_gaps, tr("Fill long gaps with a curve"));
    *settings != before
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::muted()));
}

fn row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(RichText::new(label).color(theme::muted()));
    ui.label(value);
    ui.end_row();
}

fn number(ui: &mut egui::Ui, label: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>, suffix: &str, hint: &str) {
    ui.label(label).on_hover_text(hint);
    let speed = (*value * 0.01).max(0.1);
    ui.add(egui::DragValue::new(value).range(range).speed(speed).max_decimals(1).suffix(suffix)).on_hover_text(hint);
    ui.end_row();
}

/// Whether any point has a height. Some apps write 0 for every point when
/// they recorded none.
fn has_heights(points: &[EditPoint]) -> bool {
    points.iter().any(|point| point.ele.is_some_and(|ele| ele != 0.0))
}

fn format_hours(hours: f64) -> String {
    let text = format!("{hours:.1}");
    text.strip_suffix(".0").map(str::to_string).unwrap_or(text)
}

fn closest_on_segment(pos: Pos2, a: Pos2, b: Pos2) -> Pos2 {
    let along = b - a;
    let t = ((pos - a).dot(along) / along.length_sq().max(1e-6)).clamp(0.0, 1.0);
    a + along * t
}

fn distance_to_segment(pos: Pos2, a: Pos2, b: Pos2) -> f32 {
    pos.distance(closest_on_segment(pos, a, b))
}

/// A track's line fitted in `rect`, from a green start to a red end, for
/// its tile in the folder grid.
pub(super) fn paint_track_shape(painter: &egui::Painter, rect: Rect, segments: &[crate::track::MapSegment], bounds: crate::track::Bounds) {
    if bounds.is_empty() {
        return;
    }
    let [w, h] = bounds.size();
    let room = rect.shrink(rect.width() * 0.14);
    let scale = (f64::from(room.width()) / w.max(1e-12)).min(f64::from(room.height()) / h.max(1e-12));
    let [cx, cy] = bounds.center();
    let at = |[x, y]: [f64; 2]| room.center() + Vec2::new(((x - cx) * scale) as f32, ((y - cy) * scale) as f32);
    for segment in segments {
        let mut line: Vec<Pos2> = Vec::with_capacity(segment.points.len().min(2000));
        for point in &segment.points {
            let pos = at(*point);
            if line.last().is_none_or(|last: &Pos2| last.distance(pos) >= 1.0) {
                line.push(pos);
            }
        }
        if line.len() >= 2 {
            painter.add(egui::Shape::line(line.clone(), Stroke::new(4.0, Color32::from_white_alpha(220))));
            painter.add(egui::Shape::line(line, Stroke::new(2.2, LINE)));
        }
    }
    let first = segments.first().and_then(|segment| segment.points.first());
    let last = segments.last().and_then(|segment| segment.points.last());
    for (point, color) in [(first, START), (last, DANGER)] {
        if let Some(point) = point {
            painter.circle(at(*point), 4.0, color, Stroke::new(1.5, WHITE));
        }
    }
}

/// "GPX", "KML", or "JSON", from the file name.
pub(super) fn format_badge(path: &str) -> String {
    Path::new(path).extension().map(|ext| ext.to_string_lossy().to_uppercase()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ele: Option<f64>) -> EditPoint {
        EditPoint { lat: 0.0, lon: 0.0, ele, time: None, dwell_ms: 0 }
    }

    /// An editor for a track that isn't there, so it ends up with an error
    /// unless edits are restored.
    fn editor(ctx: &egui::Context) -> TrackEditor {
        TrackEditor::new(PathBuf::from("/nowhere/ride.kml"), Settings::default(), ctx)
    }

    /// Waits for the editor's thread to answer.
    fn opened(editor: &mut TrackEditor) {
        for _ in 0..500 {
            editor.pump();
            if editor.opening.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("the track was never read");
    }

    #[test]
    fn undo_keeps_its_share_of_points() {
        let ctx = egui::Context::default();
        let mut editor = editor(&ctx);
        editor.points = vec![at(None); 400_000];
        for _ in 0..5 {
            editor.push_undo();
        }
        // Two copies of 400,000 points fit in a million, a third doesn't.
        assert_eq!(editor.undo.len(), 2);
        // The last edit can be undone however large the track.
        editor.points = vec![at(None); 1_100_000];
        editor.push_undo();
        assert_eq!(editor.undo.len(), 1);
    }

    #[test]
    fn kept_edits_come_back_when_the_track_is_gone() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("picasso-track-recovery-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_recovery(&dir).is_none());
        let mut quitting = editor(&ctx);
        quitting.name = "Ride".into();
        quitting.points = vec![EditPoint::new(25.0, 121.0, Some(3.0), Some(1_000)), EditPoint::new(25.1, 121.1, None, Some(61_000))];
        let keeper = Keeper::new(&dir);
        keeper.keep(&quitting);
        keeper.flush();
        let recovery = read_recovery(&dir).unwrap();
        assert_eq!(recovery.path, PathBuf::from("/nowhere/ride.kml"));

        let mut restored = editor(&ctx);
        restored.restored = Some(recovery.loaded);
        opened(&mut restored);
        assert!(restored.error.is_none() && restored.dirty);
        assert_eq!(restored.name, "Ride");
        assert_eq!(restored.points, quitting.points);
        assert_eq!(restored.raw, quitting.points);

        // A removal after a save wins over the writes sent before it.
        keeper.keep(&quitting);
        keeper.keep(&restored);
        keeper.forget();
        keeper.flush();
        assert!(!dir.join(RECOVERY_FILE).exists());
        std::fs::write(dir.join(RECOVERY_FILE), "not json").unwrap();
        assert!(read_recovery(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_thread_that_stopped_leaves_the_editor() {
        let ctx = egui::Context::default();
        let mut editor = editor(&ctx);
        opened(&mut editor);
        assert!(editor.error.is_some());
        let (sender, receiver) = crossbeam_channel::bounded::<Vec<EditPoint>>(1);
        drop(sender);
        editor.cleaning = Some(receiver);
        assert!(editor.busy());
        editor.pump();
        assert!(!editor.busy());
        assert_eq!(editor.note, tr("The cleanup stopped unexpectedly."));
    }

    #[test]
    fn heights_of_zero_everywhere_count_as_none() {
        assert!(!has_heights(&[at(None), at(None)]));
        assert!(!has_heights(&[at(Some(0.0)), at(Some(0.0))]));
        assert!(has_heights(&[at(Some(0.0)), at(Some(12.5))]));
        assert!(has_heights(&[at(None), at(Some(-3.0))]));
    }

    #[test]
    fn undo_over_a_cleanup_brings_its_settings_back() {
        let ctx = egui::Context::default();
        let mut editor = editor(&ctx);
        opened(&mut editor);
        editor.points = vec![at(None); 3];
        let old = editor.settings;
        editor.settings.fill_gaps = !old.fill_gaps;
        let new = editor.settings;
        let (sender, receiver) = crossbeam_channel::bounded(1);
        sender.send(vec![at(None); 2]).unwrap();
        editor.cleaning = Some(receiver);
        editor.pump();
        assert_eq!((editor.points.len(), editor.points_settings), (2, new));
        // Undoing an edit leaves a change in the panel not applied yet.
        editor.push_undo();
        editor.points[0].lat = 1.0;
        editor.settings.max_speed_kmh = 7.0;
        editor.undo();
        assert_eq!((editor.points[0].lat, editor.settings.max_speed_kmh), (0.0, 7.0));
        // Undoing the cleanup puts its settings back.
        editor.undo();
        assert_eq!((editor.points.len(), editor.settings, editor.points_settings), (3, old, old));
        editor.redo();
        assert_eq!((editor.points.len(), editor.settings, editor.points_settings), (2, new, new));
    }

    #[test]
    fn the_track_itself_is_the_same_file_by_any_name() {
        let dir = std::env::temp_dir().join(format!("picasso-same-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let track = dir.join("ride.gpx");
        std::fs::write(&track, "x").unwrap();
        assert!(same_file(&dir.join(".").join("ride.gpx"), &track));
        assert!(!same_file(&dir.join("ride-optimized.gpx"), &track));
        std::fs::write(dir.join("ride-optimized.gpx"), "x").unwrap();
        assert!(!same_file(&dir.join("ride-optimized.gpx"), &track));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn looked_up_heights_fill_the_points_and_undo_takes_them_off() {
        let ctx = egui::Context::default();
        let mut editor = editor(&ctx);
        editor.points = vec![at(Some(0.0)); 2];
        editor.raw = vec![at(None); 3];
        // Offline: nothing changes and nothing is left to undo.
        editor.take_heights(&[None; 5]);
        assert!(editor.undo.is_empty() && !editor.terrain_heights);
        assert!(!has_heights(&editor.points));
        editor.take_heights(&[Some(512.04), None, Some(1.0), Some(2.0), Some(3.0)]);
        assert_eq!(editor.points.iter().map(|point| point.ele).collect::<Vec<_>>(), [Some(512.0), None]);
        assert_eq!(editor.raw.iter().map(|point| point.ele).collect::<Vec<_>>(), [Some(1.0), Some(2.0), Some(3.0)]);
        assert!(editor.terrain_heights && editor.dirty);
        assert!(editor.note.contains("1 point still has no height"), "{}", editor.note);
        // Looking up again fills only the point still without a height.
        editor.take_heights(&[Some(9.0), Some(640.0), Some(9.0), Some(9.0), Some(9.0)]);
        assert_eq!(editor.points.iter().map(|point| point.ele).collect::<Vec<_>>(), [Some(512.0), Some(640.0)]);
        assert_eq!(editor.raw.iter().map(|point| point.ele).collect::<Vec<_>>(), [Some(1.0), Some(2.0), Some(3.0)]);
        editor.undo();
        editor.undo();
        assert!(!has_heights(&editor.points));
        // A track on the sea stays as it is, with nothing to undo.
        let undone = editor.undo.len();
        editor.take_heights(&[Some(0.0), Some(0.02), None, None, None]);
        assert_eq!(editor.undo.len(), undone);
        assert!(!has_heights(&editor.points));
    }
}
