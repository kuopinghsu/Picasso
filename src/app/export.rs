//! Exporting the selected photos, edits applied, into one folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, Margin, RichText};
use rayon::prelude::*;

use crate::edit::{self, ExportOptions, Settings};
use crate::i18n::{tr, trf};
use crate::worker::UiEvent;

use super::editor::{EXPORT_EDGES, EXPORT_FORMATS};
use super::{about, theme, Modal, PicassoApp};

/// Photos exported at the same time; each holds a full-size image.
const EXPORT_THREADS: usize = 3;
/// How long the status bar keeps the note about a finished export.
const KEEP_NOTE_FOR: Duration = Duration::from_secs(120);

/// A photo to export, read from the catalog when the dialog opens.
struct Job {
    source: PathBuf,
    settings: Settings,
    rotation: i32,
}

pub(super) struct ExportDialog {
    jobs: Vec<Job>,
    videos: usize,
    format: usize,
    options: ExportOptions,
    folder: String,
    error: String,
}

/// A batch export running on a background thread, or just finished.
pub(super) struct ExportRun {
    cancel: Arc<AtomicBool>,
    folder: PathBuf,
    total: usize,
    exported: usize,
    failed: usize,
    error: Option<String>,
    finished: Option<Instant>,
}

impl PicassoApp {
    /// Opens the export dialog for `ids`; videos are left out.
    pub(super) fn open_export(&mut self, ids: &[i64]) {
        let mut jobs = Vec::new();
        let mut videos = 0;
        for &id in ids {
            let photo = match self.index_of(id).and_then(|index| self.photos.get(index)) {
                Some(photo) => Some(photo.clone()),
                None => self.db.photo(id),
            };
            let Some(photo) = photo else { continue };
            if photo.is_video() {
                videos += 1;
                continue;
            }
            jobs.push(Job {
                source: PathBuf::from(&photo.path),
                settings: self.edits.get(&id).cloned().unwrap_or_default(),
                rotation: photo.rotation,
            });
        }
        if jobs.is_empty() {
            self.activity = tr("Videos can't be exported").to_string();
            return;
        }
        let folder = self
            .config
            .text("export_folder")
            .map(str::to_owned)
            .or_else(|| crate::db::pictures_dir().map(|dir| dir.join("Picasso Exports").display().to_string()))
            .unwrap_or_default();
        let format = self.config.number("export_format").map_or(0, |n| n as usize).min(EXPORT_FORMATS.len() - 1);
        let edge = self.config.number("export_edge").map_or(0, |n| n as usize).min(EXPORT_EDGES.len() - 1);
        let quality = self.config.number("export_quality").map_or(ExportOptions::default().quality, |n| n.clamp(50.0, 100.0) as u8);
        self.modal = Modal::Export(ExportDialog {
            jobs,
            videos,
            format,
            options: ExportOptions {
                quality,
                long_edge: EXPORT_EDGES[edge],
            },
            folder,
            error: String::new(),
        });
    }

