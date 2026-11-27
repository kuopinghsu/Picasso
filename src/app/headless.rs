//! The app drawn without a window, for tests and `make bench`. It opens a
//! library of its own and is given input as the window would give it, one
//! frame per refresh of a 60 Hz display.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Event, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, Rect, TouchPhase, Vec2,
};
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
        let scratch = Scratch {
            data: dir.join("data"),
            cache: dir.join("cache"),
            config: dir.join("config"),
            dir,
        };
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
        std::fs::write(
            self.config.join("settings.json"),
            r#"{"check_updates": false, "language": "en"}"#,
        )
        .unwrap();
        let ctx = egui::Context::default();
        let app = PicassoApp::open(
            &ctx,
            Vec::new(),
            (self.data.clone(), self.cache.clone()),
            &self.config,
        )
        .expect("open the library");
        Headless {
            ctx,
            app,
            window: eframe::Frame::_new_kittest(),
            started: Instant::now(),
            size: WINDOW,
            pixels_per_point: 1.0,
            access: None,
        }
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
        input.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(self.pixels_per_point),
                ..Default::default()
            },
        );
        let (app, window) = (&mut self.app, &mut self.window);
        let begun = Instant::now();
        let mut output = self.ctx.run_ui(input, |ui| {
            app.logic(ui.ctx(), window);
            app.ui(ui, window);
        });
        self.ctx
            .tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        // The headless window has no renderer to apply texture updates.
        output.textures_delta.clear();
        let time = begun.elapsed();
        self.access = output.platform_output.accesskit_update.take();
        time
    }

    /// Draws a frame each refresh until `done`, failing after `limit`.
    pub(crate) fn draw_until(
        &mut self,
        what: &str,
        limit: Duration,
        mut done: impl FnMut(&PicassoApp) -> bool,
    ) {
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
            let listed = app.photos.len() == photos
                && !app.loading
                && app.layout.is_some()
                && app.scans == 0;
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
pub(crate) fn scroll_trip(
    scratch: &Scratch,
    trip_id: i64,
    photos: usize,
    frames: usize,
) -> Scrolled {
    let mut window = scratch.open();
    window.app.set_nav(Nav::Trip(trip_id));
    window.settle(photos, Duration::from_secs(300));

    let height = window
        .app
        .layout
        .as_ref()
        .map_or(0.0, |layout| layout.layout.height);
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
        window.draw(vec![
            Event::PointerMoved(pointer),
            wheel(TouchPhase::Start, 0.0),
        ]);
        let mut times = Vec::with_capacity(frames);
        let mut next = Instant::now();
        for _ in 0..frames {
            next += REFRESH;
            times.push(window.draw(vec![
                Event::PointerMoved(pointer),
                wheel(TouchPhase::Move, dy),
            ]));
            let app = &window.app;
            *shown += app.thumb_wanted.lock().map_or(0, |drawn| {
                drawn
                    .iter()
                    .filter(|id| app.thumbs.map.contains_key(id))
                    .count()
            });
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        window.draw(vec![wheel(TouchPhase::End, 0.0)]);
        times
    };
    let fling = scroll(&mut window, -step, &mut 0);
    let last = window.app.photos.last().map(|photo| photo.id);
    let reached_end = last.is_some_and(|id| {
        window
            .app
            .thumb_wanted
            .lock()
            .is_ok_and(|drawn| drawn.contains(&id))
    });
    let mut thumbs_shown = 0;
    let slow = scroll(&mut window, WINDOW.y / 60.0, &mut thumbs_shown);
    Scrolled {
        fling,
        slow,
        reached_end,
        thumbs_shown,
    }
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
    #[test]
    fn stale_tile_failure_cannot_consume_a_requeued_tile() {
        use crate::geo::{TileDone, TileImage};

        let dir = std::env::temp_dir().join(format!(
            "picasso-headless-tile-reset-{}",
            std::process::id()
        ));
        let (scratch, _db) = super::Scratch::new(dir);
        let mut window = scratch.open();
        let app = &mut window.app;
        let (tx, rx) = crossbeam_channel::unbounded();
        app.tile_rx = rx;
        let old_generation = app.tile_generation;
        app.reset_tiles();
        let key = (2, 1, 1);
        app.tile_wait.insert(key, true);
        tx.send(TileDone {
            source: app.map_source,
            generation: old_generation,
            z: 2,
            x: 1,
            y: 1,
            image: None,
        })
        .unwrap();
        app.pump_tiles(&window.ctx);
        assert_eq!(app.tile_wait.get(&key), Some(&true));
        assert!(!app.tile_fail.contains(&key));

        tx.send(TileDone {
            source: app.map_source,
            generation: app.tile_generation,
            z: 2,
            x: 1,
            y: 1,
            image: Some(TileImage {
                w: 1,
                h: 1,
                rgba: vec![255; 4],
                labels: Vec::new(),
            }),
        })
        .unwrap();
        app.pump_tiles(&window.ctx);
        assert!(app.tiles.contains_key(&key));
        assert!(!app.tile_wait.contains_key(&key));
    }

    use super::super::{Modal, Mode};
    use super::*;
    use crate::i18n::tr;
    use crate::worker::UiEvent;
    use egui::accesskit::{Action, Node, Role, TreeUpdate};

    /// Whether a screen reader has a name to read for `node`: a label, a
    /// label it points to, the hint of an empty text field, or the text of
    /// a label, which egui gives as its value.
    fn named(node: &Node) -> bool {
        !node.labelled_by().is_empty() || name(node).is_some()
    }

    fn name(node: &Node) -> Option<&str> {
        let text = if node.role() == Role::Label {
            node.value()
        } else {
            None
        };
        [node.label(), node.placeholder(), text]
            .into_iter()
            .flatten()
            .find(|text| !text.trim().is_empty())
    }

    /// Controls of the last frame, those that can be clicked or take the
    /// keyboard, that have no name, and those reaching past the window's
    /// right edge, where no one can reach them.
    fn problems(update: &TreeUpdate, width: f32) -> Vec<String> {
        let mut found = Vec::new();
        for (_, node) in &update.nodes {
            let control =
                node.supports_action(Action::Click) || node.supports_action(Action::Focus);
            if !control
                || node.is_disabled()
                || matches!(node.role(), Role::ScrollBar | Role::Splitter)
            {
                continue;
            }
            let bounds = node.bounds();
            if !named(node) {
                found.push(format!("a {:?} at {bounds:?} has no name", node.role()));
            }
            if node.label().is_some_and(|label| {
                label.chars().count() > 80 && node.description() == Some(label)
            }) {
                found.push(format!(
                    "a {:?} at {bounds:?} is named by its whole tooltip",
                    node.role()
                ));
            }
            if bounds.is_some_and(|rect| rect.x1 > f64::from(width) + 1.0) {
                found.push(format!(
                    "{:?} {:?} reaches past the edge, to {bounds:?}",
                    node.role(),
                    node.label()
                ));
            }
        }
        found
    }

    fn key(key: egui::Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn tab() -> Event {
        key(egui::Key::Tab, Modifiers::NONE)
    }

    /// Presses `pressed` and draws the frame after, which shows what it did.
    fn press(window: &mut Headless, pressed: egui::Key, modifiers: Modifiers) {
        window.draw(vec![key(pressed, modifiers)]);
        window.draw(Vec::new());
    }

    /// What the control with keyboard focus is called.
    fn focused(window: &Headless) -> Option<String> {
        let update = window.access.as_ref()?;
        let (_, node) = update.nodes.iter().find(|(id, _)| *id == update.focus)?;
        name(node).map(str::to_owned)
    }

    /// Presses Tab until the focus is on a control called `wanted`, at
    /// most `presses` times; whether it got there.
    fn tab_to(window: &mut Headless, presses: usize, wanted: impl Fn(&str) -> bool) -> bool {
        (0..presses).any(|_| {
            press(window, egui::Key::Tab, Modifiers::NONE);
            focused(window).is_some_and(|name| wanted(&name))
        })
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

    /// Where a control is on screen, as screen readers are told.
    type Bounds = egui::accesskit::Rect;

    /// The nodes of `role` in the last frame's tree, with their bounds.
    fn nodes_of(window: &Headless, role: Role) -> Vec<(&Node, Bounds)> {
        let tree = window.access.as_ref().expect("the tree");
        tree.nodes
            .iter()
            .filter(|(_, node)| node.role() == role)
            .filter_map(|(_, node)| Some((node, node.bounds()?)))
            .collect()
    }

    /// The bounds of the nodes of `role` in the last frame's tree.
    fn bounds_of(window: &Headless, role: Role) -> Vec<Bounds> {
        nodes_of(window, role)
            .into_iter()
            .map(|(_, bounds)| bounds)
            .collect()
    }

    fn center(rect: Bounds) -> Pos2 {
        Pos2::new(
            ((rect.x0 + rect.x1) / 2.0) as f32,
            ((rect.y0 + rect.y1) / 2.0) as f32,
        )
    }

    /// Draws `frames` frames a refresh apart, so animations run their time.
    fn settle(window: &mut Headless, frames: usize) {
        for _ in 0..frames {
            window.draw(Vec::new());
            std::thread::sleep(REFRESH);
        }
    }

    /// Moves the pointer to `at`, rests it there, and clicks.
    fn click(window: &mut Headless, at: Pos2) {
        let button = |pressed: bool| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        for _ in 0..5 {
            window.draw(vec![Event::PointerMoved(at)]);
            std::thread::sleep(REFRESH);
        }
        window.draw(vec![button(true)]);
        window.draw(vec![button(false)]);
    }

    /// Opens the first photo in the editor, with screen readers told what
    /// is drawn, and draws only the frame E is pressed in.
    fn open_editor(window: &mut Headless) {
        window.ctx.enable_accesskit();
        window.draw(Vec::new());
        window.app.select_click(0, Modifiers::NONE);
        window.draw(vec![key(egui::Key::E, Modifiers::NONE)]);
        assert!(window.app.editing);
    }

    /// The app on a library of `photos` small JPEGs in one folder, under a
    /// temporary folder named after `name`, once its first scan has listed
    /// them.
    fn library(name: &str, photos: usize) -> (Scratch, Headless) {
        let dir =
            std::env::temp_dir().join(format!("picasso-headless-{name}-{}", std::process::id()));
        let (scratch, db) = Scratch::new(dir.clone());
        let pics = dir.join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        let jpeg = crate::image_io::encode_jpeg(
            &image::RgbaImage::from_pixel(64, 48, image::Rgba([90, 120, 160, 255])),
            80,
        )
        .unwrap();
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
        window.draw_until("the thumbnail", Duration::from_secs(30), |app| {
            app.thumbs.map.contains_key(&id)
        });

        // The photo leaves the screen; then a scan finds its file changed and sends a new thumbnail.
        window.app.set_nav(Nav::Trips);
        for _ in 0..3 {
            window.draw(Vec::new());
        }
        let thumb = UiEvent::Thumb {
            id,
            w: 1,
            h: 1,
            rgba: vec![0; 4],
            edit: crate::edit::Settings::default(),
        };
        window.app.events_tx.send(thumb).unwrap();
        window.draw(Vec::new());
        assert!(
            !window.app.thumbs.map.contains_key(&id),
            "the old thumbnail is gone, to be asked for again when drawn"
        );
        assert!(!window.app.thumb_waiting.contains(&id));
    }

    #[test]
    fn a_publishing_job_whose_thread_ends_without_a_word_stops_counting() {
        let (_scratch, mut window) = library("publishing", 1);
        let alive = std::sync::Arc::new(());
        window
            .app
            .publishing
            .insert((1, 2), std::sync::Arc::downgrade(&alive));
        assert!(window.app.is_publishing(1, 2));
        // Its thread stops before it can say how the job ended, taking what it held.
        std::thread::spawn(move || drop(alive)).join().unwrap();
        assert!(
            !window.app.is_publishing(1, 2),
            "the Story can be published again"
        );
        assert!(!window.app.is_publishing(1, 3));
    }

    #[test]
    fn uploads_saves_left_unused_are_asked_about_one_site_at_a_time_once_nothing_else_is_open() {
        let (_scratch, mut window) = library("leftovers", 1);
        let escape = || {
            vec![Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]
        };
        let db = &window.app.db;
        db.record_upload(
            7,
            "a",
            "standard",
            41,
            "https://example.com/wp-content/uploads/temple-tea-c9d0e1f2.jpg",
            r#"{"mime":"image/jpeg"}"#,
        )
        .unwrap();
        db.record_upload(
            7,
            "b",
            "standard",
            42,
            "https://example.com/wp-content/uploads/a.jpg",
            r#"{"mime":"image/jpeg"}"#,
        )
        .unwrap();
        db.record_upload(
            8,
            "c",
            "standard",
            51,
            "https://example.com/wp-content/uploads/temple-walk-0a1b2c3d.mp4",
            r#"{"mime":"video/mp4"}"#,
        )
        .unwrap();

        window.app.queue_leftovers(7, "Travel blog", vec![42], None);
        assert!(
            window.app.publish_leftovers.is_empty(),
            "a name too short to look for on the site is never deleted, so never offered"
        );

        // Two saves finish while another window is open.
        window.app.modal = Modal::About;
        window
            .app
            .queue_leftovers(7, "Travel blog", vec![41, 42], None);
        window.app.queue_leftovers(8, "Food blog", vec![51], None);
        window.draw(Vec::new());
        assert!(
            matches!(window.app.modal, Modal::About),
            "it waits for the window open"
        );
        window.app.modal = Modal::None;
        window.draw(Vec::new());
        assert!(matches!(window.app.modal, Modal::DeleteUploads));

        // Closing it keeps them, and the other site's are asked about next.
        window.draw(escape());
        assert!(
            window.app.db.uploads(7).contains_key("a"),
            "nothing deleted"
        );
        assert_eq!(window.app.publish_leftovers.len(), 1);
        window.draw(Vec::new());
        assert!(matches!(window.app.modal, Modal::DeleteUploads));
        window.draw(escape());
        window.draw(Vec::new());
        assert!(matches!(window.app.modal, Modal::None));

        // Deleting from the site stops partway: those not reached are asked about again, once no deletion runs there.
        let alive = std::sync::Arc::new(());
        window
            .app
            .deleting_uploads
            .insert(8, std::sync::Arc::downgrade(&alive));
        let stopped = crate::publish::Deleted {
            gone: Vec::new(),
            in_use: 0,
            failure: Some(crate::wordpress::Failure::Forbidden),
            left: vec![51],
        };
        window.app.on_uploads_deleted(8, "Food blog", stopped);
        window.draw(Vec::new());
        assert!(
            matches!(window.app.modal, Modal::None),
            "not while uploads are still being deleted from the site"
        );
        drop(alive);
        window.draw(Vec::new());
        assert!(matches!(window.app.modal, Modal::DeleteUploads));
    }

    #[test]
    fn every_screen_names_its_controls_and_fits_the_window() {
        use super::super::ThemeMode;
        use crate::trip::{Block, Capture, StoryBlock, TripDraft};
        const PHOTOS: usize = 12;
        let (_scratch, mut window) = library("screens", PHOTOS);
        let long = "A long walk from the western coast across the high plains to the eastern sea at the old port, with every village between";
        let db = &window.app.db;
        let trip = db
            .create_trip(&TripDraft::new(long, "Europe/Madrid"))
            .unwrap();
        let ids: Vec<i64> = window.app.photos.iter().map(|photo| photo.id).collect();
        let media = db.add_to_trip(trip, &ids).unwrap();
        for (index, id) in media.iter().enumerate() {
            let clock = chrono::NaiveDate::from_ymd_opt(2025, 7, 1 + index as u32 / 4)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap();
            db.store_capture(
                *id,
                Capture::Clock {
                    clock,
                    offset: Some(7200),
                },
                clock.and_utc().timestamp() - 7200,
            );
        }
        db.suggest_days(trip).unwrap();
        let (story, _) = db
            .create_story_from_days(trip, long, None, |date| date.to_string())
            .unwrap();
        let mut blocks = db.story_blocks(story);
        let mut table = Block::table(3, 2);
        if let Block::Table { rows, widths, .. } = &mut table {
            rows[0] = vec!["Ferry".into(), "Leaves".into(), "Price".into()];
            *widths = vec![50, 25, 25];
        }
        blocks.push(StoryBlock::new(table));
        blocks.push(StoryBlock::new(Block::YouTube {
            url: "https://youtu.be/abc-DEF_123?t=90".into(),
            caption: "The ferry leaving".into(),
        }));
        db.save_story_blocks(story, db.story(story).unwrap().revision, &blocks)
            .unwrap();
        // Two Places, a track beside the photos linked to the Trip, and a site whose posts show the Places.
        db.set_pins(&ids[..6], 42.88, -8.54);
        db.set_pins(&ids[6..], 41.38, 2.17);
        db.suggest_places(trip, 300.0, 3 * 3600).unwrap();
        let photo = &window.app.photos[0];
        let gpx = Path::new(&photo.path).with_file_name("camino.gpx");
        std::fs::write(&gpx, r#"<gpx><trk><name>Camino</name><trkseg><trkpt lat="42.88" lon="-8.54"><time>2025-07-01T08:00:00Z</time></trkpt><trkpt lat="42.9" lon="-8.5"><time>2025-07-01T09:00:00Z</time></trkpt></trkseg></trk></gpx>"#).unwrap();
        let gpx_path = gpx.to_string_lossy().into_owned();
        db.store_track(
            photo.folder_id,
            &gpx_path,
            1,
            0,
            &crate::track::Track::read(&gpx).unwrap(),
        )
        .unwrap();
        let track = db
            .tracks()
            .into_iter()
            .find(|(.., path, _)| *path == gpx_path)
            .unwrap()
            .0;
        db.link_trip_track(trip, track, None).unwrap();
        let folder = Path::new(&photo.path)
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let site = crate::db::Site {
            label: "Travel blog".into(),
            site_url: "https://example.com".into(),
            api_root: "https://example.com/wp-json/".into(),
            user: "ana".into(),
            secret_ref: "wordpress ana@https://example.com".into(),
            account: "Ana".into(),
            can_upload: true,
            can_edit: true,
            can_publish: true,
            checked_at: Some(100),
            ..crate::db::Site::default()
        };
        let site = db.save_site(&site).unwrap();
        let shown = crate::db::PlaceChoice {
            places: true,
            rough: true,
            left_out: Vec::new(),
            journey: true,
        };
        db.save_place_choice(story, site, &shown).unwrap();
        window.app.library_dirty = true;
        window.app.selection.extend(ids.iter().take(2));
        window.ctx.enable_accesskit();

        // Each screen as it is now, with what was found on it.
        let walk = |window: &mut Headless, how: &str| {
            let mut found = Vec::new();
            let screens = [
                "the library",
                "the Trips page",
                "a Trip",
                "a Story",
                "Geotag",
                "a Trip's Geotag",
                "Publish to WordPress",
                "New story",
            ];
            for screen in screens {
                match screen {
                    "the library" => window.app.set_nav(Nav::Library),
                    "the Trips page" => window.app.set_nav(Nav::Trips),
                    "a Trip" => window.app.set_nav(Nav::Trip(trip)),
                    "Geotag" => {
                        window.app.set_nav(Nav::Library);
                        window.app.open_geotag(&folder);
                    }
                    "a Trip's Geotag" => {
                        window.app.set_nav(Nav::Trip(trip));
                        window.app.open_trip_geotag(trip);
                    }
                    "Publish to WordPress" => {
                        window.app.open_story(story);
                        window.app.ask_publish_story();
                    }
                    "New story" => {
                        window.app.set_nav(Nav::Trip(trip));
                        window.app.ask_new_story(trip);
                    }
                    _ => {
                        window.app.open_story(story);
                    }
                }
                for _ in 0..20 {
                    window.draw(Vec::new());
                    std::thread::sleep(REFRESH);
                }
                let tree = window.access.as_ref().expect("the tree");
                found.extend(
                    problems(tree, window.size.x)
                        .into_iter()
                        .map(|problem| format!("{screen}, {how}: {problem}")),
                );
                if screens[4..].contains(&screen) {
                    let opened = match &window.app.modal {
                        Modal::Geotag(_) => "Geotag",
                        Modal::PublishStory(_) => "Publish",
                        Modal::NewStory(_) => "New story",
                        _ => "nothing",
                    };
                    assert!(screen.contains(opened), "{screen}, {how}: {opened} open");
                    let wanted = match screen {
                        "Publish to WordPress" => [
                            tr("Show places on the post's map"),
                            tr("About 1 km"),
                            tr("Link to the trip's Journey"),
                        ],
                        "New story" => [
                            tr("First day"),
                            tr("Last day"),
                            tr("Start with their photos"),
                        ],
                        _ => [tr("Photos that have GPS"), tr("Add tracks…"), tr("Check")],
                    };
                    let geotag = screen.contains("Geotag");
                    for wanted in wanted
                        .iter()
                        .chain([&"camino.gpx"].into_iter().filter(|_| geotag))
                    {
                        let drawn = tree
                            .nodes
                            .iter()
                            .any(|(_, node)| name(node).is_some_and(|name| name.contains(wanted)));
                        assert!(drawn, "{screen}, {how}: {wanted} is not drawn");
                    }
                    window.app.modal = Modal::None;
                }
                if matches!(screen, "a Story" | "Publish to WordPress") {
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
                    found.extend(walk(
                        &mut window,
                        &format!("{}×{} at {pixels_per_point}×, {theme:?}", size.x, size.y),
                    ));
                    assert_eq!(window.ctx.pixels_per_point(), pixels_per_point);
                    assert_eq!(
                        window.ctx.global_style().visuals.dark_mode,
                        theme == ThemeMode::Dark
                    );
                }
            }
        }

        // German has the longest words and labels, and Traditional Chinese
        // other letters, in the smallest window.
        window.size = Vec2::new(880.0, 560.0);
        window.pixels_per_point = 1.0;
        for lang in [
            crate::i18n::Lang::German,
            crate::i18n::Lang::TraditionalChinese,
        ] {
            crate::i18n::set_for_thread(Some(lang));
            window.app.reload();
            found.extend(walk(&mut window, &format!("smallest window, {lang:?}")));
            let tree = window.access.as_ref().expect("the tree");
            let settings = crate::i18n::tr("Settings");
            assert_ne!(settings, "Settings");
            assert!(
                tree.nodes
                    .iter()
                    .any(|(_, node)| name(node) == Some(settings)),
                "drawn in {lang:?}"
            );
        }
        crate::i18n::set_for_thread(None);
        window.app.reload();
        assert!(found.is_empty(), "{}", found.join("\n"));

        // Tab goes from the search field through the controls, each with a name.
        window.size = WINDOW;
        window.app.set_nav(Nav::Library);
        window.draw_until("the library", Duration::from_secs(10), |app| {
            app.nav == Nav::Library && app.layout.is_some()
        });
        // From the top of the grid, wherever the screens above left it, so
        // Tab meets the first photo whatever the runner's timing.
        window.app.jump_to = Some(0.0);
        window.app.selection.clear();
        window.draw(Vec::new());
        window.draw(Vec::new());
        let (reached, unnamed) = tab_through(&mut window, 30);
        assert!(
            unnamed.is_empty(),
            "Tab reached controls with no name: {unnamed:?}"
        );
        for name in [
            "Search photos",
            "Settings",
            "Thumbnail size",
            "Library",
            "pics, 12",
            "Sort by Date",
            "IMG_000.jpg",
        ] {
            assert!(
                reached.iter().any(|reached| reached == name),
                "Tab never reached {name}: {reached:?}"
            );
        }
    }

    #[test]
    fn back_to_photo_returns_from_places_to_the_photo_shown() {
        let (_scratch, mut window) = library("back", 6);
        window.draw_until("the library", Duration::from_secs(10), |app| {
            app.layout.is_some()
        });
        // From the viewer: the same photo open again, not the first.
        window.app.open_at(3);
        let id = window.app.photos[3].id;
        window.app.show_on_map(id);
        window.draw(Vec::new());
        assert_eq!(window.app.nav, Nav::Places);
        window.app.return_from_places();
        window.draw_until("the photo again", Duration::from_secs(10), |app| {
            app.mode == Mode::View
        });
        assert_eq!(
            (
                window.app.nav.clone(),
                window.app.photos[window.app.viewer_index].id
            ),
            (Nav::Library, id)
        );
        // From the grid: the photo selected and scrolled to.
        window.app.close_viewer();
        let other = window.app.photos[1].id;
        window.app.show_on_map(other);
        window.draw(Vec::new());
        window.app.return_from_places();
        window.draw_until("the library again", Duration::from_secs(10), |app| {
            app.nav == Nav::Library && app.returning.is_none() && !app.loading
        });
        assert_eq!(
            (
                window.app.mode,
                window.app.selection.iter().copied().collect::<Vec<_>>()
            ),
            (Mode::Browse, vec![other])
        );
        assert!(window.app.places_return.is_none(), "used once");
    }

    #[test]
    fn the_library_a_trip_and_a_story_work_from_the_keyboard_alone() {
        use crate::trip::{MemberQuery, TripDraft};
        use egui::Key;
        let (_scratch, mut window) = library("keys", 6);
        let db = &window.app.db;
        let trip = db
            .create_trip(&TripDraft::new("Coast", "Europe/Madrid"))
            .unwrap();
        let ids: Vec<i64> = window.app.photos.iter().map(|photo| photo.id).collect();
        db.add_to_trip(trip, &ids).unwrap();
        let day = db.add_trip_day(trip, None, "Arrival").unwrap();
        db.create_story(trip, "Coast story").unwrap();
        window.app.library_dirty = true;
        window.ctx.enable_accesskit();
        window.draw_until("the library", Duration::from_secs(10), |app| {
            app.layout.is_some()
        });
        let file = |window: &Headless, index: usize| window.app.photos[index].filename.clone();

        // With nothing focused the arrows choose a photo, S stars it, Enter
        // opens it and Escape goes back; Delete asks before trashing.
        press(&mut window, Key::ArrowRight, Modifiers::NONE);
        assert_eq!(window.app.selection.len(), 1);
        let chosen = window.app.primary_index().unwrap();
        press(&mut window, Key::S, Modifiers::NONE);
        assert!(window.app.photos[chosen].starred);
        press(&mut window, Key::Enter, Modifiers::NONE);
        assert_eq!(
            (window.app.mode, window.app.viewer_index),
            (Mode::View, chosen)
        );
        press(&mut window, Key::ArrowRight, Modifiers::NONE);
        assert_eq!(window.app.viewer_index, chosen + 1);
        press(&mut window, Key::Escape, Modifiers::NONE);
        assert_eq!(window.app.mode, Mode::Browse);
        press(&mut window, Key::Delete, Modifiers::NONE);
        assert!(matches!(window.app.modal, Modal::Trash { .. }));
        press(&mut window, Key::Escape, Modifiers::NONE);
        assert!(matches!(window.app.modal, Modal::None));
        assert_eq!(window.app.photos.len(), 6, "nothing trashed");

        // A photo reached with Tab: the arrows move the selection and the
        // focus together, and the shortcuts still work.
        let first = file(&window, 0);
        assert!(
            tab_to(&mut window, 40, |name| name == first),
            "Tab reaches the first photo"
        );
        press(&mut window, Key::ArrowRight, Modifiers::NONE);
        assert_eq!(window.app.primary_index(), Some(1));
        assert_eq!(
            focused(&window),
            Some(file(&window, 1)),
            "the focus follows"
        );
        press(&mut window, Key::ArrowLeft, Modifiers::NONE);
        assert_eq!(focused(&window), Some(file(&window, 0)));
        press(&mut window, Key::ArrowRight, Modifiers::NONE);
        assert_eq!(focused(&window), Some(file(&window, 1)));
        let starred = window.app.photos[1].starred;
        press(&mut window, Key::S, Modifiers::NONE);
        assert_ne!(window.app.photos[1].starred, starred);
        press(&mut window, Key::Enter, Modifiers::NONE);
        assert_eq!(
            (window.app.mode, window.app.viewer_index),
            (Mode::View, 1),
            "Enter opens the focused photo"
        );
        press(&mut window, Key::Escape, Modifiers::NONE);

        // A button reached with Tab takes Enter, and F1 still opens Help.
        assert!(
            tab_to(&mut window, 40, |name| name.starts_with(tr("All trips"))),
            "Tab reaches All trips"
        );
        press(&mut window, Key::F1, Modifiers::NONE);
        assert!(matches!(window.app.modal, Modal::Help));
        press(&mut window, Key::Escape, Modifiers::NONE);
        assert!(matches!(window.app.modal, Modal::None));
        assert!(tab_to(&mut window, 40, |name| name.starts_with(tr("All trips"))));
        press(&mut window, Key::Enter, Modifiers::NONE);
        assert_eq!(window.app.nav, Nav::Trips);
        assert!(
            tab_to(&mut window, 40, |name| name == "Coast"),
            "Tab reaches the Trip's card"
        );
        press(&mut window, Key::Enter, Modifiers::NONE);
        assert_eq!(window.app.nav, Nav::Trip(trip));

        // In the Trip, Space chooses the focused photo, Alt+1 puts it in the
        // first Day, and the Story opens and closes from its buttons.
        window.draw_until("the Trip's photos", Duration::from_secs(10), |app| {
            app.photos.len() == 6
        });
        let first = file(&window, 0);
        assert!(
            tab_to(&mut window, 60, |name| name == first),
            "Tab reaches the Trip's first photo"
        );
        press(&mut window, Key::Space, Modifiers::NONE);
        assert_eq!(window.app.primary_index(), Some(0));
        press(&mut window, Key::Num1, Modifiers::ALT);
        let members = window
            .app
            .db
            .trip_members(trip, &MemberQuery::default(), 0, usize::MAX);
        let photo = window.app.photos[0].id;
        assert_eq!(
            members
                .iter()
                .find(|member| member.photo_id == Some(photo))
                .and_then(|member| member.day_id),
            Some(day)
        );
        let open = crate::i18n::trf("Continue “{}”", &[&"Coast story"]);
        assert!(
            tab_to(&mut window, 80, |name| name == open),
            "Tab reaches the Story"
        );
        press(&mut window, Key::Enter, Modifiers::NONE);
        assert!(window.app.story_shown());
        assert!(
            tab_to(&mut window, 40, |name| name.ends_with("Coast")),
            "Tab reaches the way back"
        );
        press(&mut window, Key::Enter, Modifiers::NONE);
        assert!(!window.app.story_shown());
    }

    #[test]
    fn a_selection_becomes_a_trip_its_photos_go_into_days_and_the_editor_gives_back_where_you_were()
    {
        use crate::trip::{DayFilter, MemberQuery};
        use egui::Key;
        use std::collections::HashSet;
        let (_scratch, mut window) = library("flows", 8);
        window.ctx.enable_accesskit();
        window.draw_until("the library", Duration::from_secs(10), |app| {
            app.layout.is_some()
        });

        // Three photos, a click and a Shift-click, then Add to Trip ▸ New trip…
        window.app.select_click(0, Modifiers::NONE);
        window.app.select_click(2, Modifiers::SHIFT);
        let chosen = window.app.selection_in_order();
        assert_eq!(chosen.len(), 3);
        window.app.add_photos_to_trip(None, chosen.clone());
        window.draw(Vec::new());
        window.draw(vec![Event::Text("Coast".into())]);
        press(&mut window, Key::Tab, Modifiers::NONE);
        window.draw(vec![Event::Text("+02:00".into())]);
        assert!(
            tab_to(&mut window, 20, |name| name == tr("Create")),
            "Tab reaches Create"
        );
        press(&mut window, Key::Enter, Modifiers::NONE);
        let Nav::Trip(trip) = window.app.nav else {
            panic!("the new Trip opens, not {:?}", window.app.nav)
        };
        assert_eq!(
            window.app.db.trip(trip).map(|trip| (trip.title, trip.zone)),
            Some(("Coast".into(), "+02:00".into()))
        );
        window.draw_until("the Trip's photos", Duration::from_secs(10), |app| {
            app.photos.len() == 3
        });
        let ids: Vec<i64> = window.app.photos.iter().map(|photo| photo.id).collect();
        assert_eq!(
            ids.iter().copied().collect::<HashSet<_>>(),
            chosen.iter().copied().collect(),
            "exactly the photos chosen"
        );

        // All three into the first Day at once, one of them into the second, and back.
        let arrival = window.app.db.add_trip_day(trip, None, "Arrival").unwrap();
        let coast = window.app.db.add_trip_day(trip, None, "Coast").unwrap();
        window.app.reload();
        window.draw(Vec::new());
        let day_of = |window: &Headless, photo: i64| {
            let members = window
                .app
                .db
                .trip_members(trip, &MemberQuery::default(), 0, usize::MAX);
            members
                .iter()
                .find(|member| member.photo_id == Some(photo))
                .and_then(|member| member.day_id)
        };
        press(&mut window, Key::A, Modifiers::COMMAND);
        press(&mut window, Key::Num1, Modifiers::ALT);
        assert!(ids.iter().all(|id| day_of(&window, *id) == Some(arrival)));
        window.app.select_click(1, Modifiers::NONE);
        press(&mut window, Key::Num2, Modifiers::ALT);
        assert_eq!(day_of(&window, ids[1]), Some(coast));
        press(&mut window, Key::Z, Modifiers::COMMAND);
        assert_eq!(day_of(&window, ids[1]), Some(arrival), "undone");

        // Showing the first Day, into the editor with one photo and out again:
        // the same Trip, Day, selection and photo in view.
        window.app.trip_filter.day = DayFilter::Day(arrival);
        window.app.reload();
        window.draw(Vec::new());
        window.app.select_click(1, Modifiers::NONE);
        let photo = window.app.photos[1].id;
        press(&mut window, Key::E, Modifiers::NONE);
        assert!(window.app.editing && window.app.mode == Mode::View);
        press(&mut window, Key::Escape, Modifiers::NONE);
        press(&mut window, Key::Escape, Modifiers::NONE);
        assert_eq!(
            (
                window.app.mode,
                window.app.nav.clone(),
                window.app.trip_filter.day
            ),
            (Mode::Browse, Nav::Trip(trip), DayFilter::Day(arrival))
        );
        assert_eq!(
            window.app.selection.iter().copied().collect::<Vec<_>>(),
            [photo]
        );

        // To the Library and back: the Trip still shows the first Day.
        window.app.set_nav(Nav::Library);
        window.draw(Vec::new());
        window.app.set_nav(Nav::Trip(trip));
        assert_eq!(window.app.trip_filter.day, DayFilter::Day(arrival));
    }

    #[test]
    fn the_edit_panels_scroll_bar_keeps_clear_of_its_sliders() {
        use super::super::editor::Tool;
        let (_scratch, mut window) = library("edit-scroll-bar", 2);
        // Tall enough for Transform's controls, and Mask's before its first
        // mask, to fit without a bar, though not Adjust's sections.
        window.size = Vec2::new(1280.0, 1000.0);
        // Every page's scroll bars sit beside what they scroll, not over it.
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let style = window.ctx.style_of(theme);
            assert!(!style.spacing.scroll.floating, "floating bars in {theme:?}");
        }
        open_editor(&mut window);
        // Each slider and the scroll bar beside it, nearest on its right:
        // the slider ends before the bar, or the bar takes the clicks on its
        // right end. Sliders with no bar beside them are left out.
        let pairs = |window: &Headless| -> Vec<_> {
            let bars = bounds_of(window, Role::ScrollBar);
            bounds_of(window, Role::Slider)
                .into_iter()
                .filter_map(|slider| {
                    bars.iter()
                        .filter(|bar| bar.x0 > slider.x0)
                        .filter(|bar| slider.y0 < bar.y1 && slider.y1 > bar.y0)
                        .min_by(|a, b| a.x0.total_cmp(&b.x0))
                        .map(|bar| (slider, *bar))
                })
                .collect()
        };
        let check = |window: &Headless, when: &str| {
            let found = pairs(window);
            assert!(!found.is_empty(), "{when}: no slider beside a scroll bar");
            for (slider, bar) in found {
                assert!(
                    slider.x1 <= bar.x0,
                    "{when}: a slider at {slider:?} runs under the scroll bar at {bar:?}"
                );
            }
        };
        // The first frame the sliders are drawn beside a bar, before it could
        // slide in.
        let first = |window: &mut Headless, when: &str| {
            let mut frames = 0;
            while pairs(window).is_empty() {
                assert!(frames < 10, "{when}: no slider beside a scroll bar");
                window.draw(Vec::new());
                frames += 1;
            }
            check(window, when);
        };
        first(&mut window, "opening the editor");
        settle(&mut window, 30);
        check(&window, "half a second on");
        // The Edit panel's bars: right of where Adjust's sliders start.
        let panel_left = pairs(&window)
            .iter()
            .map(|(slider, _)| slider.x0)
            .fold(f64::MAX, f64::min);
        let panel_bars = |window: &Headless| {
            bounds_of(window, Role::ScrollBar)
                .into_iter()
                .filter(|bar| bar.x0 > panel_left)
                .count()
        };
        // Crop's straighten slider keeps clear of its bar too.
        window.app.set_tool(Tool::Crop);
        window.draw(Vec::new());
        first(&mut window, "Crop");
        // To Transform, which has no sliders and fits without a bar, and back
        // to Adjust.
        window.app.set_tool(Tool::Transform);
        settle(&mut window, 30);
        assert_eq!(panel_bars(&window), 0, "Transform fits without a bar");
        window.app.set_tool(Tool::Adjust);
        window.draw(Vec::new());
        first(&mut window, "back from Transform");
        // Mask before its first mask has no sliders, and no bar beside its
        // buttons.
        window.app.set_tool(Tool::Mask);
        settle(&mut window, 30);
        assert_eq!(panel_bars(&window), 0, "Mask without a mask has no bar");
        // With a mask, its sliders are below the list of masks.
        let add = nodes_of(&window, Role::Button)
            .into_iter()
            .find(|(node, _)| node.label() == Some("+ Linear gradient"))
            .map(|(_, rect)| center(rect))
            .expect("Mask's + Linear gradient");
        click(&mut window, add);
        settle(&mut window, 30);
        check(&window, "Mask with a mask");
        window.app.set_tool(Tool::Transform);
        settle(&mut window, 30);
        window.app.set_tool(Tool::Mask);
        window.draw(Vec::new());
        first(&mut window, "Mask from Transform");
    }

    #[test]
    fn a_click_at_the_right_end_of_an_edit_slider_takes_it_to_its_top_value() {
        // The steps of issue 2: open the editor, find a slider beside the
        // panel's scroll bar, and click at its far-right end. The slider
        // takes the click and goes to its top value; under a bar floating
        // over it, the bar took the click and the slider stayed.
        let (_scratch, mut window) = library("edit-slider-end", 2);
        open_editor(&mut window);
        settle(&mut window, 30);
        let bars = bounds_of(&window, Role::ScrollBar);
        // A slider in full view beside a bar to its right, below its top.
        let (id, slider, before, top) = {
            let tree = window.access.as_ref().expect("the tree");
            tree.nodes
                .iter()
                .filter(|(_, node)| node.role() == Role::Slider)
                .filter_map(|(id, node)| {
                    Some((
                        *id,
                        node.bounds()?,
                        node.numeric_value()?,
                        node.max_numeric_value()?,
                    ))
                })
                .find(|(_, slider, value, top)| {
                    value < top
                        && bars.iter().any(|bar| {
                            bar.x0 > slider.x0 && slider.y0 >= bar.y0 && slider.y1 <= bar.y1
                        })
                })
                .expect("a slider in view beside the Edit panel's scroll bar")
        };
        let at = Pos2::new(slider.x1 as f32 - 2.0, center(slider).y);
        click(&mut window, at);
        window.draw(Vec::new());
        let tree = window.access.as_ref().expect("the tree");
        let after = tree
            .nodes
            .iter()
            .find(|(node, _)| *node == id)
            .and_then(|(_, node)| node.numeric_value())
            .expect("the same slider");
        assert!(
            (after - top).abs() < 1e-6,
            "a click at {at:?}, the right end of the slider at {slider:?}, took it from {before} to {after}, not its top value {top}"
        );
    }

    #[test]
    fn the_sort_list_keeps_less_used_orders_in_a_submenu() {
        use crate::types::Sort;
        let (_scratch, mut window) = library("sort-menu", 3);
        window.ctx.enable_accesskit();
        settle(&mut window, 3);
        // Where the control called `wanted` is, and whether it is there.
        let find = |window: &Headless, wanted: &str| -> Option<Pos2> {
            let tree = window.access.as_ref().expect("the tree");
            tree.nodes
                .iter()
                .find(|(_, node)| name(node) == Some(wanted))
                .and_then(|(_, node)| node.bounds())
                .map(center)
        };
        let list = find(&window, "Sort by Date").expect("the sort list");
        click(&mut window, list);
        settle(&mut window, 5);
        for wanted in [
            "Sort by Date, oldest",
            "Sort by Name",
            "Sort by Size",
            "More sorts",
        ] {
            assert!(find(&window, wanted).is_some(), "{wanted} not in the list");
        }
        assert!(
            find(&window, "Sort by File type").is_none(),
            "the submenu is closed until pointed at"
        );
        let more = find(&window, "More sorts").unwrap();
        for _ in 0..5 {
            window.draw(vec![Event::PointerMoved(more)]);
            std::thread::sleep(REFRESH);
        }
        settle(&mut window, 5);
        for wanted in [
            "Sort by Size, smallest",
            "Sort by Date modified",
            "Sort by Date modified, oldest",
        ] {
            assert!(
                find(&window, wanted).is_some(),
                "{wanted} not in the submenu"
            );
        }
        let kind = find(&window, "Sort by File type").expect("the submenu");
        // Along the row first, then down, so the pointer stays on the submenu.
        window.draw(vec![Event::PointerMoved(Pos2::new(kind.x, more.y))]);
        click(&mut window, kind);
        settle(&mut window, 5);
        assert_eq!(window.app.sort, Sort::Type);
        // The list is closed and named for the order chosen.
        assert!(find(&window, "More sorts").is_none());
        assert!(nodes_of(&window, Role::ComboBox)
            .iter()
            .any(|(node, _)| name(node) == Some("Sort by File type")));

        // On a touch screen nothing points at More sorts before it is
        // tapped: the tap opens the submenu and leaves the list open.
        let list = find(&window, "Sort by File type").expect("the sort list");
        click(&mut window, list);
        settle(&mut window, 5);
        let more = find(&window, "More sorts").expect("the list");
        let button = |pressed: bool| Event::PointerButton {
            pos: more,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        window.draw(vec![Event::PointerMoved(more), button(true)]);
        window.draw(vec![button(false)]);
        settle(&mut window, 5);
        let smallest = find(&window, "Sort by Size, smallest").expect("the submenu after a tap");
        window.draw(vec![Event::PointerMoved(Pos2::new(smallest.x, more.y))]);
        click(&mut window, smallest);
        settle(&mut window, 5);
        assert_eq!(window.app.sort, Sort::SizeSmallest);

        // With the keyboard: Enter opens the list, ↓ goes to More sorts, →
        // opens it, and Enter picks an order there.
        window.draw(vec![Event::PointerGone]);
        assert!(
            tab_to(&mut window, 40, |name| name == "Sort by Size, smallest"),
            "Tab reaches the sort list"
        );
        press(&mut window, egui::Key::Enter, Modifiers::NONE);
        settle(&mut window, 3);
        let mut moves = 0;
        while focused(&window).as_deref() != Some("More sorts") {
            moves += 1;
            assert!(
                moves < 10,
                "↓ never reached More sorts: {:?}",
                focused(&window)
            );
            press(&mut window, egui::Key::ArrowDown, Modifiers::NONE);
        }
        press(&mut window, egui::Key::ArrowRight, Modifiers::NONE);
        settle(&mut window, 3);
        let mut moves = 0;
        while focused(&window).as_deref() != Some("Sort by Date modified") {
            moves += 1;
            assert!(
                moves < 10,
                "↓ never reached the order: {:?}",
                focused(&window)
            );
            press(&mut window, egui::Key::ArrowDown, Modifiers::NONE);
        }
        press(&mut window, egui::Key::Enter, Modifiers::NONE);
        settle(&mut window, 3);
        assert_eq!(window.app.sort, Sort::ModifiedNewest);
    }

    #[test]
    fn photos_taken_together_stack_and_open_from_their_number() {
        // The photos are written together, so they were taken within a
        // second of each other by their file times.
        let (_scratch, mut window) = library("stacks", 4);
        window.ctx.enable_accesskit();
        settle(&mut window, 3);
        assert_eq!(window.app.photos.len(), 4, "no stacks until asked for");
        let find = |window: &Headless, wanted: &str| -> Option<Pos2> {
            let tree = window.access.as_ref().expect("the tree");
            tree.nodes
                .iter()
                .find(|(_, node)| name(node) == Some(wanted))
                .and_then(|(_, node)| node.bounds())
                .map(center)
        };
        let check = find(&window, "Stack photos taken within").expect("the stack setting");
        click(&mut window, check);
        settle(&mut window, 5);
        assert_eq!(window.app.photos.len(), 1, "one stack of four");
        assert!(
            find(&window, "Show the 4 photos of this stack").is_some(),
            "the stack's number, named"
        );
        // The count still says how many photos the view holds.
        assert_eq!(window.app.photos_in_view(), 4);
        let number = find(&window, "Show the 4 photos of this stack").unwrap();
        click(&mut window, number);
        settle(&mut window, 5);
        assert_eq!(window.app.photos.len(), 4, "the stack opens in place");
        // A selected photo keeps its stack open, until it is closed.
        let member = window.app.photos[2].id;
        window.app.selection = std::collections::HashSet::from([member]);
        window.app.anchor = Some(member);
        window.app.restack();
        assert_eq!(window.app.photos.len(), 4);
        let number = find(&window, "Stack these 4 photos again").expect("open stack's number");
        click(&mut window, number);
        settle(&mut window, 5);
        assert_eq!(window.app.photos.len(), 1, "and closes again");
        assert!(
            window.app.selection.is_empty(),
            "with its photos out of the selection"
        );
        assert_eq!(
            window.app.anchor,
            Some(window.app.photos[0].id),
            "and Shift-click going from the photo on top"
        );
        // A stack opened only for a selected photo closes once it isn't.
        window.app.selection = std::collections::HashSet::from([member]);
        window.app.restack();
        assert_eq!(window.app.photos.len(), 4);
        window.app.selection.clear();
        window.app.restack();
        assert_eq!(window.app.photos.len(), 1);
        // Everything Tab reaches has a name, the stack's number among them.
        let (reached, unnamed) = tab_through(&mut window, 30);
        assert!(unnamed.is_empty(), "unnamed: {unnamed:?}");
        assert!(
            reached
                .iter()
                .any(|name| name == "Show the 4 photos of this stack"),
            "Tab reaches the stack's number: {reached:?}"
        );
        // An open stack whose photo on top leaves stays open.
        let number = find(&window, "Show the 4 photos of this stack").unwrap();
        click(&mut window, number);
        settle(&mut window, 5);
        let top = window.app.photos[0].id;
        window.app.drop_from_view(&[top]);
        assert_eq!(window.app.photos.len(), 3, "the other three stay open");
        // Sorted by name the photos show one by one again.
        window.app.sort = crate::types::Sort::Name;
        window.app.reload_view();
        settle(&mut window, 10);
        assert_eq!(window.app.photos.len(), 4);
        // The setting is kept in the library for the next start.
        assert_eq!(window.app.db.setting("stack_time").as_deref(), Some("1"));
    }

    #[test]
    fn look_alikes_stack_once_their_looks_are_found() {
        // Three copies of one picture, taken the same day.
        let (_scratch, mut window) = library("look-alikes", 3);
        window.ctx.enable_accesskit();
        settle(&mut window, 3);
        let tree = window.access.as_ref().expect("the tree");
        let check = tree
            .nodes
            .iter()
            .find(|(_, node)| name(node) == Some("Stack look-alikes"))
            .and_then(|(_, node)| node.bounds())
            .map(center)
            .expect("the look-alike setting");
        click(&mut window, check);
        window.draw_until("the look-alikes stacked", Duration::from_secs(30), |app| {
            app.photos.len() == 1
        });
        assert_eq!(window.app.photos_in_view(), 3);
        assert_eq!(window.app.db.setting("stack_alike").as_deref(), Some("1"));
        assert_eq!(window.app.db.setting("stack_time").as_deref(), Some("0"));
    }

    #[test]
    fn the_library_grids_own_scroll_bar_keeps_clear_of_its_photos() {
        use crate::types::Sort;
        // Sorted by name there is no date strip, so the grid shows its bar.
        let (_scratch, mut window) = library("grid-scroll-bar", 80);
        window.ctx.enable_accesskit();
        window.app.sort = Sort::Name;
        window.app.reload_view();
        // Scrolled a little, so the heading is pinned at the bar's top.
        window.app.jump_to = Some(100.0);
        settle(&mut window, 30);
        // The photos drawn, by name, and the grid's bar, nearest right of
        // them and beside one.
        let shown = |window: &Headless| {
            let photos: Vec<(String, Bounds)> = nodes_of(window, Role::Button)
                .into_iter()
                .filter_map(|(node, rect)| {
                    let name = node.label().filter(|name| name.starts_with("IMG_"))?;
                    Some((name.to_owned(), rect))
                })
                .collect();
            let left = photos
                .iter()
                .map(|(_, rect)| rect.x0)
                .fold(f64::MAX, f64::min);
            let bar = bounds_of(window, Role::ScrollBar)
                .into_iter()
                .filter(|bar| bar.x0 > left)
                .filter(|bar| {
                    photos
                        .iter()
                        .any(|(_, rect)| rect.y0 < bar.y1 && rect.y1 > bar.y0)
                })
                .min_by(|a, b| a.x0.total_cmp(&b.x0));
            (photos, bar)
        };
        let (photos, bar) = shown(&window);
        let bar = bar.expect("the grid's scroll bar beside its photos");
        for (name, rect) in &photos {
            assert!(
                rect.x1 <= bar.x0,
                "{name} at {rect:?} runs under the scroll bar at {bar:?}"
            );
        }

        // Further down, the bar's handle is clear of its top, where the pinned
        // heading is drawn. A click on the bar there still scrolls the grid
        // back up. (egui gives the click to the bar however wide the heading
        // is; that the heading isn't painted over the bar isn't checked.)
        window.app.jump_to = Some(1200.0);
        for _ in 0..10 {
            window.draw(Vec::new());
        }
        let (photos, _) = shown(&window);
        let top_photo = photos
            .iter()
            .filter(|(_, rect)| rect.y0 >= bar.y0)
            .min_by(|a, b| a.1.y0.total_cmp(&b.1.y0))
            .cloned()
            .expect("a photo in view");
        let at = Pos2::new(center(bar).x, bar.y0 as f32 + 10.0);
        click(&mut window, at);
        for _ in 0..10 {
            window.draw(Vec::new());
        }
        let (after, _) = shown(&window);
        let moved = after
            .iter()
            .find(|(name, _)| *name == top_photo.0)
            .is_none_or(|(_, rect)| (rect.y0 - top_photo.1.y0).abs() > 1.0);
        assert!(
            moved,
            "a click on the bar at {at:?}, under the pinned heading, left {} where it was",
            top_photo.0
        );
    }

    #[test]
    fn a_trip_its_days_and_a_story_export_whole_after_a_restart_a_missing_photo_and_a_restore() {
        use crate::story_html::{self, Format, Preflight, Profile, Progress};
        use crate::trip::{Block, DaySource, TripDraft};
        let (scratch, mut window) = library("offline", 4);
        let pics = scratch.dir.join("pics");
        let mut ids: Vec<i64> = window.app.photos.iter().map(|photo| photo.id).collect();
        ids.sort_unstable();

        // A Trip of the four photos over two Days, a Story from the Days, every photo described.
        let db = &window.app.db;
        let trip = db.create_trip(&TripDraft::new("Coast", "+02:00")).unwrap();
        let members = db.add_to_trip(trip, &ids).unwrap();
        let arrival = db.add_trip_day(trip, None, "Arrival").unwrap();
        let harbor = db.add_trip_day(trip, None, "Harbor").unwrap();
        db.move_to_day(trip, Some(arrival), &members[..1], DaySource::User)
            .unwrap();
        db.move_to_day(trip, Some(harbor), &members[1..], DaySource::User)
            .unwrap();
        let (story, _) = db
            .create_story_from_days(trip, "Coast story", None, |day| day.to_string())
            .unwrap();
        let mut blocks = db.story_blocks(story);
        for block in &mut blocks {
            if let Block::Photo(media) = &mut block.block {
                media.alt = "Sea".into();
            }
        }
        db.save_story_blocks(story, db.story(story).unwrap().revision, &blocks)
            .unwrap();

        // What the Export button does once the Story is open, without opening the folder afterwards.
        let out = scratch.dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let export = |window: &mut Headless, skip_missing: bool| -> (Preflight, String, usize) {
            window.app.reload();
            window.app.set_nav(Nav::Trip(trip));
            window.app.open_story(story);
            let mut job = window.app.story_job().expect("the Story opens");
            let check = story_html::preflight(&job.page());
            assert_eq!(
                check.blocked(skip_missing),
                check.missing > 0 && !skip_missing
            );
            job.read_profiles();
            let (folder, outcome) = story_html::export_story(
                &out,
                "Coast story",
                Format::Html,
                Profile::default(),
                &job.page(),
                &Progress::default(),
            )
            .unwrap();
            assert!(window.app.close_story());
            (
                check,
                std::fs::read_to_string(folder.join("index.html")).unwrap(),
                outcome.missing,
            )
        };
        let shown = |html: &str| html.matches("alt=\"Sea\"").count();
        let (check, html, missing) = export(&mut window, false);
        assert_eq!(
            (check.missing, check.no_alt, missing, shown(&html)),
            (0, 0, 0, 4)
        );
        let (arrival_at, harbor_at) = (html.find("Arrival").unwrap(), html.find("Harbor").unwrap());
        assert!(arrival_at < harbor_at, "the Days in their order");

        // Picasso is quit and started again: the same Story exports the same way.
        drop(window);
        let mut window = scratch.open();
        window.app.watcher = None;
        window.settle(4, Duration::from_secs(60));
        assert_eq!(window.app.db.story_blocks(story), blocks);
        let (_, again, _) = export(&mut window, false);
        assert_eq!(shown(&again), 4);

        // A photo's file goes missing: it is named, waits for the user, and is shown as unavailable.
        let gone = pics.join("IMG_003.jpg");
        let bytes = std::fs::read(&gone).unwrap();
        std::fs::remove_file(&gone).unwrap();
        let (check, html, missing) = export(&mut window, true);
        assert_eq!((check.missing, missing, shown(&html)), (1, 1, 3));
        assert!(html.contains("Photo unavailable"));
        std::fs::write(&gone, &bytes).unwrap();
        let (check, html, _) = export(&mut window, false);
        assert_eq!(
            (check.missing, shown(&html)),
            (0, 4),
            "whole again once the file is back"
        );

        // A backup restored into an empty library brings back the Trip, its Days and the Story.
        let backup = crate::backup::create(&scratch.data).unwrap();
        drop(window);
        let (restored, fresh) = Scratch::new(std::env::temp_dir().join(format!(
            "picasso-headless-offline-restored-{}",
            std::process::id()
        )));
        drop(fresh);
        crate::backup::stage_restore(&restored.data, &backup).unwrap();
        let mut window = restored.open();
        window.app.watcher = None;
        window.settle(4, Duration::from_secs(60));
        let days: Vec<String> = window
            .app
            .db
            .trip_days(trip)
            .into_iter()
            .map(|day| day.title)
            .collect();
        assert_eq!(days, ["Arrival", "Harbor"]);
        assert_eq!(window.app.db.story_blocks(story), blocks);
        let (check, restored_html, _) = export(&mut window, false);
        assert_eq!((check.missing, shown(&restored_html)), (0, 4));
        assert!(restored_html.find("Arrival") < restored_html.find("Harbor"));
    }

    /// Opens the named pipe at `path` for writing and closes it, so a reader
    /// waiting on it reads an empty file. Nothing happens when none waits.
    #[cfg(unix)]
    fn answer(path: &Path) {
        use std::os::unix::fs::OpenOptionsExt;
        drop(
            std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(path),
        );
    }

    #[test]
    fn a_photo_whose_file_is_gone_says_so_in_the_viewer_and_shows_once_back() {
        let (_scratch, mut window) = library("gone", 3);
        window.ctx.enable_accesskit();
        let said = |window: &Headless, words: &str| {
            let tree = window.access.as_ref().expect("the tree");
            tree.nodes.iter().any(|(_, node)| name(node) == Some(words))
        };
        // As when its drive is unplugged after the library read it.
        let gone = window.app.photos[1].clone();
        let bytes = std::fs::read(&gone.path).unwrap();
        std::fs::remove_file(&gone.path).unwrap();
        let broken = window.app.photos[2].clone();
        std::fs::write(&broken.path, b"not a picture").unwrap();
        window.app.open_at(1);
        window.draw_until(
            "the photo shown as missing",
            Duration::from_secs(30),
            |app| app.display_failed.contains_key(&gone.id),
        );
        settle(&mut window, 3);
        assert!(
            said(&window, "The file wasn't found"),
            "the viewer says why"
        );
        // Its drive is back while the viewer stays on it: it shows.
        std::fs::write(&gone.path, bytes).unwrap();
        window.draw_until("the photo shown again", Duration::from_secs(30), |app| {
            app.displays.iter().any(|item| item.id == gone.id)
        });
        settle(&mut window, 3);
        assert!(!said(&window, "The file wasn't found"));
        // A file that is there but can't be read says so too, and isn't
        // read again and again while the viewer stays on it.
        window.app.viewer_step(1);
        window.draw_until("the broken photo", Duration::from_secs(30), |app| {
            app.display_failed.contains_key(&broken.id)
        });
        settle(&mut window, 3);
        assert!(said(&window, "This photo can't be shown"));
        assert_eq!(window.app.display_retry(broken.id), None);
        let nonce = window.app.display_nonce[&broken.id];
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            window.draw(Vec::new());
            std::thread::sleep(REFRESH);
        }
        assert_eq!(
            window.app.display_nonce[&broken.id], nonce,
            "not asked for again"
        );
        assert!(said(&window, "This photo can't be shown"));
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
        window.draw_until("the photo beside it", Duration::from_secs(30), |app| {
            app.displays.iter().any(|item| item.id == beside)
        });
        let mut slowest = Duration::ZERO;
        for _ in 0..60 {
            slowest = slowest.max(window.draw(Vec::new()));
            std::thread::sleep(REFRESH);
        }
        assert!(
            slowest < Duration::from_millis(500),
            "a frame took {slowest:?} while a decoder waited"
        );
        assert!(
            window.app.display_inflight.contains_key(&stuck.id),
            "its decoder is still waiting"
        );
        assert!(!window.app.display_failed.contains_key(&stuck.id));

        // The file answers with nothing, each time it is opened: the photo shows as one that can't be read.
        window.draw_until(
            "the photo shown as unreadable",
            Duration::from_secs(30),
            |app| {
                answer(Path::new(&stuck.path));
                !app.display_inflight.contains_key(&stuck.id)
            },
        );
        assert!(window.app.display_failed.contains_key(&stuck.id));
    }
}
