//! Picasso application state: library navigation, viewer, and background jobs.

mod about;
mod dates;
mod dupes;
mod editor;
mod editor_panel;
mod editor_stage;
mod export;
mod frame_stats;
mod geotagging;
mod help;
mod library;
mod loader;
mod places;
mod recovery;
mod renamer;
mod settings;
mod share;
mod side;
mod stats;
mod story;
mod travel;
mod theme;
mod track_editor;
mod tree;
mod trips;
mod unreadable;
mod updates;
mod viewer;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use eframe::egui::{self, Color32, Key, Modifiers, Rect, TextureHandle, TextureOptions, Vec2};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::Config;
use crate::db::{self, Database};
use crate::geo::MapSource;
use crate::i18n::{self, tr, trf, Lang};
use crate::thumbs::{self, ThumbStore};
use crate::types::{folder_label, turn, FileType, Nav, Photo, Sort};
use crate::worker::{self, Command, DisplayGate, ThumbJob, ThumbWanted, UiEvent};

use library::CachedLayout;
use side::SidePanel;
use theme::ThemeMode;

/// How long the status bar offers to undo a Move to trash.
const UNDO_TRASH_FOR: Duration = Duration::from_secs(120);
/// Range of the thumbnail size slider, in points.
const MIN_THUMB: f32 = 96.0;
const MAX_THUMB: f32 = 320.0;
/// Pause after the last keystroke before the search runs.
const SEARCH_DELAY: Duration = Duration::from_millis(250);
/// A photo's histogram and its EXIF as label and value.
type PhotoInfo = (crate::photo_info::Histogram, Vec<(&'static str, String)>);
/// How long the info and edit panels keep the last photo's histogram and
/// EXIF while the next photo's load, so a quick change doesn't blank them.
const PANEL_HOLD: Duration = Duration::from_millis(600);
const SEARCH_HINT: &str =
    "Finds every word in file and folder names, captions, cameras, lenses, and albums. Add a date such as 2019, 2019-05, or 2019-05-17, or narrow with type:video, type:raw, gps:no, is:edited, is:starred, camera:, lens:, focal:24-70, or mp:20-. A word starting with - leaves out what it finds, as in -beach.";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Browse,
    View,
}

#[derive(Clone, Copy)]
enum Zoom {
    Fit,
    Scale(f32),
}

struct Slideshow {
    playing: bool,
    interval: f32,
    last: Instant,
    /// The photo shown before this one, and when the fade from it began.
    fade: Option<(i64, Instant)>,
    /// Photos still to show when shuffling, the next one last.
    queue: Vec<i64>,
}

/// Seconds one slideshow photo takes to fade into the next.
const SLIDE_FADE: f32 = 0.7;
/// Seconds a slideshow waits past its interval for the next photo to load.
const SLIDE_WAIT: f32 = 3.0;

/// The recent searches listed under the search box.
#[derive(Default)]
struct SearchMenu {
    open: bool,
    /// Something was typed since the box got the focus, so the list shows
    /// only the searches that contain it.
    typed: bool,
    /// The search picked with the arrow keys, in `shown`.
    pick: Option<usize>,
    shown: Vec<String>,
}

enum Modal {
    None,
    AddFolder { path: String, error: String },
    NewAlbum { name: String, error: String },
    /// `bytes` is their total size, added up when the dialog first shows.
    Trash { ids: Vec<i64>, bytes: Option<u64> },
    /// Files the scans could not read; `confirm_all` while asking before
    /// moving them all to the trash.
    Unreadable { confirm_all: bool },
    Export(export::ExportDialog),
    AdjustDate(dates::DateDialog),
    Geotag(geotagging::GeotagDialog),
    Rename(renamer::RenameDialog),
    /// The duplicate search; `confirm_all` while asking before moving every
    /// extra copy to the trash.
    Duplicates { confirm_all: bool },
    Trip(trips::TripDialog),
    AddToTrip(trips::AddDialog),
    DeleteTrip(trips::DeleteDialog),
    Unavailable(trips::UnavailableDialog),
    DayDetails(trips::DayDialog),
    /// Delete Story (id, name)?
    DeleteStory(i64, String),
    ExportStory(story::ExportDialog),
    Help,
    About,
    Settings,
}

impl Modal {
    fn trash(ids: Vec<i64>) -> Self {
        Modal::Trash { ids, bytes: None }
    }
}

struct ThumbTex {
    texture: TextureHandle,
    w: usize,
    h: usize,
    /// Value of [`ThumbCache::clock`] when last drawn.
    used: u64,
    /// Short edge of the sharper copy last asked for; 0 when none was.
    asked: u32,
}

impl ThumbTex {
    /// Larger than the cached thumbnail, made for a big tile.
    fn sharp(&self) -> bool {
        self.w.min(self.h) > worker::THUMB_EDGE as usize
    }
}

/// Thumbnail textures, least recently drawn first out.
struct ThumbCache {
    map: HashMap<i64, ThumbTex>,
    clock: u64,
    /// Thumbnails drawn or prefetched in the last frame. The cache holds at
    /// least twice that many, so a large window with small thumbnails doesn't
    /// evict what it is about to draw.
    keep: usize,
}

impl ThumbCache {
    /// Each thumbnail texture is about 600 KB, so this keeps them to about
    /// 250 MB unless the window shows more at once.
    const CAPACITY: usize = 400;
    /// Sharper copies for big tiles run to a few MB each, so fewer are kept.
    const SHARP_CAPACITY: usize = 48;

    fn new() -> Self {
        Self {
            map: HashMap::new(),
            clock: 0,
            keep: 0,
        }
    }

    fn get(&mut self, id: i64) -> Option<(TextureHandle, usize, usize)> {
        self.clock += 1;
        let tex = self.map.get_mut(&id)?;
        tex.used = self.clock;
        Some((tex.texture.clone(), tex.w, tex.h))
    }

    fn insert(&mut self, id: i64, mut tex: ThumbTex) {
        self.clock += 1;
        tex.used = self.clock;
        self.map.insert(id, tex);
        let sharp = Self::SHARP_CAPACITY.max(2 * self.keep);
        if self.map.values().filter(|tex| tex.sharp()).count() > sharp {
            let mut ages: Vec<(u64, i64)> = self.map.iter().filter(|(_, tex)| tex.sharp()).map(|(id, tex)| (tex.used, *id)).collect();
            ages.sort_unstable();
            for (_, old) in ages.into_iter().take(sharp / 4) {
                self.map.remove(&old);
            }
        }
        let capacity = Self::CAPACITY.max(2 * self.keep);
        if self.map.len() > capacity {
            // Evict in chunks so the scan for the oldest runs rarely.
            let mut ages: Vec<(u64, i64)> = self.map.iter().map(|(id, tex)| (tex.used, *id)).collect();
            ages.sort_unstable();
            for (_, old) in ages.into_iter().take(capacity / 10) {
                self.map.remove(&old);
            }
        }
    }
}

struct DisplayTex {
    id: i64,
    texture: TextureHandle,
    w: usize,
    h: usize,
    histogram: Box<crate::photo_info::Histogram>,
    exif: Vec<(&'static str, String)>,
}

pub struct PicassoApp {
    db: Database,
    /// Settings shared by every library, in the system's settings folder.
    config: Config,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    cmd_tx: Sender<Command>,
    urgent_tx: Sender<ThumbJob>,
    events: Receiver<UiEvent>,
    display_gate: DisplayGate,
    watcher: Option<RecommendedWatcher>,
    watch_rx: Receiver<Result<Event, notify::Error>>,
    watched: Vec<PathBuf>,
    watch_deadline: HashMap<i64, Instant>,

    folders: Vec<crate::types::Folder>,
    /// Folders left out of scanning, as (path, added folder id).
    excluded: Vec<(String, i64)>,
    folder_tree: Vec<tree::FolderRoot>,
    /// Expanded or collapsed state of folder tree rows the user toggled.
    tree_open: HashMap<String, bool>,
    albums: Vec<crate::types::Album>,
    /// Active Trips, then archived ones.
    trips: Vec<crate::trip::Trip>,
    /// The Story being edited, shown in place of its Trip's photos.
    story: Option<story::StoryEditor>,
    /// The archived Trips are listed in the side bar.
    show_archived_trips: bool,
    /// Which of the Trip's photos the grid shows.
    trip_filter: crate::trip::MemberQuery,
    /// The filters each Trip was left with, for coming back to it this session.
    trip_filters: HashMap<i64, crate::trip::MemberQuery>,
    /// The grid's selection and anchor when the viewer opened, given back when it closes.
    browse_selection: Option<(HashSet<i64>, Option<i64>)>,
    /// The Trip whose Days are being suggested in the background.
    suggesting: Option<i64>,
    /// The last Move to Day, to undo: the Trip and where its photos were.
    day_undo: Option<(i64, Vec<crate::trip::DayAssignment>)>,
    photos: Vec<Photo>,
    /// Position of each photo in `photos`.
    index_of: HashMap<i64, usize>,
    nav: Nav,
    sort: Sort,
    /// Visible filter independent of the photo ordering.
    edited_only: bool,
    search: String,
    /// When the typed search runs.
    search_due: Option<Instant>,
    /// Ctrl+F asked for the search box, with its text selected.
    focus_search: bool,
    search_menu: SearchMenu,
    /// A picture is being rendered for the clipboard.
    copying: bool,
    mode: Mode,
    viewer_index: usize,
    /// Photo the viewer's filmstrip last brought into view, and how far the
    /// strip is scrolled.
    film_shown: Option<i64>,
    film_offset: f32,
    /// Histogram and EXIF the info panel last showed, whose photo, and
    /// when the viewer moved on from it.
    info_held: Option<(i64, PhotoInfo, Option<Instant>)>,
    selection: HashSet<i64>,
    anchor: Option<i64>,
    grid_cols: usize,
    thumb_size: f32,
    scroll_to: Option<i64>,
    /// Photo at the top of the Library grid, kept in the settings at exit
    /// and scrolled to on the next start.
    library_top: Option<i64>,
    /// Grid scroll offset picked in the date strip.
    jump_to: Option<f32>,
    /// Last frame the library scroll bar was dragged, for its date bubble.
    scroll_hint_at: Option<Instant>,
    /// Grid scroll offset the date strip last saw, and when it last changed,
    /// so the strip names the month in view for a moment after scrolling.
    strip_offset: f32,
    strip_moved_at: Option<Instant>,
    /// Scroll from one offset to another the date strip is animating.
    strip_glide: Option<(f32, f32, Instant)>,
    /// Where on the date strip's handle the pointer took hold of it.
    strip_grab: Option<f32>,
    view_epoch: u64,
    layout: Option<CachedLayout>,

    thumbs: ThumbCache,
    thumb_waiting: HashSet<i64>,
    thumb_failed: HashSet<i64>,
    /// Thumbnails drawn or prefetched in the frame being built.
    thumb_drawn: HashSet<i64>,
    /// `thumb_drawn` of the last finished frame, shared with the decoders.
    thumb_wanted: ThumbWanted,
    displays: Vec<DisplayTex>,
    display_nonce: HashMap<i64, u64>,
    display_inflight: HashMap<i64, u64>,
    display_failed: HashSet<i64>,
    /// Photo whose full-resolution view was asked for, at most one at a time.
    sharp: Option<i64>,
    nonce_seq: u64,
    /// Player for the video on screen in the viewer.
    video: Option<viewer::VideoView>,
    /// Sound output, opened the first time a video plays.
    audio_out: Option<rodio::MixerDeviceSink>,
    audio_tried: bool,

    zoom: Zoom,
    pan: Vec2,
    stage_rect: Rect,
    show_info: bool,
    side: SidePanel,
    /// The tab the side panel opens on.
    side_last: SidePanel,
    /// Width of the photo tray's buttons and notes on the last frame.
    tray_rest: f32,
    exif: HashMap<i64, Vec<(&'static str, String)>>,
    exif_waiting: HashSet<i64>,
    side_zoom: u8,
    side_pan: Vec2,
    /// Wheel scroll toward the next zoom step of the Map panel.
    side_wheel: f32,
    /// Trackpad pinch toward the next zoom step of the Map panel.
    side_pinch: f32,
    side_map_photo: Option<i64>,
    caption_buf: String,
    caption_photo: Option<i64>,
    /// Language picked in Settings, or `None` to follow the system.
    language: Option<Lang>,
    theme_mode: ThemeMode,
    /// Seconds each photo stays on screen in a slideshow.
    slideshow_interval: f32,
    /// Slideshows show the photos in a random order.
    slideshow_shuffle: bool,
    /// Ask before moving photos to the system trash.
    confirm_trash: bool,
    settings: settings::SettingsState,
    updates: updates::UpdateState,
    /// Applied edit of every edited photo.
    edits: HashMap<i64, crate::edit::Settings>,
    /// Edit panel open in the viewer.
    editing: bool,
    edit: Option<editor::EditSession>,
    edit_tx: Sender<crate::edit::EditJob>,
    /// Settings copied with Ctrl+Shift+C, without crop.
    edit_clipboard: Option<crate::edit::Settings>,
    user_presets: Vec<crate::edit::presets::UserPreset>,
    events_tx: Sender<UiEvent>,
    slideshow: Option<Slideshow>,
    activity: String,
    /// When the latest Move to trash ran, and the path and folder of each
    /// file in it, for Undo.
    last_trashed: Option<(Instant, Vec<(String, i64)>)>,
    /// What the latest scan of each folder could not read, by folder id.
    unreadable: HashMap<i64, Vec<crate::types::Unreadable>>,
    export_run: Option<export::ExportRun>,
    dupes: Option<dupes::DupeScan>,
    dupes_run: u64,
    scans: i32,
    /// The catalog changed in the background and the view should be re-read.
    library_dirty: bool,
    last_reload: Instant,
    load_tx: Sender<loader::LoadJob>,
    load_rx: Receiver<loader::Loaded>,
    /// Number of the newest view asked of the loader.
    load_seq: u64,
    /// `db.changes()` when that view was asked for. A result read before a
    /// later write from this window, such as a star, is asked for again.
    load_changes: u64,
    load_retries: u8,
    /// The newest view hasn't arrived yet.
    loading: bool,
    /// Fit the map to the photos once the Places view arrives.
    frame_after_load: bool,
    modal: Modal,
    logo: TextureHandle,
    map_lat: f64,
    map_lon: f64,
    map_zoom: u8,
    map_pan: Vec2,
    /// Wheel scroll toward the next zoom step of the Places map.
    map_wheel: f32,
    /// Trackpad pinch toward the next zoom step of the Places map.
    map_pinch: f32,
    map_place: bool,
    /// Thumbnails of the cluster under the pointer on the Places map.
    map_card: Option<places::MapCard>,
    /// Clusters on the Places map in the latest frame, for the side panel.
    map_view_clusters: Vec<places::Cluster>,
    /// Statistics of the photos in view, worked out when first shown.
    stats: Option<stats::StatsPage>,
    /// Travel Map marks and view.
    travel: travel::Travel,
    /// Cluster under the pointer on the map, lit in the side panel.
    map_hover: Option<places::ClusterKey>,
    /// Cluster and track under the pointer in the side panel, lit on the map.
    map_hot: Option<places::ClusterKey>,
    track_hot: Option<usize>,
    /// Track whose photos were last selected on the map, drawn wider.
    picked_track: Option<i64>,
    /// Size of the Places map in the latest frame.
    map_size: Vec2,
    /// GPS tracks from the GPX and KML files in the folders.
    tracks: Vec<crate::track::MapTrack>,
    show_tracks: bool,
    /// Places shows only the starred photos.
    map_starred: bool,
    map_source: MapSource,
    carto_key: String,
    tiles: HashMap<(u8, u32, u32), places::MapTile>,
    tile_tx: Sender<crate::geo::TileJob>,
    tile_rx: Receiver<crate::geo::TileDone>,
    tile_wait: HashSet<(u8, u32, u32)>,
    tile_fail: HashSet<(u8, u32, u32)>,
    /// Set by `PICASSO_FRAME_STATS=1`.
    frame_stats: Option<frame_stats::FrameStats>,
    /// Added folder whose removal waits for confirmation. It is asked over
    /// whatever window is open, as Settings and the folder tree both offer it.
    remove_asked: Option<i64>,
    /// The GPS track open in the track editor, which fills the window.
    track_editor: Option<track_editor::TrackEditor>,
    /// Track edits kept when Picasso last quit, waiting for the user to
    /// restore or discard them.
    track_recovery: Option<track_editor::Recovery>,
    /// Keeps the open track's unsaved edits on disk.
    track_keeper: track_editor::Keeper,
}

/// The library window, or the recovery window when the library can't be
/// opened.
pub fn start(ctx: &egui::Context, args: Vec<PathBuf>) -> Box<dyn eframe::App> {
    match PicassoApp::new(ctx, args) {
        Ok(app) => Box::new(app),
        Err(err) => Box::new(recovery::Recovery::new(ctx, db::app_dirs().0, err)),
    }
}

impl PicassoApp {
    fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Result<Self, String> {
        let (data_dir, cache) = db::app_dirs();
        let _ = std::fs::create_dir_all(&data_dir);
        let track_recovery = track_editor::read_recovery(&data_dir);
        let track_keeper = track_editor::Keeper::new(&data_dir);
        let _ = std::fs::create_dir_all(cache.join("tiles"));
        // Face crops from older builds are no longer used.
        let _ = std::fs::remove_dir_all(cache.join("faces"));
        // Created before the worker threads, so they don't race to set it up.
        drop(ThumbStore::open(&cache));
        let migrate_cache = cache.clone();
        let _ = std::thread::Builder::new()
            .name("picasso-thumbs-migrate".into())
            .spawn(move || thumbs::migrate_files(&migrate_cache));
        let restored = crate::backup::apply_staged(&data_dir);
        let database = Database::open(&data_dir.join("picasso.db"))?;
        if crate::backup::due(&data_dir) {
            let data_dir = data_dir.clone();
            let _ = std::thread::Builder::new().name("picasso-backup".into()).spawn(move || {
                if let Err(err) = crate::backup::create(&data_dir) {
                    eprintln!("picasso backup: {err}");
                }
            });
        }
        let mut config = Config::load(&db::config_dir(), &database);
        FileType::set_enabled(&config.file_types());
        let theme_mode = ThemeMode::from_code(config.text("theme").unwrap_or_default());
        theme::apply(ctx, theme_mode.is_dark(system_dark(ctx)));
        let language = config.text("language").and_then(Lang::from_code);
        i18n::set(language.unwrap_or_else(i18n::system));
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (urgent_tx, urgent_rx) = crossbeam_channel::unbounded();
        let (normal_tx, normal_rx) = crossbeam_channel::unbounded();
        let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
        let display_gate = Arc::new(Mutex::new(HashMap::new()));
        let thumb_wanted: ThumbWanted = Arc::new(Mutex::new(HashSet::new()));
        for index in 0..worker::thumb_threads() {
            let ctx = ctx.clone();
            let cache = cache.clone();
            let gate = display_gate.clone();
            let wanted = thumb_wanted.clone();
            let events = ev_tx.clone();
            let urgent = urgent_rx.clone();
            // The first thread serves only the window, so what is on screen
            // never waits behind background thumbnails.
            let normal = (index > 0).then(|| normal_rx.clone());
            std::thread::Builder::new()
                .name(format!("picasso-thumbs-{index}"))
                .spawn(move || worker::thumb_loop(urgent, normal, events, ctx, cache, gate, wanted))
                .expect("thumb thread");
        }
        drop((urgent_rx, normal_rx));
        let events_tx = ev_tx.clone();
        let (edit_tx, edit_rx) = crossbeam_channel::unbounded();
        {
            let ctx = ctx.clone();
            let events = ev_tx.clone();
            std::thread::Builder::new()
                .name("picasso-edit".into())
                .spawn(move || crate::edit::edit_loop(edit_rx, events, ctx))
                .expect("edit thread");
        }
        {
            let ctx = ctx.clone();
            let db_path = data_dir.join("picasso.db");
            let cache = cache.clone();
            std::thread::Builder::new()
                .name("picasso-index".into())
                .spawn(move || worker::index_loop(db_path, cache, cmd_rx, normal_tx, ev_tx, ctx))
                .expect("index thread");
        }
        let (load_tx, load_rx) = loader::spawn(data_dir.join("picasso.db"), ctx.clone());
        let (tile_tx, tile_job_rx) = crossbeam_channel::unbounded();
        let (tile_done_tx, tile_rx) = crossbeam_channel::unbounded();
        for index in 0..2 {
            let cache = cache.clone();
            let jobs = tile_job_rx.clone();
            let done = tile_done_tx.clone();
            std::thread::Builder::new()
                .name(format!("picasso-tiles-{index}"))
                .spawn(move || crate::geo::tile_loop(jobs, done, cache))
                .expect("tile thread");
        }
        let map_source = config.text("map_source").and_then(MapSource::from_code).unwrap_or_default();
        let carto_key = settings::load_carto_key(&mut config);
        let (watch_tx, watch_rx) = crossbeam_channel::unbounded();
        let watcher = notify::recommended_watcher(move |result| {
            let _ = watch_tx.send(result);
        })
        .ok();
        let travel = travel::Travel::load(&config);

        let mut app = Self {
            db: database,
            config,
            data_dir,
            cache_dir: cache,
            cmd_tx,
            urgent_tx,
            events: ev_rx,
            display_gate,
            watcher,
            watch_rx,
            watched: Vec::new(),
            watch_deadline: HashMap::new(),
            folders: Vec::new(),
            excluded: Vec::new(),
            folder_tree: Vec::new(),
            tree_open: HashMap::new(),
            albums: Vec::new(),
            trips: Vec::new(),
            story: None,
            show_archived_trips: false,
            trip_filter: crate::trip::MemberQuery::default(),
            trip_filters: HashMap::new(),
            browse_selection: None,
            suggesting: None,
            day_undo: None,
            photos: Vec::new(),
            index_of: HashMap::new(),
            nav: Nav::Library,
            sort: Sort::DateNewest,
            edited_only: false,
            search: String::new(),
            search_due: None,
            focus_search: false,
            search_menu: SearchMenu::default(),
            copying: false,
            mode: Mode::Browse,
            viewer_index: 0,
            film_shown: None,
            film_offset: 0.0,
            info_held: None,
            selection: HashSet::new(),
            anchor: None,
            grid_cols: 1,
            thumb_size: 144.0,
            scroll_to: None,
            library_top: None,
            jump_to: None,
            scroll_hint_at: None,
            strip_offset: 0.0,
            strip_moved_at: None,
            strip_glide: None,
            strip_grab: None,
            view_epoch: 0,
            layout: None,
            thumbs: ThumbCache::new(),
            thumb_waiting: HashSet::new(),
            thumb_failed: HashSet::new(),
            thumb_drawn: HashSet::new(),
            thumb_wanted,
            displays: Vec::new(),
            display_nonce: HashMap::new(),
            display_inflight: HashMap::new(),
            display_failed: HashSet::new(),
            sharp: None,
            nonce_seq: 1,
            video: None,
            audio_out: None,
            audio_tried: false,
            zoom: Zoom::Fit,
            pan: Vec2::ZERO,
            stage_rect: Rect::NOTHING,
            show_info: false,
            side: SidePanel::None,
            side_last: SidePanel::Info,
            tray_rest: 0.0,
            exif: HashMap::new(),
            exif_waiting: HashSet::new(),
            side_zoom: 13,
            side_pan: Vec2::ZERO,
            side_wheel: 0.0,
            side_pinch: 0.0,
            side_map_photo: None,
            caption_buf: String::new(),
            caption_photo: None,
            language,
            theme_mode,
            slideshow_interval: 3.0,
            slideshow_shuffle: false,
            confirm_trash: true,
            settings: settings::SettingsState::default(),
            updates: updates::UpdateState::default(),
            edits: HashMap::new(),
            editing: false,
            edit: None,
            edit_tx,
            edit_clipboard: None,
            user_presets: Vec::new(),
            events_tx,
            slideshow: None,
            frame_stats: frame_stats::FrameStats::from_env(),
            remove_asked: None,
            track_editor: None,
            track_recovery,
            track_keeper,
            activity: String::new(),
            last_trashed: None,
            unreadable: HashMap::new(),
            export_run: None,
            dupes: None,
            dupes_run: 0,
            scans: 0,
            library_dirty: false,
            last_reload: Instant::now(),
            load_tx,
            load_rx,
            load_seq: 0,
            load_changes: 0,
            load_retries: 0,
            loading: false,
            frame_after_load: false,
            modal: Modal::None,
            logo: theme::load_logo(ctx),
            map_lat: 20.0,
            map_lon: 0.0,
            map_zoom: 2,
            map_pan: Vec2::ZERO,
            map_wheel: 0.0,
            map_pinch: 0.0,
            map_place: false,
            map_card: None,
            map_view_clusters: Vec::new(),
            stats: None,
            travel,
            map_hover: None,
            map_hot: None,
            track_hot: None,
            picked_track: None,
            map_size: Vec2::new(800.0, 600.0),
            tracks: Vec::new(),
            show_tracks: true,
            map_starred: false,
            map_source,
            carto_key,
            tiles: HashMap::new(),
            tile_tx,
            tile_rx,
            tile_wait: HashSet::new(),
            tile_fail: HashSet::new(),
        };

        let seeded = app.db.setting("seeded").is_some();
        if !seeded {
            if let Some(pictures) = db::pictures_dir() {
                if let Ok(id) = app.db.add_folder(&pictures) {
                    let _ = app.cmd_tx.send(Command::Scan(id));
                }
            }
            app.db.set_setting("seeded", "1");
        }
        for arg in args {
            let path = if arg.is_dir() {
                arg
            } else {
                arg.parent().map(Path::to_path_buf).unwrap_or(arg)
            };
            if path.is_dir() {
                if let Ok(id) = app.db.add_folder(&path) {
                    let _ = app.cmd_tx.send(Command::Scan(id));
                }
            }
        }
        if seeded {
            // The index thread resumes unfinished folders on its own; every
            // other folder is checked for files that changed while Picasso
            // was closed. Unchanged files are skipped by size and date.
            for folder in app.db.folders() {
                let _ = app.cmd_tx.send(Command::Scan(folder.id));
            }
        }
        for (id, steps, applied) in app.db.edit_histories() {
            let settings = crate::edit::History::from_json(&steps, applied).settings();
            if !settings.is_identity() {
                app.edits.insert(id, settings);
            }
        }
        app.user_presets = crate::edit::presets::user_presets_from_json(&app.db.setting("user_presets").unwrap_or_default());
        app.show_info = app.db.setting("viewer_info").as_deref() != Some("0");
        app.side = SidePanel::from_setting(app.db.setting("side_panel").as_deref());
        if app.side != SidePanel::None {
            app.side_last = app.side;
        }
        let config = &app.config;
        app.thumb_size = config.number("thumb_size").map_or(app.thumb_size, |size| size.clamp(MIN_THUMB, MAX_THUMB));
        app.slideshow_interval = config.number("slideshow_interval").map_or(app.slideshow_interval, |secs| secs.clamp(1.0, 15.0));
        app.slideshow_shuffle = config.flag("slideshow_shuffle").unwrap_or(false);
        app.confirm_trash = config.flag("confirm_trash").unwrap_or(true);
        app.show_tracks = config.flag("map_tracks").unwrap_or(true);
        app.map_starred = config.flag("map_starred").unwrap_or(false);
        app.library_top = config.text("library_top").and_then(|id| id.parse().ok());
        app.scroll_to = app.library_top;
        for (folder, item) in app.db.unreadable() {
            app.unreadable.entry(folder).or_default().push(item);
        }
        app.excluded = app.db.excluded_dirs();
        app.load_update();
        match restored {
            Some(Ok(())) => app.activity = tr("The library was restored from the backup").to_string(),
            Some(Err(err)) => app.activity = trf("Couldn't restore the backup: {}", &[&err]),
            None => {}
        }
        app.reload();
        app.sync_watches();
        Ok(app)
    }

    /// Re-reads the folders, their tree, and the albums, then the view.
    fn reload(&mut self) {
        self.library_dirty = false;
        self.last_reload = Instant::now();
        self.request_load(true);
    }

    /// Re-reads the photos of the current view only. Use [`Self::reload`]
    /// once photos were added, removed, or moved between albums, so the
    /// counts in the side bar follow.
    fn reload_view(&mut self) {
        self.request_load(false);
    }

    /// Asks the loader thread for the current view; [`Self::pump_loads`]
    /// shows it when it arrives.
    fn request_load(&mut self, full: bool) {
        self.load_seq += 1;
        self.load_changes = self.db.changes();
        self.loading = true;
        let _ = self.load_tx.send(loader::LoadJob {
            seq: self.load_seq,
            full,
            nav: self.nav.clone(),
            search: self.search.clone(),
            sort: self.sort,
            trip_filter: self.trip_filter,
        });
    }

    fn pump_loads(&mut self) {
        while let Ok(loaded) = self.load_rx.try_recv() {
            if let Some(lists) = loaded.lists {
                self.folders = lists.folders;
                self.folder_tree = lists.tree;
                self.albums = lists.albums;
                self.trips = lists.trips;
            }
            if loaded.seq != self.load_seq {
                continue;
            }
            if self.db.changes() != self.load_changes && self.load_retries < 3 {
                self.load_retries += 1;
                self.request_load(false);
                continue;
            }
            self.load_retries = 0;
            self.loading = false;
            let keep = self.photos.get(self.viewer_index).map(|photo| photo.id);
            self.show_photos(loaded.photos, keep);
            if std::mem::take(&mut self.frame_after_load) {
                self.frame_map();
            }
        }
    }

    /// Takes `ids` out of the view at once, ahead of the reload that follows.
    fn drop_from_view(&mut self, ids: &[i64]) {
        let gone: HashSet<i64> = ids.iter().copied().collect();
        let mut photos = std::mem::take(&mut self.photos);
        let keep = photos.get(self.viewer_index).map(|photo| photo.id);
        photos.retain(|photo| !gone.contains(&photo.id));
        self.show_photos(photos, keep);
    }

    /// Makes `photos` the current view, keeping the photo `keep` open in the
    /// viewer and the selection where they are still in it.
    fn show_photos(&mut self, photos: Vec<Photo>, keep: Option<i64>) {
        self.photos = photos;
        if self.edited_only {
            self.photos.retain(|photo| self.edits.contains_key(&photo.id));
        }
        if self.map_starred && self.nav == Nav::Places {
            self.photos.retain(|photo| photo.starred);
        }
        if let Some(edited) = crate::db::edited_filter(&self.search) {
            self.photos.retain(|photo| self.edits.contains_key(&photo.id) == edited);
        }
        self.index_of = self.photos.iter().enumerate().map(|(index, photo)| (photo.id, index)).collect();
        self.view_epoch = self.view_epoch.wrapping_add(1);
        if let Some(index) = keep.and_then(|id| self.index_of(id)) {
            self.viewer_index = index;
        }
        if self.photos.is_empty() {
            self.viewer_index = 0;
            if self.mode == Mode::View {
                self.mode = Mode::Browse;
            }
        } else {
            self.viewer_index = self.viewer_index.min(self.photos.len() - 1);
        }
        if matches!(self.nav, Nav::Places) {
            self.selection.retain(|id| self.db.photo(*id).is_some());
        } else {
            self.selection.retain(|id| self.index_of.contains_key(id));
        }
        if self.mode == Mode::View {
            self.sync_caption();
        }
    }

    /// Position of photo `id` in the current view.
    fn index_of(&self, id: i64) -> Option<usize> {
        self.index_of.get(&id).copied()
    }

    /// Positions of the first `count` selected photos, in view order.
    fn tray_indices(&self, count: usize) -> Vec<usize> {
        if count == 0 {
            return Vec::new();
        }
        if self.selection.len() * 4 > self.photos.len() {
            // Most photos are selected, so the first few turn up at once.
            return self
                .photos
                .iter()
                .enumerate()
                .filter(|(_, photo)| self.selection.contains(&photo.id))
                .take(count)
                .map(|(index, _)| index)
                .collect();
        }
        let mut indices: Vec<usize> = self.selection.iter().filter_map(|id| self.index_of(*id)).collect();
        if indices.len() > count {
            indices.select_nth_unstable(count - 1);
            indices.truncate(count);
        }
        indices.sort_unstable();
        indices
    }

    /// The selected photos in view order; any no longer in view come last.
    fn selection_in_order(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self.selection.iter().copied().collect();
        ids.sort_by_key(|id| self.index_of(*id).unwrap_or(usize::MAX));
        ids
    }

    fn sync_caption(&mut self) {
        let Some(photo) = self.photos.get(self.viewer_index) else {
            return;
        };
        if self.caption_photo != Some(photo.id) {
            self.caption_buf = photo.caption.clone();
            self.caption_photo = Some(photo.id);
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        self.pump_loads();
        for _ in 0..16 {
            match self.events.try_recv() {
                Ok(event) => self.on_event(event, ctx),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        if !self.events.is_empty() {
            ctx.request_repaint();
        }
        // While a scan streams in batches, re-read the view at most twice a
        // second so a large library stays responsive.
        // Showing only edited photos, the view waits for the viewer to close,
        // so a photo whose edits are reset doesn't vanish from under it.
        let editing_view = self.edited_only && self.mode == Mode::View;
        if self.library_dirty && !editing_view && !self.loading {
            let wait = Duration::from_millis(500);
            if self.scans == 0 || self.last_reload.elapsed() >= wait {
                self.reload();
            } else {
                ctx.request_repaint_after(wait.saturating_sub(self.last_reload.elapsed()));
            }
        }
    }

    fn on_event(&mut self, event: UiEvent, ctx: &egui::Context) {
        match event {
            UiEvent::ScanStarted(name) => {
                self.scans += 1;
                self.activity = trf("Scanning {}", &[&name]);
            }
            UiEvent::ScanProgress { folder, checked, indexed } => {
                self.activity = trf("Scanning {} · {} checked · {} updated", &[&folder, &checked, &indexed]);
            }
            UiEvent::ScanFinished { folder, unreadable } => {
                self.scans = self.scans.saturating_sub(1);
                if self.scans == 0 {
                    self.activity.clear();
                }
                if unreadable.is_empty() {
                    self.unreadable.remove(&folder);
                } else {
                    self.unreadable.insert(folder, unreadable);
                }
                self.library_dirty = true;
            }
            UiEvent::Library => self.library_dirty = true,
            UiEvent::Tracks(tracks) => self.tracks = tracks,
            UiEvent::Restored(ids) => {
                for id in ids {
                    let settings = self
                        .db
                        .edit_history(id)
                        .map(|(steps, applied)| crate::edit::History::from_json(&steps, applied).settings());
                    match settings.filter(|settings| !settings.is_identity()) {
                        Some(settings) => self.edits.insert(id, settings),
                        None => self.edits.remove(&id),
                    };
                    self.forget_thumb(id);
                    self.forget_display(id);
                }
                self.library_dirty = true;
            }
            UiEvent::Thumb { id, w, h, rgba, edit } => {
                self.thumb_waiting.remove(&id);
                if edit != self.edit_settings(id) {
                    // Rendered with older steps; asked for again when drawn.
                    return;
                }
                let old = self.thumbs.map.get(&id);
                if old.is_some_and(|old| old.w.min(old.h) > w.min(h)) {
                    return;
                }
                let asked = old.map_or(0, |old| old.asked);
                if let Some(texture) = upload(ctx, &format!("thumb-{id}"), w, h, &rgba) {
                    self.thumbs.insert(id, ThumbTex { texture, w, h, used: 0, asked });
                } else {
                    self.thumb_failed.insert(id);
                }
            }
            UiEvent::Display {
                id,
                nonce,
                w,
                h,
                rgba,
                histogram,
                exif,
                edit,
            } => {
                if self.display_nonce.get(&id) != Some(&nonce) {
                    return;
                }
                self.display_inflight.remove(&id);
                if edit != self.edit_settings(id) {
                    return;
                }
                if let Some(texture) = upload(ctx, &format!("full-{id}-{nonce}"), w, h, &rgba) {
                    self.displays.retain(|item| item.id != id);
                    self.displays.push(DisplayTex {
                        id,
                        texture,
                        w,
                        h,
                        histogram,
                        exif,
                    });
                    if self.displays.len() > 4 {
                        // A shuffled slideshow loads photos far from the one
                        // shown, so the oldest can be on screen or fading out.
                        let shown = self.photos.get(self.viewer_index).map(|photo| photo.id);
                        let fading = self.slideshow.as_ref().and_then(|show| show.fade).map(|(id, _)| id);
                        let oldest = self.displays.iter().position(|item| Some(item.id) != shown && Some(item.id) != fading);
                        self.displays.remove(oldest.unwrap_or(0));
                    }
                }
            }
            UiEvent::ThumbFailed { id } => {
                self.thumb_waiting.remove(&id);
                self.thumb_failed.insert(id);
            }
            UiEvent::ThumbSkipped { id } => {
                self.thumb_waiting.remove(&id);
                if let Some(tex) = self.thumbs.map.get_mut(&id) {
                    tex.asked = 0;
                }
            }
            UiEvent::DisplayFailed { id, nonce } => {
                if self.display_nonce.get(&id) == Some(&nonce) {
                    self.display_inflight.remove(&id);
                    // A full-resolution decode that fails leaves the usual view.
                    if !(self.sharp == Some(id) && self.display_texture(id).is_some()) {
                        self.display_failed.insert(id);
                    }
                }
            }
            UiEvent::Exif { id, rows } => {
                self.exif_waiting.remove(&id);
                if self.exif.len() >= 256 {
                    self.exif.clear();
                }
                self.exif.insert(id, rows);
            }
            event @ (UiEvent::EditBase { .. }
            | UiEvent::EditFailed { .. }
            | UiEvent::EditPreview { .. }
            | UiEvent::Exported(_)) => self.on_edit_event(event, ctx),
            UiEvent::Copied(result) => {
                self.copying = false;
                self.activity = match result {
                    Ok(()) => tr("Copied the picture").to_string(),
                    Err(err) => trf("Couldn't copy the picture: {}", &[&err]),
                };
            }
            UiEvent::BatchExported {
                exported,
                failed,
                error,
                finished,
            } => self.on_batch_export(exported, failed, error, finished),
            UiEvent::Duplicates {
                run,
                checked,
                total,
                groups,
            } => self.on_duplicates(run, checked, total, groups),
            UiEvent::DaysSuggested { trip_id, outcome } => self.on_days_suggested(trip_id, outcome),
            UiEvent::StoryPreviewed(story_id, outcome) => self.on_story_previewed(story_id, outcome),
            UiEvent::StoryExported(story_id, outcome) => self.on_story_exported(story_id, outcome),
        }
    }

    fn pump_watch(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.watch_rx.try_recv() {
            let Ok(event) = result else { continue };
            if !matches!(
                event.kind,
                notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)
            ) {
                continue;
            }
            for path in event.paths {
                if self.excluded.iter().any(|(dir, _)| path.starts_with(dir)) {
                    continue;
                }
                if let Some(id) = self.folder_for(&path) {
                    self.watch_deadline.insert(id, Instant::now() + Duration::from_millis(700));
                }
            }
        }
        let renaming = self.renaming();
        let due: Vec<i64> = self
            .watch_deadline
            .iter()
            .filter(|(_, when)| !renaming && Instant::now() >= **when)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.watch_deadline.remove(&id);
            let _ = self.cmd_tx.send(Command::Scan(id));
        }
        if !self.watch_deadline.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn folder_for(&self, path: &Path) -> Option<i64> {
        self.folders
            .iter()
            .filter(|folder| path.starts_with(&folder.path))
            .max_by_key(|folder| folder.path.len())
            .map(|folder| folder.id)
    }

    fn sync_watches(&mut self) {
        let desired: Vec<PathBuf> = self.folders.iter().map(|folder| PathBuf::from(&folder.path)).collect();
        let Some(watcher) = self.watcher.as_mut() else {
            self.watched = desired;
            return;
        };
        for old in &self.watched {
            if !desired.iter().any(|path| path == old) {
                let _ = watcher.unwatch(old);
            }
        }
        for path in &desired {
            if !self.watched.iter().any(|old| old == path) && path.is_dir() {
                if let Err(err) = watcher.watch(path, RecursiveMode::Recursive) {
                    self.activity = trf("Can't watch {}: {}", &[&path.display(), &err]);
                }
            }
        }
        self.watched = desired;
    }

    fn tick_slideshow(&mut self) {
        let Some(show) = self.slideshow.as_ref() else { return };
        if !show.playing || self.photos.is_empty() {
            return;
        }
        let (elapsed, interval) = (show.last.elapsed().as_secs_f32(), show.interval);
        let Some(next) = self.upcoming_slide() else { return };
        self.request_display(next);
        let ready = self.photos.get(next).is_some_and(|photo| {
            photo.is_video() || self.display_texture(photo.id).is_some() || self.display_failed.contains(&photo.id)
        });
        if elapsed < interval || (!ready && elapsed < interval + SLIDE_WAIT) {
            return;
        }
        let from = self.photos.get(self.viewer_index).map(|photo| photo.id);
        self.set_index(next, true);
        if let Some(show) = self.slideshow.as_mut() {
            show.queue.pop();
            show.fade = from.map(|id| (id, Instant::now()));
        }
    }

    /// Index of the photo the slideshow shows next: the following one, or
    /// with shuffle on, the next of the shuffled order. Every photo comes
    /// once before any comes again.
    fn upcoming_slide(&mut self) -> Option<usize> {
        if !self.slideshow_shuffle || self.photos.len() < 2 {
            return Some((self.viewer_index + 1) % self.photos.len().max(1));
        }
        let current = self.photos.get(self.viewer_index).map(|photo| photo.id);
        let show = self.slideshow.as_mut()?;
        loop {
            match show.queue.last() {
                Some(id) if Some(*id) != current && self.index_of.contains_key(id) => return self.index_of.get(id).copied(),
                Some(_) => {
                    show.queue.pop();
                }
                None => {
                    show.queue = self.photos.iter().map(|photo| photo.id).filter(|id| Some(*id) != current).collect();
                    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |time| time.as_nanos() as u64);
                    shuffle(&mut show.queue, seed);
                }
            }
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| {
                    let path = file.path();
                    if path.as_os_str().is_empty() {
                        None
                    } else {
                        Some(path.to_path_buf())
                    }
                })
                .collect()
        });
        for path in dropped {
            let folder = if path.is_dir() {
                path
            } else if crate::types::is_media_path(&path) || crate::track::is_track_path(&path) {
                path.parent().map(Path::to_path_buf).unwrap_or(path)
            } else {
                continue;
            };
            if let Err(err) = self.add_folder(&folder) {
                self.activity = err;
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // An open menu or drop-down closes on Escape itself; the press is
        // left for it rather than also leaving the viewer.
        let escape = !egui::Popup::is_any_open(ctx) && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
        if escape {
            // The folder removal is asked over any window, so it closes first.
            if self.remove_asked.take().is_some() {
                return;
            }
            if !matches!(self.modal, Modal::None) {
                if !self.renaming() {
                    self.modal = Modal::None;
                }
                return;
            }
            if self.slideshow.is_some() {
                self.slideshow = None;
                return;
            }
            if self.editing && self.mode == Mode::View {
                if !self.edit_escape() {
                    self.toggle_editing();
                }
                return;
            }
            if self.mode == Mode::View {
                self.close_viewer();
                return;
            }
            let fullscreen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
            if fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            return;
        }
        if ctx.egui_wants_keyboard_input() || !matches!(self.modal, Modal::None) || self.remove_asked.is_some() {
            return;
        }
        let command = |key| ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, key));
        if !self.editing && command(Key::F) {
            if self.mode == Mode::View {
                self.close_viewer();
            }
            self.focus_search = true;
        }
        let plain = |key| ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key));
        let shift = |key| ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, key));
        // Statistics and the Travel Map show no photos to select, open, or trash.
        let photo_keys = self.mode == Mode::View || !(matches!(self.nav, Nav::Statistics | Nav::TravelMap | Nav::Trips) || self.story_shown());

        if command(Key::A) && self.mode == Mode::Browse && photo_keys {
            self.selection.clear();
            for photo in &self.photos {
                self.selection.insert(photo.id);
            }
            self.anchor = self.photos.first().map(|photo| photo.id);
        }
        if plain(Key::F11) {
            let on = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!on));
        }
        if plain(Key::F5) {
            self.rescan();
        }
        if plain(Key::F1) || plain(Key::Questionmark) {
            self.modal = Modal::Help;
        }
        if command(Key::Comma) {
            self.open_settings(ctx);
        }
        if !photo_keys {
            return;
        }
        if Self::take_copy_command(ctx) {
            let index = match self.mode {
                Mode::Browse => self.primary_index(),
                Mode::View => Some(self.viewer_index),
            };
            if let Some(id) = index.and_then(|index| self.photos.get(index)).map(|photo| photo.id) {
                self.copy_picture(ctx, id);
            }
        }
        let command_shift = |key| ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, key));
        if command_shift(Key::C) {
            let index = match self.mode {
                Mode::Browse => self.primary_index(),
                Mode::View => Some(self.viewer_index),
            };
            let id = index.and_then(|index| self.photos.get(index)).map(|photo| photo.id);
            self.copy_settings(id);
        }
        if command_shift(Key::V) {
            let ids: Vec<i64> = match self.mode {
                Mode::Browse => self.selection.iter().copied().collect(),
                Mode::View => Vec::new(),
            };
            if self.edit.is_some() || !ids.is_empty() {
                self.paste_settings(&ids);
            }
        }
        if self.editing && self.mode == Mode::View && self.edit.is_some() {
            let redo = command_shift(Key::Z);
            if redo || command(Key::Y) {
                self.edit_redo();
            } else if command(Key::Z) {
                self.edit_undo();
            }
            // Plain shortcuts also match with Shift held, so Shift+Y goes first.
            if shift(Key::Y) {
                self.toggle_compare(editor::Compare::Split(0.5));
            } else if plain(Key::Y) {
                self.toggle_compare(editor::Compare::SideBySide);
            }
            if plain(Key::Backslash) {
                self.toggle_compare(editor::Compare::Before);
            }
            if plain(Key::J) {
                self.toggle_clipping();
            }
            if plain(Key::C) {
                self.set_tool(editor::Tool::Crop);
            }
            if plain(Key::T) {
                self.set_tool(editor::Tool::Transform);
            }
            let framing = self.edit.as_ref().filter(|session| session.uncropped()).map(|session| session.tool);
            if let Some(tool) = framing.filter(|_| plain(Key::Enter)) {
                self.set_tool(tool);
            }
        }
        if plain(Key::E) {
            if self.mode == Mode::Browse {
                if let Some(index) = self.primary_index() {
                    self.open_at(index);
                    self.editing = true;
                }
            } else if self.slideshow.is_none() {
                self.toggle_editing();
            }
        }
        if self.mode == Mode::Browse {
            if plain(Key::Enter) {
                if let Some(index) = self.primary_index() {
                    self.open_at(index);
                } else if !self.photos.is_empty() {
                    self.open_at(0);
                }
            }
            let cols = self.grid_cols.max(1) as isize;
            if plain(Key::ArrowLeft) {
                self.move_selection(-1);
            }
            if plain(Key::ArrowRight) {
                self.move_selection(1);
            }
            if plain(Key::ArrowUp) {
                self.move_selection(-cols);
            }
            if plain(Key::ArrowDown) {
                self.move_selection(cols);
            }
            if plain(Key::Delete) && !self.selection.is_empty() {
                let ids: Vec<i64> = self.selection.iter().copied().collect();
                self.modal = Modal::trash(ids);
            }
        } else {
            if plain(Key::ArrowLeft) {
                self.viewer_step(-1);
            }
            if plain(Key::ArrowRight) {
                self.viewer_step(1);
            }
            if plain(Key::Home) {
                self.viewer_step(-(self.viewer_index as isize));
            }
            if plain(Key::End) && !self.photos.is_empty() {
                self.viewer_step((self.photos.len() - 1 - self.viewer_index) as isize);
            }
            if plain(Key::M) {
                if let Some(view) = self.video.as_mut() {
                    view.player.toggle_mute();
                }
            }
            if plain(Key::Space) {
                if let Some(view) = self.video.as_mut().filter(|_| self.slideshow.is_none()) {
                    view.player.toggle();
                } else if self.slideshow.is_some() {
                    if let Some(show) = self.slideshow.as_mut() {
                        show.playing = !show.playing;
                        show.last = Instant::now();
                    }
                } else {
                    self.start_slideshow();
                }
            }
            if plain(Key::Delete) {
                if let Some(id) = self.photos.get(self.viewer_index).map(|photo| photo.id) {
                    self.modal = Modal::trash(vec![id]);
                }
            }
            if plain(Key::Plus) || plain(Key::Equals) {
                self.zoom_keys(1.25);
            }
            if plain(Key::Minus) {
                self.zoom_keys(1.0 / 1.25);
            }
            if plain(Key::Num0) {
                self.zoom = Zoom::Fit;
                self.pan = Vec2::ZERO;
            }
            if plain(Key::Num1) {
                self.zoom = Zoom::Scale(1.0 / ctx.pixels_per_point());
                self.pan = Vec2::ZERO;
            }
        }
        if self.slideshow.is_some() && shift(Key::S) {
            self.toggle_shuffle();
        }
        if plain(Key::S) {
            if self.mode == Mode::View {
                if let Some(id) = self.photos.get(self.viewer_index).map(|photo| photo.id) {
                    self.toggle_star(id);
                }
            } else {
                self.toggle_star_selection();
            }
        }
        if command(Key::CloseBracket) || plain(Key::R) {
            self.rotate_current(90);
        }
        if command(Key::OpenBracket) || shift(Key::R) {
            self.rotate_current(-90);
        }
        if plain(Key::I) {
            if self.mode == Mode::View {
                self.toggle_info();
            } else {
                self.toggle_side_panel();
            }
        }
        let _ = command;
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (rect, icon) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
            theme::paint_logo(ui.painter(), &self.logo, rect);
            let name = ui.add(egui::Label::new(egui::RichText::new("Picasso").size(17.0).strong().color(theme::text())).sense(egui::Sense::click()));
            let brand = icon.union(name).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("About Picasso"));
            if brand.clicked() {
                self.modal = Modal::About;
            }
            ui.add_space(24.0);
            let search_w = (ui.available_width() - 320.0).clamp(160.0, 420.0);
            let search_id = egui::Id::unique("search");
            self.search_keys(ui, search_id);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .id(search_id)
                    .hint_text(tr("Search photos"))
                    .margin(egui::Margin::symmetric(12, 6))
                    .desired_width(search_w),
            );
            let response = response.on_hover_text(tr(SEARCH_HINT));
            if std::mem::take(&mut self.focus_search) {
                response.request_focus();
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(self.search.chars().count()));
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), response.id);
                }
            }
            if response.gained_focus() {
                self.search_menu.typed = false;
                self.search_menu.pick = None;
            }
            if response.changed() {
                self.search_due = Some(Instant::now() + SEARCH_DELAY);
                self.search_menu.typed = true;
                self.search_menu.pick = None;
            }
            let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if let Some(search) = self.recent_searches(ui, &response, submitted) {
                self.search = search;
                self.search_due = Some(Instant::now());
            }
            if let Some(due) = self.search_due {
                let left = due.saturating_duration_since(Instant::now());
                if submitted || left.is_zero() {
                    self.search_due = None;
                    if self.nav == Nav::Places && !self.search.trim().is_empty() {
                        self.frame_after_load = true;
                    }
                    self.reload_view();
                } else {
                    ui.ctx().request_repaint_after(left);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let gear = ui.button("     ");
                theme::paint_settings_icon(ui.painter(), gear.rect.center(), theme::text());
                if gear.on_hover_text(tr("Settings")).clicked() {
                    self.open_settings(ui.ctx());
                }
                let help = ui.button("     ");
                theme::paint_help_icon(ui.painter(), help.rect.center(), theme::text());
                if help.on_hover_text(tr("Help & Shortcuts (F1)")).clicked() {
                    self.modal = Modal::Help;
                }
                if ui.button(tr("Slideshow")).on_hover_text(tr("Play Fullscreen Slideshow")).clicked() {
                    self.start_slideshow();
                }
                if ui.button(tr("Refresh")).on_hover_text(tr("Rescan folders")).clicked() {
                    self.rescan();
                }
                if ui
                    .add(theme::primary_button(tr("+  Import")))
                    .on_hover_text(tr("Add Folder to Picasso…"))
                    .clicked()
                {
                    self.modal = Modal::AddFolder {
                        path: String::new(),
                        error: String::new(),
                    };
                }
                self.update_link(ui);
            });
        });
    }

    fn set_theme_mode(&mut self, ctx: &egui::Context, mode: ThemeMode) {
        self.theme_mode = mode;
        self.config.set("theme", mode.code());
        theme::set_dark(ctx, mode.is_dark(system_dark(ctx)));
    }

    /// Flips between the light and dark themes from the viewer toolbar.
    pub(super) fn toggle_theme(&mut self, ctx: &egui::Context) {
        let mode = if theme::is_dark() { ThemeMode::Light } else { ThemeMode::Dark };
        self.set_theme_mode(ctx, mode);
    }

    /// `None` follows the operating system each time Picasso starts.
    fn set_language(&mut self, language: Option<Lang>) {
        self.language = language;
        self.config.set("language", language.map_or("auto", Lang::code));
        i18n::set(language.unwrap_or_else(i18n::system));
        self.exif.clear();
        // OpenFreeMap names are drawn in the app language.
        if self.map_source == MapSource::OpenFreeMap {
            self.reset_tiles();
        }
        self.reload();
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Statistics and the Travel Map have no side panel, and they and
            // Places have no thumbnails to size.
            let charts = matches!(self.nav, Nav::Statistics | Nav::TravelMap | Nav::Trips);
            // Laid out from the right first, so the tray gets what is left
            // and a long selection can't push these off the window.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !charts {
                    let panel = side::panel_icon(ui, self.side != SidePanel::None);
                    if panel.on_hover_text(tr("Show or hide the side panel (I)")).clicked() {
                        self.toggle_side_panel();
                    }
                }
                if !charts && self.nav != Nav::Places {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("▮").size(16.0).color(theme::muted()));
                    let mut size = self.thumb_size;
                    let response = ui.add(egui::Slider::new(&mut size, MIN_THUMB..=MAX_THUMB).show_value(false));
                    ui.label(egui::RichText::new("▮").size(10.0).color(theme::muted()));
                    self.set_thumb_size(size, &response);
                    ui.add_space(12.0);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    self.tray(ui);
                });
            });
        });
    }

    /// The photo tray: as many of the selected photos as fit beside the
    /// buttons and notes, which were measured on the last frame.
    fn tray(&mut self, ui: &mut egui::Ui) {
        const THUMB: Vec2 = Vec2::new(46.0, 34.0);
        let start = ui.cursor().min.x;
        let room = ui.available_width() - self.tray_rest;
        let fit = (room / (THUMB.x + ui.spacing().item_spacing.x)).floor().clamp(0.0, 16.0) as usize;
        ui.label(egui::RichText::new(tr("Photo Tray")).strong().size(12.0).color(theme::muted()));
        let mut rest = ui.cursor().min.x - start;
        let held: Vec<(i64, String, i32)> = self
            .tray_indices(fit)
            .into_iter()
            .filter_map(|index| self.photos.get(index))
            .map(|photo| (photo.id, photo.path.clone(), photo.rotation))
            .collect();
        let extra = self.selection.len().saturating_sub(held.len());
        for (id, path, rotation) in &held {
            self.ensure_thumb(*id, path);
            let (rect, response) = ui.allocate_exact_size(THUMB, egui::Sense::click());
            if let Some((texture, w, h)) = self.thumb_texture(*id) {
                theme::paint_image_rounded(ui.painter(), &texture, rect, theme::cover_uv(w, h), *rotation, 5.0);
            } else {
                ui.painter().rect_filled(rect, 5, theme::placeholder());
            }
            ui.painter().rect_stroke(rect, 5, egui::Stroke::new(1.0, Color32::from_black_alpha(20)), egui::StrokeKind::Inside);
            if response.double_clicked() {
                if let Some(index) = self.index_of(*id) {
                    self.open_at(index);
                }
            } else if response.clicked() {
                self.selection.clear();
                self.selection.insert(*id);
                self.anchor = Some(*id);
                self.scroll_to = Some(*id);
            }
        }
        let notes = ui.cursor().min.x;
        if extra > 0 {
            ui.label(egui::RichText::new(format!("+{extra}")).color(theme::muted()).size(12.0));
        }
        if !self.selection.is_empty() {
            if ui.button(tr("Clear")).on_hover_text(tr("Clear items from the selection")).clicked() {
                self.selection.clear();
                self.anchor = None;
            }
            if ui.button(tr("Delete")).clicked() {
                let ids: Vec<i64> = self.selection.iter().copied().collect();
                self.modal = Modal::trash(ids);
            }
            if ui.button(tr("Export…")).on_hover_text(tr("Save the selected photos with their edits")).clicked() {
                let ids = self.selection_in_order();
                self.open_export(&ids);
            }
        }
        let mut text = photo_count(self.photos.len());
        if !self.activity.is_empty() {
            text.push_str("  ·  ");
            text.push_str(&self.activity);
        }
        ui.label(egui::RichText::new(text).color(theme::muted()).size(12.0));
        let unreadable = self.unreadable_count();
        if unreadable > 0 {
            ui.label(egui::RichText::new("·").color(theme::muted()).size(12.0));
            let note = trf("{} files could not be read", &[&unreadable]);
            if ui.link(egui::RichText::new(note).size(12.0)).on_hover_text(tr("Show the files that could not be read")).clicked() {
                self.modal = Modal::Unreadable { confirm_all: false };
            }
        }
        self.export_status(ui);
        if let Some((at, files)) = &self.last_trashed {
            let left = UNDO_TRASH_FOR.saturating_sub(at.elapsed());
            if left.is_zero() {
                self.last_trashed = None;
            } else {
                let note = trf("Moved {} to trash", &[&files.len()]);
                ui.label(egui::RichText::new(format!("·  {note}")).color(theme::muted()).size(12.0));
                if ui.link(egui::RichText::new(tr("Undo")).size(12.0)).on_hover_text(tr("Put these files back where they were")).clicked() {
                    self.undo_trash();
                }
                ui.ctx().request_repaint_after(left);
            }
        }
        if self.scans > 0 {
            ui.add(egui::Spinner::new().size(12.0));
        }
        rest += ui.cursor().min.x - notes;
        // The scan's progress changes the width many times a second, so a
        // thumbnail goes as soon as it doesn't fit but only comes back once
        // a whole one does, rather than flickering.
        let slot = THUMB.x + ui.spacing().item_spacing.x;
        if rest > self.tray_rest + 0.5 || rest < self.tray_rest - slot {
            self.tray_rest = rest;
            ui.ctx().request_repaint();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            tree_section(ui, tr("Library"));
            if self.tree_row(ui, tr("Library"), Glyph::Library, self.nav == Nav::Library, None).clicked() {
                self.set_nav(Nav::Library);
            }
            if self.tree_row(ui, tr("Starred Photos"), Glyph::Star, self.nav == Nav::Starred, None).clicked() {
                self.set_nav(Nav::Starred);
            }
            if self.tree_row(ui, tr("Places"), Glyph::Pin, self.nav == Nav::Places, None).clicked() {
                self.set_nav(Nav::Places);
            }
            if self.tree_row(ui, tr("Statistics"), Glyph::Chart, self.nav == Nav::Statistics, None).clicked() {
                self.set_nav(Nav::Statistics);
            }
            if self.tree_row(ui, tr("Travel Map"), Glyph::Globe, self.nav == Nav::TravelMap, None).clicked() {
                self.set_nav(Nav::TravelMap);
            }
            let showing = matches!(self.modal, Modal::Duplicates { .. });
            let found = self.duplicate_groups().map(|groups| groups as i64);
            if self
                .tree_row(ui, tr("Duplicates"), Glyph::Copies, showing, found)
                .on_hover_text(tr("Find files with identical copies"))
                .clicked()
            {
                self.show_duplicates(ui.ctx());
            }
            tree_section(ui, tr("Albums"));
            let albums: Vec<(i64, String, i64)> = self
                .albums
                .iter()
                .map(|album| (album.id, album.name.clone(), album.count))
                .collect();
            for (id, name, count) in albums {
                let response = self.tree_row(ui, &name, Glyph::Album, self.nav == Nav::Album(id), Some(count));
                if response.clicked() {
                    self.set_nav(Nav::Album(id));
                }
                response.context_menu(|ui| {
                    if ui.button(tr("Delete album")).clicked() {
                        self.db.delete_album(id);
                        if self.nav == Nav::Album(id) {
                            self.nav = Nav::Library;
                        }
                        self.reload();
                        ui.close();
                    }
                });
            }
            if ui
                .add(egui::Button::new(egui::RichText::new(tr("+  New album")).color(theme::muted())).frame(false))
                .clicked()
            {
                self.modal = Modal::NewAlbum {
                    name: String::new(),
                    error: String::new(),
                };
            }
            self.trips_sidebar(ui);
            tree_section(ui, tr("Folders"));
            self.folder_tree_ui(ui);
        });
    }

    fn tree_row(&mut self, ui: &mut egui::Ui, label: &str, glyph: Glyph, selected: bool, count: Option<i64>) -> egui::Response {
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), egui::Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if selected {
            ui.painter().rect_filled(rect, theme::RADIUS, theme::select());
        } else if response.hovered() {
            ui.painter().rect_filled(rect, theme::RADIUS, theme::hover());
        }
        paint_glyph(ui.painter(), rect.left_center() + Vec2::new(10.0, 0.0), glyph);
        let mut text_right = rect.right() - 10.0;
        if let Some(count) = count {
            let count_rect = ui.painter().text(
                egui::pos2(rect.right() - 10.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                count.to_string(),
                egui::FontId::proportional(12.0),
                theme::muted(),
            );
            text_right = count_rect.left() - 8.0;
        }
        let clip = Rect::from_min_max(rect.min, egui::pos2(text_right, rect.max.y));
        ui.painter().with_clip_rect(clip).text(
            rect.left_center() + Vec2::new(32.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.5),
            if selected { theme::accent() } else { theme::text() },
        );
        response
    }

    /// ↑ and ↓ in the search box move through the recent searches listed
    /// under it, and Enter searches for the one picked. Taken before the box
    /// sees them, which would move its cursor.
    fn search_keys(&mut self, ui: &egui::Ui, id: egui::Id) {
        let menu = &mut self.search_menu;
        if !menu.open || menu.shown.is_empty() || !ui.memory(|mem| mem.has_focus(id)) {
            return;
        }
        let steps = ui.input_mut(|i| i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown) as isize - i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp) as isize);
        if steps != 0 {
            let at = menu.pick.map_or(-1, |pick| pick as isize) + steps;
            menu.pick = (at >= 0).then(|| (at as usize).min(menu.shown.len() - 1));
        }
        let Some(pick) = menu.pick else { return };
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.search = menu.shown[pick].clone();
            self.search_due = Some(Instant::now());
            ui.memory_mut(|mem| mem.surrender_focus(id));
        }
    }

    /// Lists the recent searches under the search box while it has the
    /// focus, and returns the one clicked. What was typed is remembered when
    /// the box loses the focus, unless a search from the list replaces it.
    fn recent_searches(&mut self, ui: &egui::Ui, search: &egui::Response, submitted: bool) -> Option<String> {
        let id = egui::Popup::default_response_id(search);
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let in_list = ui.ctx().read_response(id).zip(pointer).is_some_and(|(list, pos)| list.rect.contains(pos));
        let menu = &mut self.search_menu;
        if search.has_focus() {
            menu.open = true;
        } else if search.lost_focus() {
            // A press on the list takes the focus; the list stays until the
            // release, which is when a click on a search counts.
            menu.open = in_list && !submitted;
            if !menu.open {
                self.config.remember_search(&self.search);
            }
        } else if menu.open && ui.input(|i| i.focused && !i.pointer.any_down() && !i.pointer.any_released()) {
            menu.open = false;
            self.config.remember_search(&self.search);
        }
        let menu = &mut self.search_menu;
        menu.shown.clear();
        if !menu.open {
            menu.pick = None;
            return None;
        }
        let typed = if menu.typed { self.search.trim().to_lowercase() } else { String::new() };
        menu.shown = self
            .config
            .searches()
            .into_iter()
            .filter(|earlier| {
                let earlier = earlier.to_lowercase();
                typed.is_empty() || (earlier != typed && earlier.contains(&typed))
            })
            .collect();
        menu.pick = menu.pick.filter(|&pick| pick < menu.shown.len());
        if menu.shown.is_empty() {
            return None;
        }
        let mut picked = None;
        let mut forget = false;
        egui::Popup::from_response(search)
            .open(true)
            .align(egui::RectAlign::BOTTOM_START)
            .align_alternatives(&[])
            .gap(4.0)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .show(|ui| {
                ui.set_width(search.rect.width() - ui.spacing().menu_margin.sum().x);
                ui.label(egui::RichText::new(tr("Recent searches")).small().color(theme::muted()));
                for (index, earlier) in menu.shown.iter().enumerate() {
                    if ui.selectable_label(menu.pick == Some(index), earlier).clicked() {
                        picked = Some(earlier.clone());
                    }
                }
                ui.separator();
                if ui.selectable_label(false, egui::RichText::new(tr("Clear recent searches")).color(theme::muted())).clicked() {
                    forget = true;
                }
            });
        if forget {
            self.config.forget_searches();
            self.search_menu.open = false;
        }
        if let Some(earlier) = &picked {
            self.config.remember_search(earlier);
            self.search_menu.open = false;
        }
        picked
    }

    fn set_nav(&mut self, nav: Nav) {
        // Places keeps the selection and the search, to show where those
        // photos are; back in the Library, the same search shows them again.
        let places = matches!(nav, Nav::Places);
        let keep_search = match (&self.nav, &nav) {
            (Nav::Places, Nav::Places) => false,
            (_, Nav::Places) | (Nav::Places, Nav::Library) => true,
            _ => false,
        };
        if matches!(nav, Nav::Statistics | Nav::TravelMap | Nav::Trips) {
            self.edited_only = false;
        }
        if self.nav != nav {
            if let Nav::Trip(id) = self.nav {
                self.trip_filters.insert(id, self.trip_filter);
            }
            self.trip_filter = match nav {
                Nav::Trip(id) => self.trip_filters.get(&id).copied().unwrap_or_default(),
                _ => crate::trip::MemberQuery::default(),
            };
            // A Story that can't be saved stays open, to come back to.
            self.close_story();
        }
        self.nav = nav;
        self.mode = Mode::Browse;
        self.slideshow = None;
        self.picked_track = None;
        if !keep_search {
            self.search.clear();
            self.search_due = None;
        }
        if !places {
            self.selection.clear();
        }
        self.frame_after_load = places;
        if self.nav == Nav::TravelMap {
            self.travel.opened();
        }
        self.reload_view();
    }

    fn modals(&mut self, ctx: &egui::Context) {
        match self.modal {
            Modal::None => {}
            Modal::Help => {
                if help::show(ctx, &self.data_dir) {
                    self.modal = Modal::None;
                }
            }
            Modal::AddFolder { .. } => self.add_folder_modal(ctx),
            Modal::NewAlbum { .. } => self.new_album_modal(ctx),
            Modal::Trash { ref ids, .. } if !self.confirm_trash => {
                let ids = ids.clone();
                self.modal = Modal::None;
                self.trash_ids(&ids);
            }
            Modal::Trash { .. } => self.trash_modal(ctx),
            Modal::Unreadable { .. } => self.unreadable_modal(ctx),
            Modal::Export(_) => self.export_modal(ctx),
            Modal::AdjustDate(_) => self.adjust_date_modal(ctx),
            Modal::Geotag(_) => self.geotag_modal(ctx),
            Modal::Rename(_) => self.renamer_modal(ctx),
            Modal::Duplicates { .. } => self.duplicates_modal(ctx),
            Modal::Trip(_) => self.trip_modal(ctx),
            Modal::AddToTrip(_) => self.add_to_trip_modal(ctx),
            Modal::DeleteTrip(_) => self.delete_trip_modal(ctx),
            Modal::Unavailable(_) => self.unavailable_modal(ctx),
            Modal::DayDetails(_) => self.day_modal(ctx),
            Modal::DeleteStory(..) => self.delete_story_modal(ctx),
            Modal::ExportStory(_) => self.export_story_modal(ctx),
            Modal::Settings => self.settings_modal(ctx),
            Modal::About => {
                if about::show(ctx, &self.logo) {
                    self.modal = Modal::None;
                }
            }
        }
        if !matches!(self.modal, Modal::Settings) {
            self.settings_closed();
        }
        self.remove_folder_window(ctx);
    }

    fn remove_folder_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.remove_asked else { return };
        let Some(folder) = self.folders.iter().find(|folder| folder.id == id).cloned() else {
            self.remove_asked = None;
            return;
        };
        let mut close = false;
        let mut confirm = false;
        egui::Window::new(tr("Remove from Picasso"))
            .id(egui::Id::unique("remove-folder"))
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(trf("Remove {} from Picasso?", &[&settings::display_path(Path::new(&folder.path))]));
                let photos = photo_count(folder.count as usize);
                let note = trf(
                    "This takes {} out of the library; the files stay where they are. Stars, captions, albums, and edits are kept for 90 days and come back if you add the folder again, even from a new place.",
                    &[&photos],
                );
                ui.add(egui::Label::new(egui::RichText::new(note).size(12.5).color(theme::muted())).wrap());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::danger_button(tr("Remove from Picasso"))).clicked() {
                        confirm = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        if close || confirm {
            self.remove_asked = None;
        }
        if confirm {
            self.remove_folder(id);
        }
    }

    fn add_folder_modal(&mut self, ctx: &egui::Context) {
        let Modal::AddFolder { path, error } = &self.modal else { return };
        let mut path = path.clone();
        let error = error.clone();
        let mut close = false;
        let mut browse = false;
        let mut submit = false;
        egui::Window::new(tr("Add folder"))
            .id(egui::Id::unique("add-folder"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(tr("Picasso watches this folder and adds new photos as they appear."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut path)
                            .hint_text("/home/you/Pictures")
                            .desired_width(320.0),
                    );
                    if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        submit = true;
                    }
                    if ui.button(tr("Browse")).clicked() {
                        browse = true;
                    }
                });
                if !error.is_empty() {
                    ui.label(egui::RichText::new(&error).color(theme::DANGER));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button(tr("Add folder"))).clicked() {
                        submit = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        if browse {
            if let Some(picked) = rfd::FileDialog::new().set_title(tr("Add a photo folder")).pick_folder() {
                path = picked.display().to_string();
            }
        }
        if close {
            self.modal = Modal::None;
            return;
        }
        if submit {
            match self.add_folder(Path::new(&path)) {
                Ok(()) => self.modal = Modal::None,
                Err(err) => {
                    self.modal = Modal::AddFolder { path, error: err };
                }
            }
        } else {
            self.modal = Modal::AddFolder { path, error };
        }
    }

    fn new_album_modal(&mut self, ctx: &egui::Context) {
        let Modal::NewAlbum { name, error } = &self.modal else { return };
        let mut name = name.clone();
        let error = error.clone();
        let mut close = false;
        let mut submit = false;
        egui::Window::new(tr("New album"))
            .id(egui::Id::unique("new-album"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let response = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr("Album name")).desired_width(260.0));
                if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    submit = true;
                }
                if !error.is_empty() {
                    ui.label(egui::RichText::new(&error).color(theme::DANGER));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button(tr("Create"))).clicked() {
                        submit = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.modal = Modal::None;
            return;
        }
        if submit {
            match self.db.create_album(&name) {
                Ok(id) => {
                    let photos: Vec<i64> = self.selection.iter().copied().collect();
                    if !photos.is_empty() {
                        self.db.add_to_album(id, &photos);
                    }
                    self.nav = Nav::Album(id);
                    self.reload();
                    self.modal = Modal::None;
                }
                Err(err) => self.modal = Modal::NewAlbum { name, error: err },
            }
        } else {
            self.modal = Modal::NewAlbum { name, error };
        }
    }

    fn trash_modal(&mut self, ctx: &egui::Context) {
        let Modal::Trash { ids, bytes } = &mut self.modal else { return };
        let count = ids.len();
        let bytes = *bytes.get_or_insert_with(|| total_bytes(&self.photos, &self.db, ids));
        let mut close = false;
        let mut confirm = false;
        egui::Window::new(tr("Move to trash"))
            .id(egui::Id::unique("trash"))
            .collapsible(false)
            .resizable(false)
            .default_width(400.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let question = if count == 1 {
                    tr("Move 1 photo to the system trash?").to_string()
                } else {
                    trf("Move {} photos to the system trash?", &[&count])
                };
                ui.label(format!("{question} ({})", crate::types::format_bytes(bytes)));
                let undo = if crate::shell::CAN_RESTORE {
                    "You can undo this from the status bar for two minutes. Stars, captions, albums, and edits are kept for 90 days in case the files come back."
                } else {
                    "To undo it, put the files back from the Trash. Stars, captions, albums, and edits are kept for 90 days in case the files come back."
                };
                ui.add(egui::Label::new(egui::RichText::new(tr(undo)).size(12.5).color(theme::muted())).wrap());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::danger_button(tr("Move to trash"))).clicked() {
                        confirm = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.modal = Modal::None;
        } else if confirm {
            if let Modal::Trash { ids, .. } = std::mem::replace(&mut self.modal, Modal::None) {
                self.trash_ids(&ids);
            }
        }
    }

    fn add_folder(&mut self, path: &Path) -> Result<(), String> {
        let id = self.db.add_folder(path)?;
        let _ = self.cmd_tx.send(Command::Scan(id));
        self.nav = Nav::Folder(id);
        self.mode = Mode::Browse;
        self.reload();
        self.sync_watches();
        Ok(())
    }

    /// Drops the folder and its photos from the catalog. The files stay.
    fn remove_folder(&mut self, id: i64) {
        if let Err(err) = self.db.remove_folder(id) {
            self.activity = trf("Couldn't remove the folder: {}", &[&err]);
            return;
        }
        self.unreadable.remove(&id);
        self.tracks.retain(|track| track.folder_id != id);
        self.excluded = self.db.excluded_dirs();
        if matches!(self.nav, Nav::Folder(_) | Nav::Dir(_)) {
            self.nav = Nav::Library;
        }
        self.reload();
        self.sync_watches();
    }

    /// Leaves a subfolder out of scanning and drops its photos from the
    /// library. The files stay.
    fn exclude_dir(&mut self, dir: &Path) -> Result<(), String> {
        let (folder, dir) = self.db.exclude_dir(dir)?;
        self.excluded = self.db.excluded_dirs();
        if let Some(list) = self.unreadable.get_mut(&folder) {
            list.retain(|item| !Path::new(&item.path).starts_with(&dir));
        }
        self.tracks.retain(|track| track.folder_id != folder || !Path::new(&track.path).starts_with(&dir));
        if let Nav::Dir(ref path) = self.nav {
            if Path::new(path).starts_with(&dir) {
                self.nav = Nav::Folder(folder);
            }
        }
        self.activity = trf("Picasso no longer scans {}", &[&settings::display_path(Path::new(&dir))]);
        self.reload();
        Ok(())
    }

    fn include_dir(&mut self, dir: &str) {
        if let Some(folder) = self.db.include_dir(dir) {
            let _ = self.cmd_tx.send(Command::Scan(folder));
            self.activity = trf("Picasso scans {} again", &[&settings::display_path(Path::new(dir))]);
        }
        self.excluded = self.db.excluded_dirs();
    }

    fn rescan(&mut self) {
        let ids: Vec<i64> = match self.nav {
            Nav::Folder(id) => vec![id],
            Nav::Dir(ref path) => self.folder_for(Path::new(path)).into_iter().collect(),
            _ => self.folders.iter().map(|folder| folder.id).collect(),
        };
        for id in ids {
            let _ = self.cmd_tx.send(Command::Scan(id));
        }
    }

    fn view_title(&self) -> String {
        if !self.search.trim().is_empty() {
            return trf("Search “{}”", &[&self.search.trim()]);
        }
        match &self.nav {
            Nav::Library => tr("Library").into(),
            Nav::Starred => tr("Starred Photos").into(),
            Nav::Places => tr("Places").into(),
            Nav::Trips => tr("Trips").into(),
            Nav::Statistics => tr("Statistics").into(),
            Nav::TravelMap => tr("Travel Map").into(),
            Nav::Folder(id) => self
                .folders
                .iter()
                .find(|folder| folder.id == *id)
                .map(|folder| folder_label(&folder.path))
                .unwrap_or_else(|| tr("Folder").into()),
            Nav::Dir(path) => folder_label(path),
            Nav::Album(id) => self
                .albums
                .iter()
                .find(|album| album.id == *id)
                .map(|album| album.name.clone())
                .unwrap_or_else(|| tr("Album").into()),
            Nav::Trip(id) => self.trip_title(*id),
        }
    }

    fn select_starred(&mut self) {
        self.selection.clear();
        for photo in &self.photos {
            if photo.starred {
                self.selection.insert(photo.id);
            }
        }
        self.anchor = self.selection.iter().next().copied();
    }

    fn open_at(&mut self, index: usize) {
        if self.photos.is_empty() {
            return;
        }
        self.mode = Mode::View;
        self.set_index(index, true);
        let id = self.photos[self.viewer_index].id;
        self.browse_selection = Some((std::mem::take(&mut self.selection), self.anchor));
        self.selection.clear();
        self.selection.insert(id);
        self.anchor = Some(id);
    }

    fn close_viewer(&mut self) {
        self.mode = Mode::Browse;
        // The grid gets back its selection if the photo last shown was part of
        // it; otherwise that photo alone. Photos gone meanwhile are dropped.
        if let (Some((selection, anchor)), Some(shown)) = (self.browse_selection.take(), self.photos.get(self.viewer_index)) {
            let shown = shown.id;
            if selection.contains(&shown) {
                self.selection = selection.into_iter().filter(|id| self.index_of(*id).is_some()).collect();
                self.anchor = anchor.filter(|id| self.selection.contains(id)).or(Some(shown));
            }
            self.scroll_to = Some(shown);
        }
        self.film_shown = None;
        self.slideshow = None;
        self.editing = false;
        self.end_edit_session();
    }

    fn set_index(&mut self, index: usize, reset_zoom: bool) {
        if self.photos.is_empty() {
            return;
        }
        let index = index.min(self.photos.len() - 1);
        let changed = index != self.viewer_index;
        self.viewer_index = index;
        if reset_zoom {
            self.zoom = Zoom::Fit;
            self.pan = Vec2::ZERO;
        }
        if changed {
            self.sync_caption();
        }
        self.request_neighbors();
        // Only the slideshow's own step fades; any other move cuts to the photo.
        if let Some(show) = self.slideshow.as_mut() {
            show.last = Instant::now();
            show.fade = None;
        }
    }

    fn viewer_step(&mut self, delta: isize) {
        if self.photos.is_empty() {
            return;
        }
        let next = (self.viewer_index as isize + delta).clamp(0, self.photos.len() as isize - 1) as usize;
        self.set_index(next, true);
        let id = self.photos[next].id;
        self.selection.clear();
        self.selection.insert(id);
        self.anchor = Some(id);
        self.scroll_to = Some(id);
    }

    fn select_click(&mut self, index: usize, modifiers: Modifiers) {
        let Some(id) = self.photos.get(index).map(|photo| photo.id) else {
            return;
        };
        if modifiers.shift {
            let anchor = self.anchor.unwrap_or(id);
            if let (Some(start), Some(end)) = (self.index_of(anchor), Some(index)) {
                let (lo, hi) = (start.min(end), start.max(end));
                self.selection.clear();
                for photo in &self.photos[lo..=hi] {
                    self.selection.insert(photo.id);
                }
            }
        } else if modifiers.command {
            if !self.selection.insert(id) {
                self.selection.remove(&id);
            }
            self.anchor = Some(id);
        } else {
            self.selection.clear();
            self.selection.insert(id);
            self.anchor = Some(id);
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.photos.is_empty() {
            return;
        }
        let current = self.primary_index().unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.photos.len() as isize - 1) as usize;
        let id = self.photos[next].id;
        self.selection.clear();
        self.selection.insert(id);
        self.anchor = Some(id);
        self.scroll_to = Some(id);
    }

    fn primary_index(&self) -> Option<usize> {
        if let Some(anchor) = self.anchor.filter(|anchor| self.selection.contains(anchor)) {
            if let Some(index) = self.index_of(anchor) {
                return Some(index);
            }
        }
        self.selection.iter().filter_map(|id| self.index_of(*id)).min()
    }

    fn primary_starred(&self) -> bool {
        self.primary_index()
            .and_then(|index| self.photos.get(index))
            .map(|photo| photo.starred)
            .unwrap_or(false)
    }

    fn toggle_star(&mut self, id: i64) {
        if let Some(photo) = self.index_of(id).and_then(|index| self.photos.get_mut(index)) {
            photo.starred = !photo.starred;
            let starred = photo.starred;
            self.db.set_star(id, starred);
            if self.starred_only() && !starred {
                self.drop_from_view(&[id]);
            }
        }
    }

    /// The view shows only starred photos, so unstarring takes one out.
    fn starred_only(&self) -> bool {
        self.nav == Nav::Starred || (self.nav == Nav::Places && self.map_starred)
    }

    fn toggle_star_selection(&mut self) {
        if let Some(index) = self.primary_index() {
            self.star_from_menu(self.photos[index].id);
        }
    }

    /// Uses the primary/clicked photo's state for the whole selection.
    fn star_from_menu(&mut self, id: i64) {
        if let Some((ids, make)) = star_change(&self.photos, &self.selection, id) {
            self.set_stars(&ids, make);
        }
    }

    fn set_stars(&mut self, ids: &[i64], make: bool) {
        if ids.is_empty() {
            return;
        }
        self.db.set_stars(ids, make);
        if self.starred_only() && !make {
            self.drop_from_view(ids);
            return;
        }
        for id in ids {
            if let Some(photo) = self.index_of.get(id).and_then(|&index| self.photos.get_mut(index)) {
                photo.starred = make;
            }
        }
    }

    /// Rotates the photo on screen in the viewer, or the selection in the
    /// grid.
    fn rotate_current(&mut self, delta: i32) {
        let ids: Vec<i64> = if self.mode == Mode::View {
            self.photos.get(self.viewer_index).map(|photo| vec![photo.id]).unwrap_or_default()
        } else {
            self.selection.iter().copied().collect()
        };
        self.rotate_ids(&ids, delta);
    }

    /// A context-menu rotate applies to the whole selection when the clicked
    /// photo is part of it, otherwise to that photo alone.
    fn rotate_from_menu(&mut self, id: i64, delta: i32) {
        let ids: Vec<i64> = if self.selection.contains(&id) {
            self.selection.iter().copied().collect()
        } else {
            vec![id]
        };
        self.rotate_ids(&ids, delta);
    }

    /// Stores the new rotation in the catalog. Cached thumbnails and decoded
    /// views are reused as they are and turned while drawing.
    fn rotate_ids(&mut self, ids: &[i64], delta: i32) {
        if ids.is_empty() {
            return;
        }
        self.db.rotate(ids, delta);
        let turned: HashSet<i64> = ids.iter().copied().collect();
        for photo in self.photos.iter_mut().filter(|photo| turned.contains(&photo.id)) {
            photo.rotation = turn(photo.rotation, delta);
        }
        if self.mode == Mode::View {
            self.zoom = Zoom::Fit;
            self.pan = Vec2::ZERO;
        }
        let text = match (ids.len() == 1, delta > 0) {
            (true, true) => "Rotated {} photo clockwise",
            (false, true) => "Rotated {} photos clockwise",
            (true, false) => "Rotated {} photo counterclockwise",
            (false, false) => "Rotated {} photos counterclockwise",
        };
        self.activity = trf(text, &[&ids.len()]);
    }

    fn remove_selection_from_album(&mut self) {
        let Nav::Album(album) = self.nav else { return };
        let ids: Vec<i64> = self.selection.iter().copied().collect();
        self.db.remove_from_album(album, &ids);
        self.selection.clear();
        self.drop_from_view(&ids);
        self.reload();
    }

    /// Moves photos to the system trash; returns those that went.
    fn trash_ids(&mut self, ids: &[i64]) -> Vec<i64> {
        let photos: Vec<crate::types::Photo> = ids.iter().filter_map(|id| self.db.photo(*id)).collect();
        if photos.is_empty() {
            return Vec::new();
        }
        let before: Vec<bool> = photos.iter().map(|photo| Path::new(&photo.path).exists()).collect();
        let present: Vec<&str> = photos.iter().zip(&before).filter(|(_, there)| **there).map(|(photo, _)| photo.path.as_str()).collect();
        // One call for all files: the system trash is slow file by file.
        let result = if present.is_empty() { Ok(()) } else { trash::delete_all(present) };
        let after: Vec<bool> = photos.iter().map(|photo| Path::new(&photo.path).exists()).collect();
        let outcome = trash_outcome(&before, &after);
        let mut notes = Vec::new();
        if let [only] = outcome.failed[..] {
            let reason = result.as_ref().err().map_or_else(|| tr("it is still there").to_string(), ToString::to_string);
            notes.push(trf("Couldn't trash {}: {}", &[&photos[only].filename, &reason]));
        } else if !outcome.failed.is_empty() {
            let reason = result.as_ref().err().map_or_else(|| tr("they are still there").to_string(), ToString::to_string);
            notes.push(trf("Couldn't move {} photos to the trash: {}", &[&outcome.failed.len(), &reason]));
        }
        if let [only] = outcome.missing[..] {
            notes.push(trf("{} wasn't found, so it stays in the library. Is its drive connected?", &[&photos[only].filename]));
        } else if !outcome.missing.is_empty() {
            notes.push(trf("{} files weren't found, so they stay in the library. Is their drive connected?", &[&outcome.missing.len()]));
        }
        self.activity = notes.join(" · ");
        let mut removed = Vec::new();
        let mut trashed = Vec::new();
        for index in outcome.trashed {
            let photo = &photos[index];
            removed.push(photo.id);
            trashed.push((photo.path.clone(), photo.folder_id));
        }
        if !removed.is_empty() {
            self.db.delete_photos(&removed);
            if crate::shell::CAN_RESTORE {
                // The status bar says so next to its Undo button.
                self.last_trashed = Some((Instant::now(), trashed));
            } else if notes.is_empty() {
                self.activity = trf("Moved {} to trash", &[&removed.len()]);
            }
            self.drop_from_view(&removed);
            self.reload();
        }
        removed
    }

    /// Puts the latest batch moved to the trash back, then rescans its
    /// folders, which gives the photos their stars and edits back.
    fn undo_trash(&mut self) {
        let Some((_, trashed)) = self.last_trashed.take() else { return };
        let paths: Vec<String> = trashed.iter().map(|(path, _)| path.clone()).collect();
        match crate::shell::restore(&paths) {
            Ok(count) => self.activity = trf("Put {} back from the trash", &[&count]),
            Err(err) => self.activity = trf("Couldn't restore from the trash: {}", &[&err]),
        }
        let folders: HashSet<i64> = trashed.iter().map(|(_, folder)| *folder).collect();
        for folder in folders {
            let _ = self.cmd_tx.send(Command::Scan(folder));
        }
    }

    fn start_slideshow(&mut self) {
        if self.photos.is_empty() {
            return;
        }
        if self.mode == Mode::Browse {
            let index = self.primary_index().unwrap_or(0);
            self.open_at(index);
        }
        self.slideshow = Some(Slideshow {
            playing: true,
            interval: self.slideshow_interval,
            last: Instant::now(),
            fade: None,
            queue: Vec::new(),
        });
    }

    /// Turns the slideshow's shuffle on or off; the choice is remembered.
    fn toggle_shuffle(&mut self) {
        self.slideshow_shuffle = !self.slideshow_shuffle;
        self.config.set("slideshow_shuffle", self.slideshow_shuffle);
        if let Some(show) = self.slideshow.as_mut() {
            show.queue.clear();
        }
    }

    /// Shows or hides the details, histogram, and EXIF panel. The choice is
    /// remembered between sessions.
    fn toggle_info(&mut self) {
        self.show_info = !self.show_info;
        self.db.set_setting("viewer_info", if self.show_info { "1" } else { "0" });
    }

    fn ensure_thumb(&mut self, id: i64, path: &str) {
        self.thumb_drawn.insert(id);
        if self.thumbs.map.contains_key(&id) || self.thumb_waiting.contains(&id) || self.thumb_failed.contains(&id) {
            return;
        }
        self.thumb_waiting.insert(id);
        // Wanted before the frame ends, or a decoder could drop it at once.
        if let Ok(mut wanted) = self.thumb_wanted.lock() {
            wanted.insert(id);
        }
        let _ = self.urgent_tx.send(ThumbJob::Thumb {
            id,
            path: path.to_string(),
            edit: self.edit_settings(id),
            edge: worker::THUMB_EDGE,
        });
    }

    /// Asks for a sharper copy of a thumbnail drawn `pixels` wide when the
    /// one shown would be stretched. Nothing is asked while the pointer is
    /// held, so dragging the size slider doesn't queue a decode per step.
    fn sharpen_thumb(&mut self, id: i64, path: &str, pixels: f32, held: bool) {
        const STEP: u32 = 128;
        const MAX_EDGE: u32 = 1024;
        let edit = self.edit_settings(id);
        let Some(tex) = self.thumbs.map.get_mut(&id) else { return };
        if held || pixels <= tex.w.min(tex.h) as f32 * 1.15 {
            return;
        }
        let edge = ((pixels / STEP as f32).ceil() as u32 * STEP).min(MAX_EDGE);
        if tex.asked >= edge {
            return;
        }
        tex.asked = edge;
        if let Ok(mut wanted) = self.thumb_wanted.lock() {
            wanted.insert(id);
        }
        let _ = self.urgent_tx.send(ThumbJob::Thumb { id, path: path.to_string(), edit, edge });
    }

    fn thumb_texture(&mut self, id: i64) -> Option<(TextureHandle, usize, usize)> {
        self.thumbs.get(id)
    }

    /// Hands this frame's thumbnails to the decoders, which drop queued
    /// requests for any others.
    fn publish_wanted_thumbs(&mut self) {
        self.thumbs.keep = self.thumb_drawn.len();
        if let Ok(mut wanted) = self.thumb_wanted.lock() {
            std::mem::swap(&mut *wanted, &mut self.thumb_drawn);
        }
        self.thumb_drawn.clear();
    }

    fn request_neighbors(&mut self) {
        if self.photos.is_empty() {
            return;
        }
        let index = self.viewer_index;
        self.request_display(index);
        if index > 0 {
            self.request_display(index - 1);
        }
        if index + 1 < self.photos.len() {
            self.request_display(index + 1);
        }
    }

    fn request_display(&mut self, index: usize) {
        let Some(photo) = self.photos.get(index) else { return };
        if self.displays.iter().any(|item| item.id == photo.id)
            || self.display_inflight.contains_key(&photo.id)
            || self.display_failed.contains(&photo.id)
        {
            return;
        }
        self.nonce_seq = self.nonce_seq.wrapping_add(1);
        let nonce = self.nonce_seq;
        let id = photo.id;
        self.display_nonce.insert(id, nonce);
        self.display_inflight.insert(id, nonce);
        if let Ok(mut gate) = self.display_gate.lock() {
            gate.insert(id, nonce);
        }
        let edit = self.edit_settings(id);
        let _ = self.urgent_tx.send(ThumbJob::Display {
            id,
            path: photo.path.clone(),
            nonce,
            edit,
            edge: worker::DISPLAY_EDGE,
        });
    }

    /// Asks for photo `index` decoded at full resolution, up to the largest
    /// texture the GPU takes, once it is drawn larger than its view.
    fn request_sharp(&mut self, index: usize, drawn: Vec2, max_side: usize) {
        let Some(photo) = self.photos.get(index) else { return };
        if self.sharp == Some(photo.id) || crate::types::is_video_path(Path::new(&photo.path)) {
            return;
        }
        let Some((_, w, h)) = self.display_texture(photo.id) else { return };
        let have = w.max(h) as f32;
        let edge = photo.width.max(photo.height).min(max_side as u32);
        if drawn.max_elem() <= have * 1.02 || edge as f32 <= have * 1.02 {
            return;
        }
        let (id, path) = (photo.id, photo.path.clone());
        self.sharp = Some(id);
        self.nonce_seq = self.nonce_seq.wrapping_add(1);
        let nonce = self.nonce_seq;
        self.display_nonce.insert(id, nonce);
        self.display_inflight.insert(id, nonce);
        if let Ok(mut gate) = self.display_gate.lock() {
            gate.insert(id, nonce);
        }
        let edit = self.edit_settings(id);
        let _ = self.urgent_tx.send(ThumbJob::Display { id, path, nonce, edit, edge });
    }

    /// Frees the full-resolution view unless it belongs to `keep`. The
    /// photo decodes at the usual size when shown again.
    fn drop_sharp(&mut self, keep: Option<i64>) {
        let Some(id) = self.sharp.filter(|id| Some(*id) != keep) else { return };
        self.sharp = None;
        self.displays.retain(|item| item.id != id);
        self.display_nonce.remove(&id);
        self.display_inflight.remove(&id);
        if let Ok(mut gate) = self.display_gate.lock() {
            gate.remove(&id);
        }
    }

    /// Histogram and EXIF rows decoded with the full-size view of `id`.
    fn display_info(&self, id: i64) -> Option<PhotoInfo> {
        self.displays
            .iter()
            .rev()
            .find(|item| item.id == id)
            .map(|item| ((*item.histogram).clone(), item.exif.clone()))
    }

    /// The info panel's histogram and EXIF for photo `id`, or for
    /// [`PANEL_HOLD`] after a change the previous photo's while `id`'s load.
    fn panel_info(&mut self, id: i64, ctx: &egui::Context) -> Option<PhotoInfo> {
        if let Some(info) = self.display_info(id) {
            self.info_held = Some((id, info.clone(), None));
            return Some(info);
        }
        let (held_id, info, left) = self.info_held.as_mut()?;
        if *held_id == id {
            return None;
        }
        let left = *left.get_or_insert_with(Instant::now);
        let remaining = PANEL_HOLD.checked_sub(left.elapsed())?;
        ctx.request_repaint_after(remaining);
        Some(info.clone())
    }

    fn display_texture(&self, id: i64) -> Option<(TextureHandle, usize, usize)> {
        self.displays
            .iter()
            .rev()
            .find(|item| item.id == id)
            .map(|item| (item.texture.clone(), item.w, item.h))
    }
}

