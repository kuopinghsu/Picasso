//! The Geotag from GPS track dialog: a track, a folder, and the camera's
//! time zone. The matching runs on its own thread; see [`crate::geotag`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

use eframe::egui::{self, Align, Layout, Margin, RichText};

use crate::db::Database;
use crate::geotag::{self, Report, NEAR, TRACK_EXTENSIONS};
use crate::i18n::{tr, trf};
use crate::types::parse_utc_offset;

use super::settings::display_path;
use super::{about, theme, Modal, PicassoApp};

pub(super) struct GeotagDialog {
    track: String,
    folder: String,
    offset: String,
    run: Option<Run>,
    result: Option<Result<Report, String>>,
}

struct Run {
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    finished: mpsc::Receiver<Result<Report, String>>,
}

impl PicassoApp {
    /// Opens the dialog for `folder`, with a track beside it filled in.
    pub(super) fn open_geotag(&mut self, folder: &str) {
        let folder = Path::new(folder);
        let track = geotag::tracks_near(folder).first().map(|path| display_path(path)).unwrap_or_default();
        self.modal = Modal::Geotag(GeotagDialog {
            track,
            folder: display_path(folder),
            offset: String::new(),
            run: None,
            result: None,
        });
    }

    pub(super) fn geotag_modal(&mut self, ctx: &egui::Context) {
        let Modal::Geotag(dialog) = &mut self.modal else { return };
        let finished = dialog.run.as_ref().and_then(|run| match run.finished.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(tr("The geotagging stopped unexpectedly").to_string())),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = finished {
            let tagged = result.as_ref().ok().map(|report| report.tagged);
            dialog.run = None;
            dialog.result = Some(result);
            if let Some(tagged) = tagged {
                self.activity = trf("Geotagged {} photos", &[&tagged]);
            }
            self.reload();
        }
        self.geotag_window(ctx);
    }

    fn geotag_window(&mut self, ctx: &egui::Context) {
        let Modal::Geotag(dialog) = &mut self.modal else { return };
        let running = dialog.run.is_some();
        let (mut close, mut start) = (false, false);
        let offset = dialog.offset.trim();
        let zone = if offset.is_empty() { Some(None) } else { parse_utc_offset(offset).map(Some) };
        let response = egui::Modal::new(egui::Id::unique("geotag")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(520.0);
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
                    ui.label(RichText::new(tr("GPS track")).strong());
                    path_row(ui, &mut dialog.track, || {
                        rfd::FileDialog::new()
                            .set_title(tr("Choose a GPS track"))
                            .add_filter(tr("GPS tracks"), &TRACK_EXTENSIONS)
                            .pick_file()
                    });
                    ui.label(RichText::new(tr("Folder")).strong());
                    path_row(ui, &mut dialog.folder, || rfd::FileDialog::new().set_title(tr("Choose the photo folder")).pick_folder());
                    ui.label(RichText::new(tr("Camera time zone")).strong());
                    ui.add(egui::TextEdit::singleline(&mut dialog.offset).desired_width(120.0).hint_text("+01:00"));
                    let note = if zone.is_some() {
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
                match &dialog.result {
                    Some(Ok(report)) => report_lines(ui, report),
                    Some(Err(err)) => {
                        ui.label(RichText::new(err).color(theme::DANGER));
                    }
                    None => {}
                }
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if dialog.result.is_some() {
                        if ui.add(theme::primary_button(tr("Done"))).clicked() {
                            close = true;
                        }
                        return;
                    }
                    let ready = !running && zone.is_some() && !dialog.track.trim().is_empty() && !dialog.folder.trim().is_empty();
                    if ui.add_enabled(ready, theme::primary_button(tr("Geotag"))).clicked() {
                        start = true;
                    }
                    if ui.add_enabled(!running, egui::Button::new(tr("Cancel"))).clicked() {
                        close = true;
                    }
                });
            });
        });
        if start {
            if let Some(zone) = zone {
                let options = geotag::Options {
                    track: PathBuf::from(dialog.track.trim()),
                    folder: PathBuf::from(dialog.folder.trim()),
                    offset: zone,
                    dry_run: false,
                };
                dialog.run = Some(spawn(options, self.data_dir.join("picasso.db"), ctx.clone()));
            }
        }
        // The library is being written while it runs, so it can't be closed.
        if !running && (close || response.should_close()) {
            self.modal = Modal::None;
        }
    }
}

/// A path box with a Choose… button that fills it from `pick`.
fn path_row(ui: &mut egui::Ui, text: &mut String, pick: impl FnOnce() -> Option<PathBuf>) {
    ui.horizontal(|ui| {
        let width = ui.available_width() - 90.0;
        ui.add(egui::TextEdit::singleline(text).desired_width(width));
        if ui.button(tr("Choose…")).clicked() {
            if let Some(path) = pick() {
                *text = display_path(&path);
            }
        }
    });
}

fn report_lines(ui: &mut egui::Ui, report: &Report) {
    let utc = |time: i64| {
        chrono::DateTime::from_timestamp(time, 0).map(|time| time.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
    };
    let (start, end) = report.track_span;
    let name = if report.track_name.is_empty() { tr("GPS track").to_string() } else { report.track_name.clone() };
    ui.label(trf("Track: {} · {} – {} UTC", &[&name, &utc(start), &utc(end)]));
    if let Some((first, last)) = report.photo_span {
        ui.label(trf("Photos without GPS: taken {} – {} UTC", &[&utc(first), &utc(last)]));
    }
    ui.label(RichText::new(trf("Tagged {} photos", &[&report.tagged])).strong());
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
    if report.zone_guessed > 0 {
        let text = trf(
            "{} photos record no time zone, so this computer's ({}) was used. If they were taken elsewhere, geotag their folder again with the camera time zone.",
            &[&report.zone_guessed, &geotag::local_offset()],
        );
        ui.label(RichText::new(text).color(theme::muted()));
    }
    if report.tagged == 0 && report.unmatched > 0 {
        ui.label(
            RichText::new(tr("None of the photos fall within the track. Compare the times above and adjust the camera time zone."))
                .color(theme::DANGER),
        );
    }
}

fn spawn(options: geotag::Options, db_path: PathBuf, ctx: egui::Context) -> Run {
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
    Run { done, total, finished }
}
