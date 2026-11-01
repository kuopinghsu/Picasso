//! The app drawn without a window, for tests and `make bench`. It opens a
//! library of its own and is given input as the window would give it, one
//! frame per refresh of a 60 Hz display.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, Rect, TouchPhase, Vec2};
use eframe::App;

use super::PicassoApp;
use crate::db::Database;
use crate::types::Nav;

/// One refresh of a 60 Hz display.
pub(crate) const REFRESH: Duration = Duration::from_micros(16_667);
/// A window of the size Picasso opens at.
pub(crate) const WINDOW: Vec2 = Vec2::new(1280.0, 800.0);

/// A library in folders of its own, removed when dropped.
pub(crate) struct Scratch {
    dir: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    config: PathBuf,
}

impl Scratch {
    /// Empty folders under `dir` and a library in them, marked as one already
    /// in use, so the app doesn't add the Pictures folder.
    pub(crate) fn new(dir: PathBuf) -> (Self, Database) {
        let _ = std::fs::remove_dir_all(&dir);
        let scratch = Scratch { data: dir.join("data"), cache: dir.join("cache"), config: dir.join("config"), dir };
        for folder in [&scratch.data, &scratch.cache, &scratch.config] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let db = Database::open(&scratch.data.join("picasso.db")).unwrap();
        db.set_setting("seeded", "1");
        (scratch, db)
    }

    /// The app on this library, in English, with no backup or update check
    /// due.
    pub(crate) fn open(&self) -> Headless {
        crate::backup::create(&self.data).unwrap();
        std::fs::write(self.config.join("settings.json"), r#"{"check_updates": false, "language": "en"}"#).unwrap();
        let ctx = egui::Context::default();
        let app = PicassoApp::open(&ctx, Vec::new(), (self.data.clone(), self.cache.clone()), &self.config).expect("open the library");
        Headless { ctx, app, window: eframe::Frame::_new_kittest(), started: Instant::now(), size: WINDOW, pixels_per_point: 1.0, access: None }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) struct Headless {
    ctx: egui::Context,
    app: PicassoApp,
    window: eframe::Frame,
    started: Instant,
    /// The window's size, in points.
    size: Vec2,
    /// Screen pixels per point: 2 on a high-DPI display.
    pixels_per_point: f32,
    /// What the last frame told screen readers, once asked for with
    /// [`egui::Context::enable_accesskit`].
    access: Option<egui::accesskit::TreeUpdate>,
}

impl Headless {
    /// Draws one frame with `events` and returns its time: the app's update
    /// and turning what it drew into triangles, as the window counts it,
    /// without the graphics card's part.
    pub(crate) fn draw(&mut self, events: Vec<Event>) -> Duration {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.started.elapsed().as_secs_f64()),
            events,
            ..Default::default()
        };
        input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(self.pixels_per_point), ..Default::default() });
        let (app, window) = (&mut self.app, &mut self.window);
        let begun = Instant::now();
        let mut output = self.ctx.run_ui(input, |ui| {
            app.logic(ui.ctx(), window);
            app.ui(ui, window);
        });
        self.ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        let time = begun.elapsed();
        self.access = output.platform_output.accesskit_update.take();
        time
    }

    /// Draws a frame each refresh until `done`, failing after `limit`.
    pub(crate) fn draw_until(&mut self, what: &str, limit: Duration, mut done: impl FnMut(&PicassoApp) -> bool) {
        let begun = Instant::now();
        loop {
            self.draw(Vec::new());
            if done(&self.app) {
                return;
            }
            assert!(begun.elapsed() < limit, "{what} took longer than {limit:?}");
            std::thread::sleep(REFRESH);
        }
    }

    /// Draws until the view lists `photos` photos and the scan the app
    /// starts with has ended.
    pub(crate) fn settle(&mut self, photos: usize, limit: Duration) {
        let begun = Instant::now();
        let mut scanned = false;
        self.draw_until(&format!("listing {photos} photos"), limit, |app| {
            scanned |= app.scans > 0;
            let listed = app.photos.len() == photos && !app.loading && app.layout.is_some() && app.scans == 0;
            listed && (scanned || begun.elapsed() > Duration::from_secs(3))
        });
    }
}

impl Drop for Headless {
    fn drop(&mut self) {
        self.app.on_exit(None);
    }
}

/// How the app drew a Trip as it scrolled, each frame's time as
/// [`Headless::draw`] gives it.
pub(crate) struct Scrolled {
    /// From top to bottom.
    pub fling: Vec<Duration>,
    /// Back up, one window height a second.
    pub slow: Vec<Duration>,
    /// Whether the Trip's last photo was on screen at the bottom.
    pub reached_end: bool,
    /// Thumbnails drawn while scrolling slowly.
    pub thumbs_shown: usize,
}

