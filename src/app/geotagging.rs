//! The Geotag from GPS track dialog: the tracks, or the photos that have
//! GPS, the photos to place, and the camera's time zone. Check works the
//! places out without writing them. The matching runs on its own thread;
//! see [`crate::geotag`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

use eframe::egui::{self, Align, Layout, Margin, RichText};

use crate::db::Database;
use crate::geotag::{self, Kept, Report, Source, Target, NEAR, TRACK_EXTENSIONS};
use crate::i18n::{tr, trf};
use crate::types::{format_utc_offset, parse_utc_offset, Sort};

use super::settings::display_path;
use super::{about, theme, Modal, PicassoApp};

pub(super) struct GeotagDialog {
    photos: Photos,
    /// Use the photos that have GPS instead of tracks.
    from_photos: bool,
    /// Track files, each with whether it is used.
    tracks: Vec<(PathBuf, bool)>,
    offset: String,
    run: Option<Run>,
    /// The last check, with the choices it was made with; it stands until they change.
    check: Option<(String, Result<Report, String>)>,
    result: Option<Result<Report, String>>,
}

/// The photos to geotag.
enum Photos {
    /// A folder and those below it, as typed.
    Folder(String),
    /// A Trip's photos.
    Trip { title: String, files: Vec<PathBuf> },
}

struct Run {
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    finished: mpsc::Receiver<Result<Report, String>>,
    /// The choices of a check; `None` when the places are being written.
    check: Option<String>,
}

impl GeotagDialog {
    fn new(photos: Photos, tracks: Vec<PathBuf>) -> GeotagDialog {
        GeotagDialog {
            photos,
            from_photos: false,
            tracks: tracks.into_iter().map(|path| (path, true)).collect(),
            offset: String::new(),
            run: None,
            check: None,
            result: None,
        }
    }

    /// What to run with, unless something is missing or the time zone can't be read.
    fn options(&self, dry_run: bool) -> Option<geotag::Options> {
        let offset = self.offset.trim();
        let offset = if offset.is_empty() { None } else { Some(parse_utc_offset(offset)?) };
        let source = if self.from_photos {
            Source::Photos
        } else {
            let tracks: Vec<PathBuf> = self.tracks.iter().filter(|(_, used)| *used).map(|(path, _)| path.clone()).collect();
            if tracks.is_empty() {
                return None;
            }
            Source::Tracks(tracks)
        };
        let target = match &self.photos {
            Photos::Folder(folder) if folder.trim().is_empty() => return None,
            Photos::Folder(folder) => Target::Folder(PathBuf::from(folder.trim())),
            Photos::Trip { files, .. } => Target::Files(files.clone()),
        };
        Some(geotag::Options { source, target, offset, dry_run })
    }

    /// The choices as text, to tell whether a check still stands.
    fn choices(&self) -> String {
        let folder = match &self.photos {
            Photos::Folder(folder) => folder.trim(),
            Photos::Trip { .. } => "",
        };
        format!("{}|{:?}|{folder}|{}", self.from_photos, self.tracks, self.offset.trim())
    }

    /// Adds `paths` to the tracks, used, unless they are there.
    fn add_tracks(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            if !self.tracks.iter().any(|(known, _)| *known == path) {
                self.tracks.push((path, true));
            }
        }
    }
}

impl PicassoApp {
    /// Opens the dialog for `folder`, with the tracks beside it.
    pub(super) fn open_geotag(&mut self, folder: &str) {
        let folder = Path::new(folder);
        self.modal = Modal::Geotag(GeotagDialog::new(Photos::Folder(display_path(folder)), geotag::tracks_near(folder)));
    }

    /// Opens the dialog for a Trip's photos, with its tracks.
    pub(super) fn open_trip_geotag(&mut self, trip_id: i64) {
        let Some(trip) = self.db.trip(trip_id) else { return };
        let files = self
            .db
            .trip_photos(trip_id, &crate::trip::MemberQuery::default(), Sort::Name)
            .into_iter()
            .map(|photo| PathBuf::from(photo.path))
            .collect();
        let tracks = self
            .db
            .trip_tracks(trip_id)
            .into_iter()
            .filter(|link| link.track_id.is_some())
            .map(|link| PathBuf::from(link.path))
            .collect();
        self.modal = Modal::Geotag(GeotagDialog::new(Photos::Trip { title: trip.title, files }, tracks));
    }