impl eframe::App for PicassoApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.keep_track_edits();
        self.save_story();
        let _ = self.cmd_tx.send(Command::Stop);
        if let Some(id) = self.library_top {
            self.config.set("library_top", id.to_string());
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(stats) = &mut self.frame_stats {
            stats.record(frame.info().cpu_usage);
        }
        self.autosave_story(ctx);
        let dark = self.theme_mode.is_dark(system_dark(ctx));
        if dark != theme::is_dark() {
            theme::set_dark(ctx, dark);
        }
        self.pump(ctx);
        self.pump_tiles(ctx);
        self.pump_watch(ctx);
        self.pump_update(ctx);
        self.tick_slideshow();
        let busy = self.scans > 0
            || self.slideshow.as_ref().map(|show| show.playing).unwrap_or(false)
            || !self.thumb_waiting.is_empty()
            || !self.display_inflight.is_empty();
        if busy {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.track_editor.is_some() {
            self.video = None;
            self.end_edit_session();
            let ctx = ui.ctx().clone();
            let helping = matches!(self.modal, Modal::Help);
            if !helping && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::F1) || input.consume_key(Modifiers::NONE, Key::Questionmark)) {
                self.modal = Modal::Help;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::bg()))
                .show(ui, |ui| self.track_editor_ui(ui));
            if matches!(self.modal, Modal::Help) && help::show(&ctx, &self.data_dir) {
                self.modal = Modal::None;
            }
            self.publish_wanted_thumbs();
            return;
        }
        if self.track_recovery.is_some() {
            self.track_recovery_ui(ui.ctx());
        } else {
            self.handle_keys(ui.ctx());
        }
        self.handle_drops(ui.ctx());
        let viewing = self.mode == Mode::View || self.slideshow.is_some();
        if !viewing {
            self.video = None;
            self.end_edit_session();
            self.drop_sharp(None);
        }
        if viewing {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::stage()))
                .show(ui, |ui| self.viewer_ui(ui));
        } else {
            egui::Panel::top("header")
                .frame(theme::toolbar_frame(theme::toolbar()).inner_margin(egui::Margin::symmetric(16, 10)))
                .show(ui, |ui| self.header(ui));
            egui::Panel::bottom("tray")
                .frame(theme::toolbar_frame(theme::tray()))
                .show(ui, |ui| self.status(ui));
            egui::Panel::left("nav")
                .resizable(true)
                .default_size(232.0)
                .min_size(190.0)
                .max_size(340.0)
                .frame(theme::toolbar_frame(theme::sidebar()).inner_margin(egui::Margin::symmetric(10, 6)))
                .show(ui, |ui| self.sidebar(ui));
            if self.side != SidePanel::None && !matches!(self.nav, Nav::Statistics | Nav::TravelMap | Nav::Trips) && !self.story_shown() {
                egui::Panel::right("side")
                    .resizable(true)
                    .default_size(300.0)
                    .min_size(250.0)
                    .max_size(520.0)
                    .frame(theme::toolbar_frame(theme::sidebar()).inner_margin(egui::Margin::symmetric(14, 10)))
                    .show(ui, |ui| self.side_ui(ui));
            }
            egui::CentralPanel::default()
                .frame(theme::frame(theme::bg()))
                .show(ui, |ui| self.library_ui(ui));
        }
        self.modals(ui.ctx());
        self.publish_wanted_thumbs();
    }
}

