//! Picasso application state: library navigation, viewer, and background jobs.

mod library;
mod places;
mod theme;
mod viewer;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use eframe::egui::{self, Color32, Key, Modifiers, Rect, TextureHandle, TextureOptions, Vec2};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::db::{self, Database};
use crate::types::{folder_label, turn, Nav, Photo, Sort};
use crate::worker::{self, Command, DisplayGate, ThumbJob, UiEvent};

use library::CachedLayout;
use theme::{MUTED, TEXT, TOOLBAR, TRAY, VIEWER_BG};

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
}

enum Modal {
    None,
    AddFolder { path: String, error: String },
    NewAlbum { name: String, error: String },
    Trash { ids: Vec<i64> },
    Help,
}

struct ThumbTex {
    texture: TextureHandle,
    w: usize,
    h: usize,
}

struct ThumbCache {
    map: HashMap<i64, ThumbTex>,
    order: VecDeque<i64>,
}

impl ThumbCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&mut self, id: i64) -> Option<(TextureHandle, usize, usize)> {
        let tex = self.map.get(&id)?;
        let out = (tex.texture.clone(), tex.w, tex.h);
        if let Some(pos) = self.order.iter().position(|item| *item == id) {
            self.order.remove(pos);
            self.order.push_back(id);
        }
        Some(out)
    }

    fn insert(&mut self, id: i64, tex: ThumbTex) {
        if !self.map.contains_key(&id) {
            self.order.push_back(id);
        }
        self.map.insert(id, tex);
        while self.order.len() > 360 {
            if let Some(old) = self.order.pop_front() {
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
    data_dir: PathBuf,
    cmd_tx: Sender<Command>,
    urgent_tx: Sender<ThumbJob>,
    events: Receiver<UiEvent>,
    display_gate: DisplayGate,
    watcher: Option<RecommendedWatcher>,
    watch_rx: Receiver<Result<Event, notify::Error>>,
    watched: Vec<PathBuf>,
    watch_deadline: HashMap<i64, Instant>,

    folders: Vec<crate::types::Folder>,
    albums: Vec<crate::types::Album>,
    photos: Vec<Photo>,
    nav: Nav,
    sort: Sort,
    search: String,
    mode: Mode,
    viewer_index: usize,
    selection: HashSet<i64>,
    anchor: Option<i64>,
    grid_cols: usize,
    thumb_size: f32,
    scroll_to: Option<i64>,
    view_epoch: u64,
    layout: Option<CachedLayout>,

    thumbs: ThumbCache,
    thumb_waiting: HashSet<i64>,
    thumb_failed: HashSet<i64>,
    displays: Vec<DisplayTex>,
    display_nonce: HashMap<i64, u64>,
    display_inflight: HashMap<i64, u64>,
    nonce_seq: u64,

    zoom: Zoom,
    pan: Vec2,
    stage_rect: Rect,
    show_info: bool,
    caption_buf: String,
    caption_photo: Option<i64>,
    slideshow: Option<Slideshow>,
    activity: String,
    scans: i32,
    /// The catalog changed in the background and the view should be re-read.
    library_dirty: bool,
    last_reload: Instant,
    modal: Modal,
    map_lat: f64,
    map_lon: f64,
    map_zoom: u8,
    map_pan: Vec2,
    map_place: bool,
    tiles: HashMap<(u8, u32, u32), TextureHandle>,
    tile_tx: Sender<crate::geo::TileJob>,
    tile_rx: Receiver<crate::geo::TileDone>,
    tile_wait: HashSet<(u8, u32, u32)>,
    tile_fail: HashSet<(u8, u32, u32)>,
}

impl PicassoApp {
    pub fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Self {
        theme::apply(ctx);
        let (data_dir, cache) = db::app_dirs();
        let _ = std::fs::create_dir_all(&data_dir);
        let _ = std::fs::create_dir_all(cache.join("thumbs"));
        let _ = std::fs::create_dir_all(cache.join("tiles"));
        // Face crops from older builds are no longer used.
        let _ = std::fs::remove_dir_all(cache.join("faces"));
        let database = Database::open(&data_dir.join("picasso.db")).expect("open Picasso library");
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (urgent_tx, urgent_rx) = crossbeam_channel::unbounded();
        let (normal_tx, normal_rx) = crossbeam_channel::unbounded();
        let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
        let display_gate = Arc::new(Mutex::new(HashMap::new()));
        for index in 0..worker::thumb_threads() {
            let ctx = ctx.clone();
            let cache = cache.clone();
            let gate = display_gate.clone();
            let events = ev_tx.clone();
            let urgent = urgent_rx.clone();
            let normal = normal_rx.clone();
            std::thread::Builder::new()
                .name(format!("picasso-thumbs-{index}"))
                .spawn(move || worker::thumb_loop(urgent, normal, events, ctx, cache, gate))
                .expect("thumb thread");
        }
        drop((urgent_rx, normal_rx));
        {
            let ctx = ctx.clone();
            let db_path = data_dir.join("picasso.db");
            let cache = cache.clone();
            std::thread::Builder::new()
                .name("picasso-index".into())
                .spawn(move || worker::index_loop(db_path, cache, cmd_rx, normal_tx, ev_tx, ctx))
                .expect("index thread");
        }
        let (tile_tx, tile_job_rx) = crossbeam_channel::unbounded();
        let (tile_done_tx, tile_rx) = crossbeam_channel::unbounded();
        {
            let cache = cache.clone();
            std::thread::Builder::new()
                .name("picasso-tiles".into())
                .spawn(move || crate::geo::tile_loop(tile_job_rx, tile_done_tx, cache))
                .expect("tile thread");
        }
        let (watch_tx, watch_rx) = crossbeam_channel::unbounded();
        let watcher = notify::recommended_watcher(move |result| {
            let _ = watch_tx.send(result);
        })
        .ok();

        let mut app = Self {
            db: database,
            data_dir,
            cmd_tx,
            urgent_tx,
            events: ev_rx,
            display_gate,
            watcher,
            watch_rx,
            watched: Vec::new(),
            watch_deadline: HashMap::new(),
            folders: Vec::new(),
            albums: Vec::new(),
            photos: Vec::new(),
            nav: Nav::Library,
            sort: Sort::DateNewest,
            search: String::new(),
            mode: Mode::Browse,
            viewer_index: 0,
            selection: HashSet::new(),
            anchor: None,
            grid_cols: 1,
            thumb_size: 144.0,
            scroll_to: None,
            view_epoch: 0,
            layout: None,
            thumbs: ThumbCache::new(),
            thumb_waiting: HashSet::new(),
            thumb_failed: HashSet::new(),
            displays: Vec::new(),
            display_nonce: HashMap::new(),
            display_inflight: HashMap::new(),
            nonce_seq: 1,
            zoom: Zoom::Fit,
            pan: Vec2::ZERO,
            stage_rect: Rect::NOTHING,
            show_info: false,
            caption_buf: String::new(),
            caption_photo: None,
            slideshow: None,
            activity: String::new(),
            scans: 0,
            library_dirty: false,
            last_reload: Instant::now(),
            modal: Modal::None,
            map_lat: 20.0,
            map_lon: 0.0,
            map_zoom: 2,
            map_pan: Vec2::ZERO,
            map_place: false,
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
        app.show_info = app.db.setting("viewer_info").as_deref() != Some("0");
        app.reload();
        app.sync_watches();
        app
    }

    fn reload(&mut self) {
        let keep = self.photos.get(self.viewer_index).map(|photo| photo.id);
        self.library_dirty = false;
        self.last_reload = Instant::now();
        self.folders = self.db.folders();
        self.albums = self.db.albums();
        self.photos = self.db.photos(&self.nav, &self.search, self.sort);
        self.view_epoch = self.view_epoch.wrapping_add(1);
        if let Some(id) = keep {
            if let Some(index) = self.photos.iter().position(|photo| photo.id == id) {
                self.viewer_index = index;
            }
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
            self.selection.retain(|id| self.photos.iter().any(|photo| photo.id == *id));
        }
        if self.mode == Mode::View {
            self.sync_caption();
        }
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
        if self.library_dirty {
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
                self.activity = format!("Scanning {name}");
            }
            UiEvent::ScanProgress { folder, checked, indexed } => {
                self.activity = format!("Scanning {folder} · {checked} checked · {indexed} updated");
            }
            UiEvent::ScanFinished { errors } => {
                self.scans = self.scans.saturating_sub(1);
                if self.scans == 0 {
                    self.activity = if errors > 0 {
                        format!("{errors} files could not be read")
                    } else {
                        String::new()
                    };
                }
                self.library_dirty = true;
            }
            UiEvent::Library => self.library_dirty = true,
            UiEvent::Thumb { id, w, h, rgba } => {
                self.thumb_waiting.remove(&id);
                if let Some(texture) = upload(ctx, &format!("thumb-{id}"), w, h, &rgba) {
                    self.thumbs.insert(id, ThumbTex { texture, w, h });
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
            } => {
                if self.display_nonce.get(&id) != Some(&nonce) {
                    return;
                }
                self.display_inflight.remove(&id);
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
                        self.displays.remove(0);
                    }
                }
            }
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
                if let Some(id) = self.folder_for(&path) {
                    self.watch_deadline.insert(id, Instant::now() + Duration::from_millis(700));
                }
            }
        }
        let due: Vec<i64> = self
            .watch_deadline
            .iter()
            .filter(|(_, when)| Instant::now() >= **when)
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
                    self.activity = format!("Can't watch {}: {err}", path.display());
                }
            }
        }
        self.watched = desired;
    }

    fn tick_slideshow(&mut self) {
        let advance = self
            .slideshow
            .as_ref()
            .map(|show| show.playing && show.last.elapsed().as_secs_f32() >= show.interval)
            .unwrap_or(false);
        if advance && !self.photos.is_empty() {
            if let Some(show) = self.slideshow.as_mut() {
                show.last = Instant::now();
            }
            let next = (self.viewer_index + 1) % self.photos.len();
            self.set_index(next, true);
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
            } else if crate::types::is_image_path(&path) {
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
        let escape = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
        if escape {
            if !matches!(self.modal, Modal::None) {
                self.modal = Modal::None;
                return;
            }
            if self.slideshow.is_some() {
                self.slideshow = None;
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
        if ctx.egui_wants_keyboard_input() || !matches!(self.modal, Modal::None) {
            return;
        }
        let command = |key| ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, key));
        let plain = |key| ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key));
        let shift = |key| ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, key));

        if command(Key::A) && self.mode == Mode::Browse {
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
        if plain(Key::F1) {
            self.modal = Modal::Help;
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
                self.modal = Modal::Trash { ids };
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
            if plain(Key::Space) {
                if self.slideshow.is_some() {
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
                    self.modal = Modal::Trash { ids: vec![id] };
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
        if plain(Key::I) && self.mode == Mode::View {
            self.toggle_info();
        }
        let _ = command;
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::hover());
            theme::paint_logo(ui.painter(), rect.center(), 11.0);
            if ui.button("Import").on_hover_text("Add Folder to Picasso…").clicked() {
                self.modal = Modal::AddFolder {
                    path: String::new(),
                    error: String::new(),
                };
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Slideshow").on_hover_text("Play Fullscreen Slideshow").clicked() {
                    self.start_slideshow();
                }
                if ui.button("Refresh").on_hover_text("Refresh Thumbnails").clicked() {
                    self.rescan();
                }
                let search_w = ui.available_width().max(120.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search")
                        .desired_width(search_w),
                );
                if response.changed() {
                    self.reload();
                }
            });
        });
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Photo Tray").strong().size(12.0).color(TEXT));
            let held: Vec<(i64, String, i32)> = self
                .photos
                .iter()
                .filter(|photo| self.selection.contains(&photo.id))
                .take(16)
                .map(|photo| (photo.id, photo.path.clone(), photo.rotation))
                .collect();
            let extra = self.selection.len().saturating_sub(held.len());
            for (id, path, rotation) in &held {
                self.ensure_thumb(*id, path);
                let (rect, response) = ui.allocate_exact_size(Vec2::new(46.0, 34.0), egui::Sense::click());
                if let Some((texture, w, h)) = self.thumb_texture(*id) {
                    theme::paint_image(ui.painter(), &texture, rect, theme::cover_uv(w, h), *rotation);
                } else {
                    ui.painter().rect_filled(rect, 0, Color32::from_rgb(0xDD, 0xDD, 0xDD));
                }
                ui.painter().rect_stroke(rect, 0, egui::Stroke::new(1.0, theme::HAIRLINE), egui::StrokeKind::Inside);
                if response.double_clicked() {
                    if let Some(index) = self.photos.iter().position(|photo| photo.id == *id) {
                        self.open_at(index);
                    }
                } else if response.clicked() {
                    self.selection.clear();
                    self.selection.insert(*id);
                    self.anchor = Some(*id);
                    self.scroll_to = Some(*id);
                }
            }
            if extra > 0 {
                ui.label(egui::RichText::new(format!("+{extra}")).color(MUTED).size(12.0));
            }
            if !self.selection.is_empty() {
                if ui.button("Clear").on_hover_text("Clear items from the selection").clicked() {
                    self.selection.clear();
                    self.anchor = None;
                }
                if ui.button("Delete").clicked() {
                    let ids: Vec<i64> = self.selection.iter().copied().collect();
                    self.modal = Modal::Trash { ids };
                }
            }
            let count = self.photos.len();
            let noun = if count == 1 { "photo" } else { "photos" };
            let mut text = format!("{count} {noun}");
            if !self.activity.is_empty() {
                text.push_str("  ·  ");
                text.push_str(&self.activity);
            }
            ui.label(egui::RichText::new(text).color(MUTED).size(12.0));
            if self.scans > 0 {
                ui.add(egui::Spinner::new().size(12.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("▮").size(16.0).color(MUTED));
                let mut size = self.thumb_size;
                ui.add(egui::Slider::new(&mut size, 96.0..=220.0).show_value(false));
                ui.label(egui::RichText::new("▮").size(10.0).color(MUTED));
                if (size - self.thumb_size).abs() > 0.1 {
                    self.thumb_size = size;
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            tree_section(ui, "Library");
            if self.tree_row(ui, "Library", Glyph::Library, self.nav == Nav::Library, 4.0).clicked() {
                self.set_nav(Nav::Library);
            }
            if self.tree_row(ui, "Starred Photos", Glyph::Star, self.nav == Nav::Starred, 4.0).clicked() {
                self.set_nav(Nav::Starred);
            }
            if self.tree_row(ui, "Places", Glyph::Pin, self.nav == Nav::Places, 4.0).clicked() {
                self.set_nav(Nav::Places);
            }
            tree_section(ui, "Albums");
            let albums: Vec<(i64, String, i64)> = self
                .albums
                .iter()
                .map(|album| (album.id, album.name.clone(), album.count))
                .collect();
            for (id, name, count) in albums {
                let label = format!("{name} ({count})");
                let response = self.tree_row(ui, &label, Glyph::Album, self.nav == Nav::Album(id), 16.0);
                if response.clicked() {
                    self.set_nav(Nav::Album(id));
                }
                response.context_menu(|ui| {
                    if ui.button("Delete album").clicked() {
                        self.db.delete_album(id);
                        if self.nav == Nav::Album(id) {
                            self.nav = Nav::Library;
                        }
                        self.reload();
                        ui.close();
                    }
                });
            }
            if ui.button("New Album…").clicked() {
                self.modal = Modal::NewAlbum {
                    name: String::new(),
                    error: String::new(),
                };
            }
            tree_section(ui, "Folders");
            let folders: Vec<(i64, String, i64)> = self
                .folders
                .iter()
                .map(|folder| (folder.id, folder_label(&folder.path), folder.count))
                .collect();
            for (id, label, count) in folders {
                let label = format!("{label} ({count})");
                let response = self.tree_row(ui, &label, Glyph::Folder, self.nav == Nav::Folder(id), 4.0);
                if response.clicked() {
                    self.set_nav(Nav::Folder(id));
                }
                response.context_menu(|ui| {
                    if ui.button("Remove from Picasso…").clicked() {
                        self.db.remove_folder(id);
                        if self.nav == Nav::Folder(id) {
                            self.nav = Nav::Library;
                        }
                        self.reload();
                        self.sync_watches();
                        ui.close();
                    }
                    if ui.button("Refresh Thumbnails").clicked() {
                        self.rescan();
                        ui.close();
                    }
                });
            }
        });
    }

    fn tree_row(&mut self, ui: &mut egui::Ui, label: &str, glyph: Glyph, selected: bool, indent: f32) -> egui::Response {
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 22.0), egui::Sense::click());
        if selected {
            ui.painter().rect_filled(rect, 0, theme::SELECT);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 0, Color32::from_rgb(0xF3, 0xF7, 0xFB));
        }
        paint_glyph(ui.painter(), rect.left_center() + Vec2::new(6.0 + indent, 0.0), glyph);
        ui.painter().text(
            rect.left_center() + Vec2::new(26.0 + indent, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            TEXT,
        );
        response
    }

    fn set_nav(&mut self, nav: Nav) {
        let keep_selection = matches!(nav, Nav::Places);
        let kept = self.selection.clone();
        self.nav = nav;
        self.mode = Mode::Browse;
        self.slideshow = None;
        self.search.clear();
        self.reload();
        if keep_selection {
            self.selection = kept;
            self.frame_map();
        } else {
            self.selection.clear();
        }
    }

    fn modals(&mut self, ctx: &egui::Context) {
        match self.modal {
            Modal::None => {}
            Modal::Help => {
                let mut open = true;
                egui::Window::new("Picasso")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label("A local photo library. Folders are scanned on this computer.");
                        ui.add_space(8.0);
                        ui.label(format!("Library: {}", self.data_dir.display()));
                        ui.add_space(8.0);
                        for line in [
                            "Enter — open    Esc — back",
                            "Arrows — move or previous / next",
                            "Space — slideshow     S — star",
                            "Places — geotag on the OpenStreetMap",
                            "HEIC and camera RAW, including NEF and 3FR",
                            "Ctrl+] or R — rotate clockwise     Ctrl+[ or Shift+R — counterclockwise",
                            "Rotation is saved in the library; photo files are never changed",
                            "I — details, histogram, and EXIF",
                            "+ / − zoom     0 fit     1 actual size",
                            "Delete — trash     F5 — rescan     F11 — fullscreen",
                        ] {
                            ui.label(line);
                        }
                    });
                if !open {
                    self.modal = Modal::None;
                }
            }
            Modal::AddFolder { .. } => self.add_folder_modal(ctx),
            Modal::NewAlbum { .. } => self.new_album_modal(ctx),
            Modal::Trash { .. } => self.trash_modal(ctx),
        }
    }

    fn add_folder_modal(&mut self, ctx: &egui::Context) {
        let Modal::AddFolder { path, error } = &self.modal else { return };
        let mut path = path.clone();
        let error = error.clone();
        let mut close = false;
        let mut browse = false;
        let mut submit = false;
        egui::Window::new("Add folder")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Picasso watches this folder and adds new photos as they appear.");
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
                    if ui.button("Browse").clicked() {
                        browse = true;
                    }
                });
                if !error.is_empty() {
                    ui.label(egui::RichText::new(&error).color(Color32::from_rgb(0xC5, 0x22, 0x1F)));
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Add folder").clicked() {
                        submit = true;
                    }
                });
            });
        if browse {
            if let Some(picked) = rfd::FileDialog::new().set_title("Add a photo folder").pick_folder() {
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
        egui::Window::new("New album")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let response = ui.add(egui::TextEdit::singleline(&mut name).hint_text("Album name").desired_width(260.0));
                if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    submit = true;
                }
                if !error.is_empty() {
                    ui.label(egui::RichText::new(&error).color(Color32::from_rgb(0xC5, 0x22, 0x1F)));
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Create").clicked() {
                        submit = true;
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
        let Modal::Trash { ids } = &self.modal else { return };
        let ids = ids.clone();
        let mut close = false;
        let mut confirm = false;
        egui::Window::new("Move to trash")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!("Move {} photo(s) to the system trash?", ids.len()));
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Move to trash").clicked() {
                        confirm = true;
                    }
                });
            });
        if close {
            self.modal = Modal::None;
        } else if confirm {
            self.modal = Modal::None;
            self.trash_ids(&ids);
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

    fn rescan(&mut self) {
        let ids: Vec<i64> = match self.nav {
            Nav::Folder(id) => vec![id],
            _ => self.folders.iter().map(|folder| folder.id).collect(),
        };
        for id in ids {
            let _ = self.cmd_tx.send(Command::Scan(id));
        }
    }

    fn view_title(&self) -> String {
        if !self.search.trim().is_empty() {
            return format!("Search “{}”", self.search.trim());
        }
        match &self.nav {
            Nav::Library => "Library".into(),
            Nav::Starred => "Starred Photos".into(),
            Nav::Places => "Places".into(),
            Nav::Folder(id) => self
                .folders
                .iter()
                .find(|folder| folder.id == *id)
                .map(|folder| folder_label(&folder.path))
                .unwrap_or_else(|| "Folder".into()),
            Nav::Album(id) => self
                .albums
                .iter()
                .find(|album| album.id == *id)
                .map(|album| album.name.clone())
                .unwrap_or_else(|| "Album".into()),
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
        self.selection.clear();
        self.selection.insert(id);
        self.anchor = Some(id);
    }

    fn close_viewer(&mut self) {
        self.mode = Mode::Browse;
        self.slideshow = None;
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
        if self.slideshow.is_some() {
            if let Some(show) = self.slideshow.as_mut() {
                show.last = Instant::now();
            }
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
            let start = self.photos.iter().position(|photo| photo.id == anchor);
            let end = self.photos.iter().position(|photo| photo.id == id);
            if let (Some(start), Some(end)) = (start, end) {
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
        if let Some(anchor) = self.anchor {
            if self.selection.contains(&anchor) {
                if let Some(index) = self.photos.iter().position(|photo| photo.id == anchor) {
                    return Some(index);
                }
            }
        }
        self.photos.iter().position(|photo| self.selection.contains(&photo.id))
    }

    fn primary_starred(&self) -> bool {
        self.primary_index()
            .and_then(|index| self.photos.get(index))
            .map(|photo| photo.starred)
            .unwrap_or(false)
    }

    fn toggle_star(&mut self, id: i64) {
        if let Some(photo) = self.photos.iter_mut().find(|photo| photo.id == id) {
            photo.starred = !photo.starred;
            let starred = photo.starred;
            self.db.set_star(id, starred);
        }
        if self.nav == Nav::Starred {
            self.reload();
        }
    }

    fn toggle_star_selection(&mut self) {
        let ids: Vec<i64> = if self.selection.is_empty() {
            self.primary_index()
                .and_then(|index| self.photos.get(index).map(|photo| photo.id))
                .into_iter()
                .collect()
        } else {
            self.selection.iter().copied().collect()
        };
        if ids.is_empty() {
            return;
        }
        let make = !self
            .photos
            .iter()
            .find(|photo| photo.id == ids[0])
            .map(|photo| photo.starred)
            .unwrap_or(false);
        for id in ids {
            self.db.set_star(id, make);
        }
        self.reload();
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
        for photo in self.photos.iter_mut().filter(|photo| ids.contains(&photo.id)) {
            photo.rotation = turn(photo.rotation, delta);
        }
        if self.mode == Mode::View {
            self.zoom = Zoom::Fit;
            self.pan = Vec2::ZERO;
        }
        let noun = if ids.len() == 1 { "photo" } else { "photos" };
        let way = if delta > 0 { "clockwise" } else { "counterclockwise" };
        self.activity = format!("Rotated {} {noun} {way}", ids.len());
    }

    fn remove_selection_from_album(&mut self) {
        let Nav::Album(album) = self.nav else { return };
        let ids: Vec<i64> = self.selection.iter().copied().collect();
        self.db.remove_from_album(album, &ids);
        self.selection.clear();
        self.reload();
    }

    fn trash_ids(&mut self, ids: &[i64]) {
        let mut removed = Vec::new();
        for id in ids {
            let Some(photo) = self.db.photo(*id) else { continue };
            match trash::delete(&photo.path) {
                Ok(()) => removed.push(*id),
                Err(err) => self.activity = format!("Couldn't trash {}: {err}", photo.filename),
            }
        }
        if !removed.is_empty() {
            self.db.delete_photos(&removed);
            self.activity = format!("Moved {} to trash", removed.len());
            self.reload();
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
            interval: 3.0,
            last: Instant::now(),
        });
    }

    /// Shows or hides the details, histogram, and EXIF panel. The choice is
    /// remembered between sessions.
    fn toggle_info(&mut self) {
        self.show_info = !self.show_info;
        self.db.set_setting("viewer_info", if self.show_info { "1" } else { "0" });
    }

    fn ensure_thumb(&mut self, id: i64, path: &str) {
        if self.thumbs.map.contains_key(&id) || self.thumb_waiting.contains(&id) || self.thumb_failed.contains(&id) {
            return;
        }
        self.thumb_waiting.insert(id);
        let _ = self.urgent_tx.send(ThumbJob::Thumb {
            id,
            path: path.to_string(),
        });
    }

    fn thumb_texture(&mut self, id: i64) -> Option<(TextureHandle, usize, usize)> {
        self.thumbs.get(id)
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
        if self.displays.iter().any(|item| item.id == photo.id) || self.display_inflight.contains_key(&photo.id) {
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
        let _ = self.urgent_tx.send(ThumbJob::Display {
            id,
            path: photo.path.clone(),
            nonce,
        });
    }

    /// Histogram and EXIF rows decoded with the full-size view of `id`.
    fn display_info(&self, id: i64) -> Option<(crate::photo_info::Histogram, Vec<(&'static str, String)>)> {
        self.displays
            .iter()
            .rev()
            .find(|item| item.id == id)
            .map(|item| ((*item.histogram).clone(), item.exif.clone()))
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
        let _ = self.cmd_tx.send(Command::Stop);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump(ctx);
        self.pump_tiles(ctx);
        self.pump_watch(ctx);
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
        self.handle_keys(ui.ctx());
        self.handle_drops(ui.ctx());
        let viewing = self.mode == Mode::View || self.slideshow.is_some();
        if viewing {
            egui::CentralPanel::default()
                .frame(theme::frame(VIEWER_BG))
                .show(ui, |ui| self.viewer_ui(ui));
        } else {
            egui::Panel::top("header")
                .frame(theme::toolbar_frame(TOOLBAR))
                .show(ui, |ui| self.header(ui));
            egui::Panel::bottom("tray")
                .frame(theme::toolbar_frame(TRAY))
                .show(ui, |ui| self.status(ui));
            egui::Panel::left("nav")
                .resizable(true)
                .default_size(220.0)
                .min_size(180.0)
                .max_size(340.0)
                .frame(theme::toolbar_frame(Color32::WHITE))
                .show(ui, |ui| self.sidebar(ui));
            egui::CentralPanel::default()
                .frame(theme::frame(theme::BG))
                .show(ui, |ui| self.library_ui(ui));
        }
        self.modals(ui.ctx());
    }
}

fn upload(ctx: &egui::Context, name: &str, w: usize, h: usize, rgba: &[u8]) -> Option<TextureHandle> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba[..w * h * 4]);
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

fn tree_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(title).size(13.0).strong().color(TEXT));
    ui.add_space(2.0);
}

#[derive(Clone, Copy)]
enum Glyph {
    Library,
    Folder,
    Album,
    Star,
    Pin,
}

fn paint_glyph(painter: &egui::Painter, center_left: egui::Pos2, glyph: Glyph) {
    let origin = center_left + Vec2::new(0.0, -6.0);
    match glyph {
        Glyph::Folder | Glyph::Album => {
            let body = theme::FOLDER;
            painter.rect_filled(Rect::from_min_size(origin + Vec2::new(0.0, 3.0), Vec2::new(12.0, 8.0)), 1, body);
            painter.rect_filled(Rect::from_min_size(origin, Vec2::new(6.0, 3.0)), 1, Color32::from_rgb(0xD7, 0xA4, 0x1E));
        }
        Glyph::Star => {
            painter.text(
                origin + Vec2::new(6.0, 6.0),
                egui::Align2::CENTER_CENTER,
                "★",
                egui::FontId::proportional(12.0),
                theme::STAR,
            );
        }
        Glyph::Library => {
            painter.rect_filled(
                Rect::from_min_size(origin + Vec2::new(1.0, 1.0), Vec2::new(10.0, 10.0)),
                1,
                theme::ACCENT,
            );
        }
        Glyph::Pin => {
            painter.circle_filled(origin + Vec2::new(6.0, 5.0), 3.4, Color32::from_rgb(0xE0, 0x45, 0x3A));
            painter.circle_filled(origin + Vec2::new(6.0, 4.2), 1.3, Color32::WHITE);
        }
    }
}