    pub(super) fn geotag_modal(&mut self, ctx: &egui::Context) {
        let Modal::Geotag(dialog) = &mut self.modal else { return };
        let finished = dialog.run.as_ref().and_then(|run| match run.finished.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(tr("The geotagging stopped unexpectedly").to_string())),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = finished {
            let run = dialog.run.take();
            match run.and_then(|run| run.check) {
                Some(choices) => dialog.check = Some((choices, result)),
                None => {
                    let tagged = result.as_ref().ok().map(|report| report.tagged);
                    dialog.result = Some(result);
                    if let Some(tagged) = tagged {
                        self.activity = trf("Geotagged {} photos", &[&tagged]);
                    }
                    self.reload();
                }
            }
        }
        self.geotag_window(ctx);
    }

    fn geotag_window(&mut self, ctx: &egui::Context) {
        let Modal::Geotag(dialog) = &mut self.modal else { return };
        let running = dialog.run.is_some();
        let (mut close, mut start, mut check) = (false, false, false);
        let offset = dialog.offset.trim();
        let zone_ok = offset.is_empty() || parse_utc_offset(offset).is_some();
        let response = egui::Modal::new(egui::Id::unique("geotag")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(540.0);
            about::title_bar(ui, &self.logo, tr("Geotag from GPS track"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.label(
                    RichText::new(tr(
                        "Each photo gets the place the track was at when it was taken. Picasso keeps the places in the library and in a .picasso-geotags.json file in each folder; the photo files stay as they are. Photos with GPS from the camera keep it.",
                    ))
                    .color(theme::muted()),
                );
                let editable = !running && dialog.result.is_none();
                ui.add_enabled_ui(editable, |ui| {
                    ui.label(RichText::new(tr("Places from")).strong());
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut dialog.from_photos, false, tr("GPS tracks"));
                        ui.radio_value(&mut dialog.from_photos, true, tr("Photos that have GPS"));
                    });
                    if dialog.from_photos {
                        ui.label(
                            RichText::new(tr(
                                "Photos with GPS, such as a phone's, mark where you were, and the others take places between them. Picasso uses those among the photos and the library's others taken within a day of them.",
                            ))
                            .size(11.5)
                            .color(theme::muted()),
                        );
                    } else {
                        tracks_list(ui, dialog);
                    }
                    let mut beside = Vec::new();
                    match &mut dialog.photos {
                        Photos::Folder(folder) => {
                            let label = ui.label(RichText::new(tr("Folder")).strong());
                            let before = folder.clone();
                            path_row(ui, folder, label.id, || {
                                rfd::FileDialog::new().set_title(tr("Choose the photo folder")).pick_folder()
                            });
                            let path = Path::new(folder.trim());
                            if *folder != before && path.is_dir() {
                                beside = geotag::tracks_near(path);
                            }
                        }
                        Photos::Trip { title, files } => {
                            ui.label(RichText::new(tr("Photos")).strong());
                            ui.label(trf("The {} photos of {}", &[&files.len(), title]));
                        }
                    }
                    dialog.add_tracks(beside);
                    ui.label(RichText::new(tr("Camera time zone")).strong());
                    ui.add(egui::TextEdit::singleline(&mut dialog.offset).desired_width(120.0).hint_text("+01:00"));
                    let note = if zone_ok {
                        RichText::new(tr(
                            "The camera clock's offset from UTC, such as +01:00. Add seconds for a clock that was off: +01:02:30. Leave it empty to use the offset each photo records, or this computer's time zone.",
                        ))
                        .size(11.5)
                        .color(theme::muted())
                    } else {
                        RichText::new(tr("Type an offset such as +01:00")).size(11.5).color(theme::DANGER)
                    };
                    ui.label(note);
                });
                if let Some(run) = &dialog.run {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        let (done, total) = (run.done.load(Ordering::Relaxed), run.total.load(Ordering::Relaxed));
                        ui.label(trf("Reading photos… {} of {}", &[&done, &total]));
                    });
                }
                let choices = dialog.choices();
                if dialog.check.as_ref().is_some_and(|(made, _)| *made != choices) {
                    dialog.check = None;
                }
                match (&dialog.result, &dialog.check) {
                    (Some(Ok(report)), _) => report_lines(ui, report, false),
                    (Some(Err(err)), _) | (None, Some((_, Err(err)))) => {
                        ui.label(RichText::new(err).color(theme::DANGER));
                    }
                    (None, Some((_, Ok(report)))) => {
                        report_lines(ui, report, true);
                        if let Some(better) = report.suggestion.filter(|_| !running) {
                            let zone = format_utc_offset(better.offset);
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(trf(
                                        "The time zone {} places {} of the {} photos taken by a camera clock; the one used places {}.",
                                        &[&zone, &better.places, &better.of, &better.now],
                                    ))
                                    .strong(),
                                );
                                if ui.button(trf("Use {}", &[&zone])).clicked() {
                                    dialog.offset = zone.clone();
                                    check = true;
                                }
                            });
                        }
                        each_photo(ui, report);
                        ui.label(RichText::new(tr("Nothing has been written yet.")).size(11.5).color(theme::muted()));
                    }
                    (None, None) => {}
                }
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if dialog.result.is_some() {
                        if ui.add(theme::primary_button(tr("Done"))).clicked() {
                            close = true;
                        }
                        return;
                    }
                    let ready = !running && dialog.options(false).is_some();
                    let mut geotag = ui.add_enabled(ready, theme::primary_button(tr("Geotag")));
                    if !running {
                        geotag = geotag.on_disabled_hover_text(tr(
                            "Choose a GPS track, or use the photos that have GPS, and check the camera time zone.",
                        ));
                    }
                    if geotag.clicked() {
                        start = true;
                    }
                    let button = ui.add_enabled(ready, egui::Button::new(tr("Check")));
                    if button.on_hover_text(tr("See which photos get a place, without writing anything")).clicked() {
                        check = true;
                    }
                    if ui.add_enabled(!running, egui::Button::new(tr("Cancel"))).clicked() {
                        close = true;
                    }
                });
            });
        });
        if start || check {
            if let Some(options) = dialog.options(check && !start) {
                let choices = (check && !start).then(|| dialog.choices());
                dialog.run = Some(spawn(options, self.data_dir.join("picasso.db"), ctx.clone(), choices));
            }
        }
        // The library is being written while it runs, so it can't be closed.
        if !running && (close || response.should_close()) {
            self.modal = Modal::None;
        }
    }
}