/// Total size of the photos `ids`, taken from the loaded view where they are
/// in it, so a large selection doesn't query the library once per photo.
fn total_bytes(photos: &[Photo], db: &Database, ids: &[i64]) -> u64 {
    let loaded: HashMap<i64, u64> = photos.iter().map(|photo| (photo.id, photo.bytes)).collect();
    ids.iter()
        .map(|id| loaded.get(id).copied().or_else(|| db.photo(*id).map(|photo| photo.bytes)).unwrap_or(0))
        .sum()
}

/// Photos sent to the trash, by index, sorted by whether each file was there
/// before and after.
#[derive(Debug, Default, PartialEq)]
struct TrashOutcome {
    trashed: Vec<usize>,
    /// Still there: the trash refused it.
    failed: Vec<usize>,
    /// Gone before the trash was asked, as when its drive is disconnected.
    /// It wasn't trashed, so it stays in the library.
    missing: Vec<usize>,
}

fn trash_outcome(before: &[bool], after: &[bool]) -> TrashOutcome {
    let mut outcome = TrashOutcome::default();
    for (index, (&was, &is)) in before.iter().zip(after).enumerate() {
        match (was, is) {
            (false, _) => outcome.missing.push(index),
            (true, true) => outcome.failed.push(index),
            (true, false) => outcome.trashed.push(index),
        }
    }
    outcome
}