    pub(super) fn export_modal(&mut self, ctx: &egui::Context) {
        let Modal::Export(dialog) = &mut self.modal else { return };
        let mut close = false;
        let mut browse = false;
        let mut start = false;
        let response = egui::Modal::new(egui::Id::unique("export")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(480.0);
            about::title_bar(ui, &self.logo, tr("Export photos"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                let intro = if dialog.jobs.len() == 1 {
                    tr("The photo is saved as a new file with its edits.").to_string()
                } else {
                    trf("{} photos are saved as new files with their edits.", &[&dialog.jobs.len()])
                };
                ui.label(RichText::new(intro).color(theme::muted()));
                if dialog.videos > 0 {
                    ui.label(RichText::new(trf("{} videos are left out.", &[&dialog.videos])).color(theme::muted()));
                }
                ui.add_space(10.0);
                egui::Grid::new("export-grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                    ui.label(tr("Format"));
                    egui::ComboBox::from_id_salt("batch-format")
                        .width(120.0)
                        .selected_text(EXPORT_FORMATS[dialog.format].0)
                        .show_ui(ui, |ui| {
                            for (index, (name, _)) in EXPORT_FORMATS.iter().enumerate() {
                                ui.selectable_value(&mut dialog.format, index, *name);
                            }
                        });
                    ui.end_row();
                    ui.label(tr("Size"));
                    egui::ComboBox::from_id_salt("batch-edge")
                        .width(120.0)
                        .selected_text(edge_label(dialog.options.long_edge))
                        .show_ui(ui, |ui| {
                            for edge in EXPORT_EDGES {
                                ui.selectable_value(&mut dialog.options.long_edge, edge, edge_label(edge));
                            }
                        })
                        .response
                        .on_hover_text(tr("Long edge of the exported file"));
                    ui.end_row();
                    if dialog.format == 0 {
                        ui.label(tr("Quality"));
                        ui.add(egui::Slider::new(&mut dialog.options.quality, 50..=100));
                        ui.end_row();
                    }
                    ui.label(tr("Folder"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut dialog.folder).desired_width(290.0));
                        if ui.button(tr("Browse")).clicked() {
                            browse = true;
                        }
                    });
                    ui.end_row();
                });
                if !dialog.error.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(&dialog.error).color(theme::DANGER));
                }
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(theme::primary_button(tr("Export"))).clicked() {
                        start = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if browse {
            let mut picker = rfd::FileDialog::new().set_title(tr("Choose a folder for the photos"));
            if Path::new(&dialog.folder).is_dir() {
                picker = picker.set_directory(&dialog.folder);
            }
            if let Some(picked) = picker.pick_folder() {
                dialog.folder = picked.display().to_string();
            }
        }
        if close || response.should_close() {
            self.modal = Modal::None;
            return;
        }
        if !start {
            return;
        }
        let folder = PathBuf::from(dialog.folder.trim());
        if folder.as_os_str().is_empty() {
            dialog.error = tr("Choose a folder for the photos").to_string();
            return;
        }
        if let Err(err) = std::fs::create_dir_all(&folder) {
            dialog.error = trf("Couldn't create the folder: {}", &[&err]);
            return;
        }
        let Modal::Export(dialog) = std::mem::replace(&mut self.modal, Modal::None) else { return };
        let edge = EXPORT_EDGES.iter().position(|edge| *edge == dialog.options.long_edge).unwrap_or(0);
        self.config.set("export_folder", folder.display().to_string());
        self.config.set("export_format", dialog.format as f64);
        self.config.set("export_edge", edge as f64);
        self.config.set("export_quality", f64::from(dialog.options.quality));
        self.start_export(dialog, folder, ctx);
    }

    fn start_export(&mut self, dialog: ExportDialog, folder: PathBuf, ctx: &egui::Context) {
        if let Some(run) = &self.export_run {
            run.cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.export_run = Some(ExportRun {
            cancel: cancel.clone(),
            folder: folder.clone(),
            total: dialog.jobs.len(),
            exported: 0,
            failed: 0,
            error: None,
            finished: None,
        });
        let extension = EXPORT_FORMATS[dialog.format].1;
        let options = dialog.options;
        let jobs = dialog.jobs;
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-batch-export".into()).spawn(move || {
            let targets = target_paths(&folder, &jobs, extension);
            let exported = AtomicUsize::new(0);
            let failed = AtomicUsize::new(0);
            let error = Mutex::new(None);
            let send = |finished: bool| {
                let error = error.lock().map(|error| error.clone()).unwrap_or_default();
                let _ = events.send(UiEvent::BatchExported {
                    exported: exported.load(Ordering::Relaxed),
                    failed: failed.load(Ordering::Relaxed),
                    error,
                    finished,
                });
                ctx.request_repaint();
            };
            let run = || {
                jobs.par_iter().zip(targets.par_iter()).for_each(|(job, target)| {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    match edit::export(&job.source, target, &job.settings, job.rotation, options) {
                        Ok(()) => {
                            exported.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(err) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            if let Ok(mut first) = error.lock() {
                                let name = job.source.file_name().unwrap_or_default().to_string_lossy();
                                first.get_or_insert_with(|| format!("{name}: {err}"));
                            }
                        }
                    }
                    send(false);
                });
            };
            match rayon::ThreadPoolBuilder::new().num_threads(EXPORT_THREADS).build() {
                Ok(pool) => pool.install(run),
                Err(_) => run(),
            }
            send(true);
        });
    }

    pub(super) fn on_batch_export(&mut self, exported: usize, failed: usize, error: Option<String>, finished: bool) {
        let Some(run) = &mut self.export_run else { return };
        if run.finished.is_some() {
            return;
        }
        run.exported = exported;
        run.failed = failed;
        run.error = error;
        if finished {
            run.finished = Some(Instant::now());
        }
    }

    /// The status bar note while photos export, with Stop, and after, with
    /// Open folder.
    pub(super) fn export_status(&mut self, ui: &mut egui::Ui) {
        let Some(run) = &self.export_run else { return };
        let small = |text: String| RichText::new(text).size(12.0);
        ui.label(small("·".into()).color(theme::muted()));
        let Some(at) = run.finished else {
            ui.add(egui::Spinner::new().size(12.0));
            let note = trf("Exporting {} of {}…", &[&(run.exported + run.failed + 1).min(run.total), &run.total]);
            ui.label(small(note).color(theme::muted()));
            if ui.link(small(tr("Stop").into())).on_hover_text(tr("Stop after the photos being saved now")).clicked() {
                run.cancel.store(true, Ordering::Relaxed);
            }
            return;
        };
        let left = KEEP_NOTE_FOR.saturating_sub(at.elapsed());
        if left.is_zero() {
            self.export_run = None;
            return;
        }
        ui.ctx().request_repaint_after(left);
        ui.label(small(trf("Exported {} photos", &[&run.exported])).color(theme::muted()));
        if run.failed > 0 {
            let note = ui.label(small(trf("{} couldn't be exported", &[&run.failed])).color(theme::DANGER));
            if let Some(error) = &run.error {
                note.on_hover_text(error);
            }
        }
        if ui.link(small(tr("Open folder").into())).on_hover_text(run.folder.display().to_string()).clicked() {
            let _ = open::that(&run.folder);
        }
    }
}