/// The tracks, each with a box to use it, and a button to add more.
fn tracks_list(ui: &mut egui::Ui, dialog: &mut GeotagDialog) {
    if dialog.tracks.is_empty() {
        ui.label(
            RichText::new(tr("No track beside these photos. Add .gpx, .kml or .kmz files, or use the photos that have GPS."))
                .size(11.5)
                .color(theme::muted()),
        );
    }
    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
        for (path, used) in &mut dialog.tracks {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            ui.checkbox(used, name).on_hover_text(display_path(path));
        }
    });
    if ui.button(tr("Add tracks…")).clicked() {
        let picked = rfd::FileDialog::new()
            .set_title(tr("Choose a GPS track"))
            .add_filter(tr("GPS tracks"), &TRACK_EXTENSIONS)
            .pick_files();
        dialog.add_tracks(picked.unwrap_or_default());
    }
}

/// A path box with a Choose… button that fills it from `pick`.
fn path_row(ui: &mut egui::Ui, text: &mut String, label: egui::Id, pick: impl FnOnce() -> Option<PathBuf>) {
    ui.horizontal(|ui| {
        let width = ui.available_width() - 90.0;
        ui.add(egui::TextEdit::singleline(text).desired_width(width)).labelled_by(label);
        if ui.button(tr("Choose…")).clicked() {
            if let Some(path) = pick() {
                *text = display_path(&path);
            }
        }
    });
}