/// Opens the library of `scratch` and scrolls the Trip `trip_id`, which shows
/// `photos` photos, as a trackpad would: to the bottom in a fling of `frames`
/// frames, then back up for as many, slowly enough for the thumbnails to be
/// decoded and shown.
pub(crate) fn scroll_trip(scratch: &Scratch, trip_id: i64, photos: usize, frames: usize) -> Scrolled {
    let mut window = scratch.open();
    window.app.set_nav(Nav::Trip(trip_id));
    window.settle(photos, Duration::from_secs(300));

    let height = window.app.layout.as_ref().map_or(0.0, |layout| layout.layout.height);
    let step = height / frames.max(1) as f32 * 1.1;
    let pointer = Pos2::new(WINDOW.x / 2.0, WINDOW.y / 2.0);
    let wheel = |phase: TouchPhase, dy: f32| Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::new(0.0, dy),
        phase,
        source: MouseWheelSource::Trackpad,
        modifiers: Modifiers::NONE,
    };
    let scroll = |window: &mut Headless, dy: f32, shown: &mut usize| {
        window.draw(vec![Event::PointerMoved(pointer), wheel(TouchPhase::Start, 0.0)]);
        let mut times = Vec::with_capacity(frames);
        let mut next = Instant::now();
        for _ in 0..frames {
            next += REFRESH;
            times.push(window.draw(vec![Event::PointerMoved(pointer), wheel(TouchPhase::Move, dy)]));
            let app = &window.app;
            *shown += app.thumb_wanted.lock().map_or(0, |drawn| drawn.iter().filter(|id| app.thumbs.map.contains_key(id)).count());
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        window.draw(vec![wheel(TouchPhase::End, 0.0)]);
        times
    };
    let fling = scroll(&mut window, -step, &mut 0);
    let last = window.app.photos.last().map(|photo| photo.id);
    let reached_end = last.is_some_and(|id| window.app.thumb_wanted.lock().is_ok_and(|drawn| drawn.contains(&id)));
    let mut thumbs_shown = 0;
    let slow = scroll(&mut window, WINDOW.y / 60.0, &mut thumbs_shown);
    Scrolled { fling, slow, reached_end, thumbs_shown }
}