fn edge_label(edge: Option<u32>) -> String {
    match edge {
        None => tr("Full size").to_string(),
        Some(edge) => trf("{} px", &[&edge]),
    }
}

/// A file name in `folder` for each job: the original name with the new
/// extension, numbered when that name is taken, so no file is overwritten.
fn target_paths(folder: &Path, jobs: &[Job], extension: &str) -> Vec<PathBuf> {
    let mut taken = HashSet::new();
    jobs.iter()
        .map(|job| {
            let stem = job
                .source
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "photo".into());
            let mut number = 1;
            loop {
                let name = if number == 1 {
                    format!("{stem}.{extension}")
                } else {
                    format!("{stem} ({number}).{extension}")
                };
                let target = folder.join(&name);
                if taken.insert(name.to_lowercase()) && !target.exists() {
                    break target;
                }
                number += 1;
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_names_never_overwrite_a_file() {
        let folder = std::env::temp_dir().join(format!("picasso-export-names-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("beach.jpg"), b"taken").unwrap();
        let job = |path: &str| Job {
            source: PathBuf::from(path),
            settings: Settings::default(),
            rotation: 0,
        };
        let jobs = [job("/a/beach.png"), job("/b/Beach.jpg"), job("/a/hill.heic"), job("/b/hill.jpg")];
        let names: Vec<String> = target_paths(&folder, &jobs, "jpg")
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["beach (2).jpg", "Beach (3).jpg", "hill.jpg", "hill (2).jpg"]);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