fn upload(ctx: &egui::Context, name: &str, w: usize, h: usize, rgba: &[u8]) -> Option<TextureHandle> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba[..w * h * 4]);
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

/// True when the operating system asks for dark mode. Unknown counts as light.
fn system_dark(ctx: &egui::Context) -> bool {
    ctx.system_theme() == Some(egui::Theme::Dark)
}

/// Puts `items` in a random order drawn from `seed` (Fisher–Yates over an
/// xorshift generator; good enough for a slideshow).
fn shuffle<T>(items: &mut [T], seed: u64) {
    let mut state = seed | 1;
    for last in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        items.swap(last, (state % (last as u64 + 1)) as usize);
    }
}

/// "1 photo" or "N photos" in the current language.
fn photo_count(count: usize) -> String {
    trf(if count == 1 { "{} photo" } else { "{} photos" }, &[&count])
}

fn tree_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(title.to_uppercase()).size(11.0).strong().color(theme::muted()));
    });
    ui.add_space(4.0);
}

#[derive(Clone, Copy)]
enum Glyph {
    Library,
    Album,
    Star,
    Pin,
    Copies,
    Chart,
    Globe,
    Trip,
}

fn paint_glyph(painter: &egui::Painter, center_left: egui::Pos2, glyph: Glyph) {
    let origin = center_left + Vec2::new(0.0, -7.0);
    match glyph {
        Glyph::Trip => {
            // A suitcase.
            painter.rect_filled(Rect::from_min_size(origin + Vec2::new(0.0, 4.0), Vec2::new(14.0, 10.0)), 3, theme::GOLD);
            painter.rect_stroke(
                Rect::from_min_size(origin + Vec2::new(4.5, 0.5), Vec2::new(5.0, 4.0)),
                1,
                egui::Stroke::new(1.5, theme::GOLD),
                egui::StrokeKind::Inside,
            );
        }
        Glyph::Album => {
            painter.rect_filled(Rect::from_min_size(origin + Vec2::new(3.0, 0.0), Vec2::new(11.0, 11.0)), 3, theme::ALBUM.gamma_multiply(0.45));
            painter.rect_filled(Rect::from_min_size(origin + Vec2::new(0.0, 3.0), Vec2::new(11.0, 11.0)), 3, theme::ALBUM);
        }
        Glyph::Star => {
            painter.text(
                origin + Vec2::new(7.0, 7.0),
                egui::Align2::CENTER_CENTER,
                "★",
                egui::FontId::proportional(14.0),
                theme::STAR,
            );
        }
        Glyph::Library => {
            let tile = Vec2::splat(6.0);
            for (dx, dy) in [(0.0, 0.0), (8.0, 0.0), (0.0, 8.0), (8.0, 8.0)] {
                painter.rect_filled(Rect::from_min_size(origin + Vec2::new(dx, dy), tile), 2, theme::accent());
            }
        }
        Glyph::Pin => {
            painter.circle_filled(origin + Vec2::new(7.0, 6.0), 4.5, theme::PIN);
            painter.circle_filled(origin + Vec2::new(7.0, 6.0), 1.6, Color32::WHITE);
        }
        Glyph::Copies => {
            let stroke = egui::Stroke::new(1.5, theme::muted());
            let back = Rect::from_min_size(origin + Vec2::new(4.0, 0.0), Vec2::new(10.0, 10.0));
            painter.rect_stroke(back, 2, stroke, egui::StrokeKind::Inside);
            let front = Rect::from_min_size(origin + Vec2::new(0.0, 4.0), Vec2::new(10.0, 10.0));
            painter.rect_filled(front, 2, theme::muted());
        }
        Glyph::Chart => {
            for (n, h) in [6.0, 13.0, 9.0].into_iter().enumerate() {
                let left = origin.x + 1.0 + n as f32 * 4.5;
                let bar = Rect::from_min_max(egui::pos2(left, origin.y + 14.0 - h), egui::pos2(left + 3.5, origin.y + 14.0));
                painter.rect_filled(bar, 1, theme::GREEN);
            }
        }
        Glyph::Globe => {
            let center = origin + Vec2::new(7.0, 7.0);
            let line = egui::Stroke::new(1.2, Color32::WHITE);
            painter.circle_filled(center, 6.5, theme::GOLD);
            painter.line_segment([center - Vec2::new(6.0, 0.0), center + Vec2::new(6.0, 0.0)], line);
            painter.add(egui::Shape::ellipse_stroke(center, Vec2::new(2.6, 6.0), line));
        }
    }
}