/// Makes a named pipe at `path`. Opening it to read waits until something
/// writes to it, as reading a file on a share that stopped answering does.
#[cfg(unix)]
pub(crate) fn named_pipe(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `path` is a NUL-terminated string that outlives the call.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o644) }, 0, "mkfifo");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::UiEvent;
    use egui::accesskit::{Action, Node, Role, TreeUpdate};

    /// Whether a screen reader has a name to read for `node`: a label, a
    /// label it points to, the hint of an empty text field, or the text of
    /// a label, which egui gives as its value.
    fn named(node: &Node) -> bool {
        !node.labelled_by().is_empty() || name(node).is_some()
    }

    fn name(node: &Node) -> Option<&str> {
        let text = if node.role() == Role::Label { node.value() } else { None };
        [node.label(), node.placeholder(), text].into_iter().flatten().find(|text| !text.trim().is_empty())
    }

    /// Controls of the last frame, those that can be clicked or take the
    /// keyboard, that have no name, and those reaching past the window's
    /// right edge, where no one can reach them.
    fn problems(update: &TreeUpdate, width: f32) -> Vec<String> {
        let mut found = Vec::new();
        for (_, node) in &update.nodes {
            let control = node.supports_action(Action::Click) || node.supports_action(Action::Focus);
            if !control || node.is_disabled() || matches!(node.role(), Role::ScrollBar | Role::Splitter) {
                continue;
            }
            let bounds = node.bounds();
            if !named(node) {
                found.push(format!("a {:?} at {bounds:?} has no name", node.role()));
            }
            if node.label().is_some_and(|label| label.chars().count() > 80 && node.description() == Some(label)) {
                found.push(format!("a {:?} at {bounds:?} is named by its whole tooltip", node.role()));
            }
            if bounds.is_some_and(|rect| rect.x1 > f64::from(width) + 1.0) {
                found.push(format!("{:?} {:?} reaches past the edge, to {bounds:?}", node.role(), node.label()));
            }
        }
        found
    }

    fn tab() -> Event {
        Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// What the control Tab moves to on each of `presses` presses is called,
    /// and the controls it reaches that have no name.
    fn tab_through(window: &mut Headless, presses: usize) -> (Vec<String>, Vec<String>) {
        let (mut reached, mut unnamed) = (Vec::new(), Vec::new());
        for _ in 0..presses {
            window.draw(vec![tab()]);
            window.draw(Vec::new());
            let update = window.access.as_ref().expect("the tree");
            let Some((_, node)) = update.nodes.iter().find(|(id, _)| *id == update.focus) else {
                continue;
            };
            if node.role() == Role::Window {
                continue;
            }
            if named(node) {
                reached.push(name(node).unwrap_or_default().to_owned());
            } else {
                unnamed.push(format!("{:?} at {:?}", node.role(), node.bounds()));
            }
        }
        (reached, unnamed)
    }

    /// The app on a library of `photos` small JPEGs in one folder, under a
    /// temporary folder named after `name`, once its first scan has listed
    /// them.
    fn library(name: &str, photos: usize) -> (Scratch, Headless) {
        let dir = std::env::temp_dir().join(format!("picasso-headless-{name}-{}", std::process::id()));
        let (scratch, db) = Scratch::new(dir.clone());
        let pics = dir.join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        let jpeg = crate::image_io::encode_jpeg(&image::RgbaImage::from_pixel(64, 48, image::Rgba([90, 120, 160, 255])), 80).unwrap();
        for index in 0..photos {
            std::fs::write(pics.join(format!("IMG_{index:03}.jpg")), &jpeg).unwrap();
        }
        db.add_folder(&pics).unwrap();
        drop(db);
        let mut window = scratch.open();
        // The scan the app starts with lists the folder; what changes on disk after it isn't followed.
        window.app.watcher = None;
        window.settle(photos, Duration::from_secs(60));
        (scratch, window)
    }

    #[test]
    fn a_thumbnail_arriving_for_a_photo_off_screen_takes_the_one_it_would_replace_with_it() {
        let (_scratch, mut window) = library("off-screen", 3);
        let id = window.app.photos[0].id;
        window.draw_until("the thumbnail", Duration::from_secs(30), |app| app.thumbs.map.contains_key(&id));

        // The photo leaves the screen; then a scan finds its file changed and sends a new thumbnail.
        window.app.set_nav(Nav::Trips);
        for _ in 0..3 {
            window.draw(Vec::new());
        }
        let thumb = UiEvent::Thumb { id, w: 1, h: 1, rgba: vec![0; 4], edit: crate::edit::Settings::default() };
        window.app.events_tx.send(thumb).unwrap();
        window.draw(Vec::new());
        assert!(!window.app.thumbs.map.contains_key(&id), "the old thumbnail is gone, to be asked for again when drawn");
        assert!(!window.app.thumb_waiting.contains(&id));
    }

    #[test]
    fn a_publishing_job_whose_thread_ends_without_a_word_stops_counting() {
        let (_scratch, mut window) = library("publishing", 1);
        let alive = std::sync::Arc::new(());
        window.app.publishing.insert((1, 2), std::sync::Arc::downgrade(&alive));
        assert!(window.app.is_publishing(1, 2));
        // Its thread stops before it can say how the job ended, taking what it held.
        std::thread::spawn(move || drop(alive)).join().unwrap();
        assert!(!window.app.is_publishing(1, 2), "the Story can be published again");
        assert!(!window.app.is_publishing(1, 3));
    }

    #[test]
    fn every_screen_names_its_controls_and_fits_the_window() {
        use super::super::ThemeMode;
        use crate::trip::{Capture, TripDraft};
        const PHOTOS: usize = 12;
        let (_scratch, mut window) = library("screens", PHOTOS);
        let long = "A long walk from the Atlantic coast of Galicia across the Meseta to the Mediterranean at Barcelona, with every village between";
        let db = &window.app.db;
        let trip = db.create_trip(&TripDraft::new(long, "Europe/Madrid")).unwrap();
        let ids: Vec<i64> = window.app.photos.iter().map(|photo| photo.id).collect();
        let media = db.add_to_trip(trip, &ids).unwrap();
        for (index, id) in media.iter().enumerate() {
            let clock = chrono::NaiveDate::from_ymd_opt(2025, 7, 1 + index as u32 / 4).unwrap().and_hms_opt(10, 0, 0).unwrap();
            db.store_capture(*id, Capture::Clock { clock, offset: Some(7200) }, clock.and_utc().timestamp() - 7200);
        }
        db.suggest_days(trip).unwrap();
        let days: Vec<i64> = db.trip_days(trip).iter().map(|day| day.id).collect();
        let (story, _) = db.create_story_from_days(trip, long, &days, |date| date.to_string()).unwrap();
        window.app.library_dirty = true;
        window.app.selection.extend(ids.iter().take(2));
        window.ctx.enable_accesskit();

        // Each screen as it is now, with what was found on it.
        let walk = |window: &mut Headless, how: &str| {
            let mut found = Vec::new();
            for screen in ["the library", "the Trips page", "a Trip", "a Story"] {
                match screen {
                    "the library" => window.app.set_nav(Nav::Library),
                    "the Trips page" => window.app.set_nav(Nav::Trips),
                    "a Trip" => window.app.set_nav(Nav::Trip(trip)),
                    _ => {
                        window.app.open_story(story);
                    }
                }
                for _ in 0..20 {
                    window.draw(Vec::new());
                    std::thread::sleep(REFRESH);
                }
                let tree = window.access.as_ref().expect("the tree");
                found.extend(problems(tree, window.size.x).into_iter().map(|problem| format!("{screen}, {how}: {problem}")));
                if screen == "a Story" {
                    let _ = window.app.close_story();
                }
            }
            found
        };

        let mut found = Vec::new();
        // The size Picasso opens at and the smallest it lets the window be.
        for size in [WINDOW, Vec2::new(880.0, 560.0)] {
            for pixels_per_point in [1.0, 2.0] {
                for theme in [ThemeMode::Light, ThemeMode::Dark] {
                    window.size = size;
                    window.pixels_per_point = pixels_per_point;
                    let ctx = window.ctx.clone();
                    window.app.set_theme_mode(&ctx, theme);
                    found.extend(walk(&mut window, &format!("{}×{} at {pixels_per_point}×, {theme:?}", size.x, size.y)));
                    assert_eq!(window.ctx.pixels_per_point(), pixels_per_point);
                    assert_eq!(window.ctx.global_style().visuals.dark_mode, theme == ThemeMode::Dark);
                }
            }
        }

        // German has the longest words and labels, and Traditional Chinese
        // other letters, in the smallest window.
        window.size = Vec2::new(880.0, 560.0);
        window.pixels_per_point = 1.0;
        for lang in [crate::i18n::Lang::German, crate::i18n::Lang::TraditionalChinese] {
            crate::i18n::set_for_thread(Some(lang));
            window.app.reload();
            found.extend(walk(&mut window, &format!("smallest window, {lang:?}")));
            let tree = window.access.as_ref().expect("the tree");
            let settings = crate::i18n::tr("Settings");
            assert_ne!(settings, "Settings");
            assert!(tree.nodes.iter().any(|(_, node)| name(node) == Some(settings)), "drawn in {lang:?}");
        }
        crate::i18n::set_for_thread(None);
        window.app.reload();
        assert!(found.is_empty(), "{}", found.join("\n"));

        // Tab goes from the search field through the controls, each with a name.
        window.size = WINDOW;
        window.app.set_nav(Nav::Library);
        window.draw_until("the library", Duration::from_secs(10), |app| app.nav == Nav::Library && app.layout.is_some());
        let (reached, unnamed) = tab_through(&mut window, 30);
        assert!(unnamed.is_empty(), "Tab reached controls with no name: {unnamed:?}");
        for name in ["Search photos", "Settings", "Thumbnail size", "Library", "pics, 12", "Sort by Date", "IMG_000.jpg"] {
            assert!(reached.iter().any(|reached| reached == name), "Tab never reached {name}: {reached:?}");
        }
    }

    /// Opens the named pipe at `path` for writing and closes it, so a reader
    /// waiting on it reads an empty file. Nothing happens when none waits.
    #[cfg(unix)]
    fn answer(path: &Path) {
        use std::os::unix::fs::OpenOptionsExt;
        drop(std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(path));
    }

    #[cfg(unix)]
    #[test]
    fn a_photo_whose_file_never_answers_holds_up_neither_the_window_nor_the_photo_beside_it() {
        const PHOTOS: usize = 40;
        let (_scratch, mut window) = library("stuck", PHOTOS);

        // The last photo stops answering, as on a share that hangs, and is opened.
        let last = PHOTOS - 1;
        let stuck = window.app.photos[last].clone();
        let beside = window.app.photos[last - 1].id;
        std::fs::remove_file(&stuck.path).unwrap();
        named_pipe(Path::new(&stuck.path));
        window.app.open_at(last);
        window.draw_until("the photo beside it", Duration::from_secs(30), |app| app.displays.iter().any(|item| item.id == beside));
        let mut slowest = Duration::ZERO;
        for _ in 0..60 {
            slowest = slowest.max(window.draw(Vec::new()));
            std::thread::sleep(REFRESH);
        }
        assert!(slowest < Duration::from_millis(500), "a frame took {slowest:?} while a decoder waited");
        assert!(window.app.display_inflight.contains_key(&stuck.id), "its decoder is still waiting");
        assert!(!window.app.display_failed.contains(&stuck.id));

        // The file answers with nothing, each time it is opened: the photo shows as one that can't be read.
        window.draw_until("the photo shown as unreadable", Duration::from_secs(30), |app| {
            answer(Path::new(&stuck.path));
            !app.display_inflight.contains_key(&stuck.id)
        });
        assert!(window.app.display_failed.contains(&stuck.id));
    }
}