fn utc(time: i64) -> String {
    chrono::DateTime::from_timestamp(time, 0).map(|time| time.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

/// What a run did, or for a check would do.
fn report_lines(ui: &mut egui::Ui, report: &Report, check: bool) {
    let (start, end) = report.track_span;
    if report.gps_photos > 0 {
        ui.label(trf("Places from {} photos with GPS · {} – {} UTC", &[&report.gps_photos, &utc(start), &utc(end)]));
    } else {
        let name = if report.track_name.is_empty() { tr("GPS track").to_string() } else { report.track_name.clone() };
        ui.label(trf("Track: {} · {} – {} UTC", &[&name, &utc(start), &utc(end)]));
    }
    if let Some((first, last)) = report.photo_span {
        ui.label(trf("Photos without GPS: taken {} – {} UTC", &[&utc(first), &utc(last)]));
    }
    ui.label(RichText::new(trf(if check { "Would tag {} photos" } else { "Tagged {} photos" }, &[&report.tagged])).strong());
    if report.had_gps > 0 {
        ui.label(RichText::new(trf("{} already had GPS from the camera", &[&report.had_gps])).color(theme::muted()));
    }
    if report.hand_pinned > 0 {
        ui.label(RichText::new(trf("{} were pinned by hand and keep their place", &[&report.hand_pinned])).color(theme::muted()));
    }
    if report.unmatched > 0 {
        let minutes = NEAR / 60;
        ui.label(RichText::new(trf("{} had no track point within {} minutes", &[&report.unmatched, &minutes])).color(theme::muted()));
    }
    if !check {
        match &report.library_error {
            Some(err) => {
                ui.label(RichText::new(trf("The library wasn't updated: {}", &[err])).color(theme::DANGER));
            }
            None if report.library > 0 => {
                ui.label(RichText::new(trf("Pinned {} photos in the library", &[&report.library])).color(theme::muted()));
            }
            None => {}
        }
        for (path, err) in &report.failed {
            let folder = path.parent().map(display_path).unwrap_or_default();
            ui.label(RichText::new(trf("Couldn't write the geotag file in {}: {}", &[&folder, err])).color(theme::DANGER));
        }
    }
    if report.zone_guessed > 0 {
        let text = trf(
            "{} photos record no time zone, so this computer's ({}) was used. If they were taken elsewhere, geotag their folder again with the camera time zone.",
            &[&report.zone_guessed, &geotag::local_offset()],
        );
        ui.label(RichText::new(text).color(theme::muted()));
    }
    if report.tagged == 0 && report.unmatched > 0 && report.suggestion.is_none() {
        ui.label(
            RichText::new(tr("None of the photos fall within the track. Compare the times above and adjust the camera time zone."))
                .color(theme::DANGER),
        );
    }
}

/// Each photo of a check: when it was taken, and the place it would get or why not.
fn each_photo(ui: &mut egui::Ui, report: &Report) {
    egui::CollapsingHeader::new(tr("Each photo")).id_salt("geotag-each").show(ui, |ui| {
        let height = ui.text_style_height(&egui::TextStyle::Body);
        egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, true]).show_rows(ui, height, report.matches.len(), |ui, rows| {
            for item in &report.matches[rows] {
                let what = match (item.kept, item.place) {
                    (Some(Kept::Gps), _) => tr("Has GPS from the camera").to_string(),
                    (Some(Kept::Pin), _) => tr("Pinned by hand").to_string(),
                    (None, Some((lat, lon))) => format!("{lat:.5}, {lon:.5}"),
                    (None, None) => tr("No track point near").to_string(),
                };
                let name = item.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(utc(item.time)).monospace().size(11.5).color(theme::muted()));
                    ui.label(RichText::new(name).size(11.5));
                    ui.label(RichText::new(what).size(11.5).color(if item.place.is_some() { theme::text() } else { theme::muted() }));
                });
            }
        });
    });
}

fn spawn(options: geotag::Options, db_path: PathBuf, ctx: egui::Context, check: Option<String>) -> Run {
    let (done, total) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (sender, finished) = mpsc::channel();
    let (done_in, total_in) = (done.clone(), total.clone());
    let _ = std::thread::Builder::new().name("picasso-geotag".into()).spawn(move || {
        let db = Database::open(&db_path);
        let progress = |count: usize, all: usize| {
            done_in.store(count, Ordering::Relaxed);
            total_in.store(all, Ordering::Relaxed);
            ctx.request_repaint();
        };
        let mut result = geotag::run(&options, db.as_ref().ok(), &progress);
        if let (Ok(report), Err(err)) = (&mut result, &db) {
            report.library_error = Some(err.clone());
        }
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    Run { done, total, finished, check }
}