/// Resolves a star action from the same reference photo used by its label.
/// Right-clicking outside the selection affects only the clicked photo.
fn star_change(photos: &[Photo], selection: &HashSet<i64>, reference: i64) -> Option<(Vec<i64>, bool)> {
    let photo = photos.iter().find(|photo| photo.id == reference)?;
    let ids = if selection.contains(&reference) {
        photos.iter().filter(|photo| selection.contains(&photo.id)).map(|photo| photo.id).collect()
    } else {
        vec![reference]
    };
    Some((ids, !photo.starred))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_that_left_count_as_trashed() {
        let outcome = trash_outcome(&[true, true, false, false], &[false, true, false, false]);
        assert_eq!(outcome, TrashOutcome { trashed: vec![0], failed: vec![1], missing: vec![2, 3] });
        assert!(trash_outcome(&[], &[]).trashed.is_empty());
    }

    #[test]
    fn a_file_on_a_disconnected_drive_is_never_reported_as_trashed() {
        let dir = std::env::temp_dir().join(format!("picasso-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let there = dir.join("there.jpg");
        std::fs::write(&there, b"x").unwrap();
        let gone = dir.join("unplugged").join("gone.jpg");
        let paths = [there.as_path(), gone.as_path()];
        let before: Vec<bool> = paths.iter().map(|path| path.exists()).collect();
        // Stands in for the trash, which here removes only the file that was there.
        std::fs::remove_file(&there).unwrap();
        let after: Vec<bool> = paths.iter().map(|path| path.exists()).collect();
        assert_eq!(trash_outcome(&before, &after), TrashOutcome { trashed: vec![0], failed: vec![], missing: vec![1] });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shuffle_keeps_every_item_once_and_changes_the_order() {
        let mut items: Vec<u32> = (0..50).collect();
        shuffle(&mut items, 12_345);
        assert_ne!(items, (0..50).collect::<Vec<_>>());
        let mut sorted = items.clone();
        sorted.sort();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
        let mut again: Vec<u32> = (0..50).collect();
        shuffle(&mut again, 54_321);
        assert_ne!(items, again, "another seed gives another order");
    }
}
