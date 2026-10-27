//! Renaming the photos of a folder from a pattern of exiftool keywords, with
//! every new name shown before any file changes. Each renaming is listed in
//! the folder, so Undo can put back earlier ones, also after Picasso closes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use eframe::egui::{self, Align, Layout, Margin, RichText, TextStyle};

use crate::i18n::{tr, trf};
use crate::rename::{self, Outcome};
use crate::types::{Nav, Sort};

use super::settings::display_path;
use super::{about, theme, Modal, PicassoApp};

pub(super) struct RenameDialog {
    dir: String,
    /// The folder's photos, by name, and the other names in it.
    files: Vec<rename::File>,
    others: Vec<String>,
    pattern: String,
    date_format: String,
    /// The time offset the dates are moved by, for a camera clock that was
    /// off; not kept for next time, since it belongs to one camera's photos.
    shift: String,
    plan: Option<Plan>,
    /// The renamings listed in the folder, oldest first.
    history: Vec<rename::Batch>,
    /// What the last action did, and whether it failed.
    message: Option<(String, bool)>,
    /// Whether a scan was running when the files were read; they are read
    /// again once it ends.
    scanning: bool,
    /// Files being renamed on another thread.
    job: Option<Job>,
    /// Whether the window was closed during the renaming; it closes once
    /// the renaming is done, so no file is left with a temporary name.
    quit: bool,
}

/// The pattern, date format, and time offset a plan was made from, and what
/// becomes of each file.
type Plan = ([String; 3], Result<Vec<Outcome>, String>);

/// A renaming under way: each file as (photo, old name, new name), what it
/// is for, and the outcome once the files have their names.
struct Job {
    renames: Vec<(Option<i64>, String, String)>,
    purpose: Purpose,
    result: Receiver<Result<(), String>>,
}

enum Purpose {
    Rename,
    /// Undo, with the files of the renaming that keep their names, and how
    /// many were no longer there.
    Undo { left: Vec<(String, String)>, gone: usize },
}

impl PicassoApp {
    pub(super) fn open_renamer(&mut self, dir: &str) {
        let (files, others) = folder_files(&self.db, dir);
        let (history, message) = match rename::history(Path::new(dir)) {
            Ok(history) => (history, None),
            Err(err) => (Vec::new(), Some((err, true))),
        };
        self.modal = Modal::Rename(RenameDialog {
            dir: dir.to_string(),
            files,
            others,
            pattern: self.config.text("rename_pattern").unwrap_or(rename::PATTERN).to_string(),
            date_format: self.config.text("rename_date_format").unwrap_or(rename::DATE_FORMAT).to_string(),
            shift: String::new(),
            plan: None,
            history,
            message,
            scanning: self.scans > 0,
            job: None,
            quit: false,
        });
    }

    /// Whether files are being renamed, when the folder watch must wait so a
    /// scan doesn't see them half done.
    pub(super) fn renaming(&self) -> bool {
        matches!(&self.modal, Modal::Rename(dialog) if dialog.job.is_some())
    }

    pub(super) fn renamer_modal(&mut self, ctx: &egui::Context) {
        let finished = match &self.modal {
            Modal::Rename(RenameDialog { job: Some(job), .. }) => match job.result.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(String::new())),
            },
            _ => None,
        };
        if let Some(result) = finished {
            self.finish(result);
            if matches!(&self.modal, Modal::Rename(dialog) if dialog.quit) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        let scanning = self.scans > 0;
        let Modal::Rename(dialog) = &mut self.modal else { return };
        if dialog.scanning && !scanning {
            (dialog.files, dialog.others) = folder_files(&self.db, &dialog.dir);
            dialog.plan = None;
        }
        dialog.scanning = scanning;
        let inputs = [dialog.pattern.clone(), dialog.date_format.clone(), dialog.shift.clone()];
        if dialog.plan.as_ref().is_none_or(|(made_from, _)| *made_from != inputs) {
            let plan = rename::plan(&dialog.pattern, &dialog.date_format, &dialog.shift, &dialog.files, &dialog.others);
            dialog.plan = Some((inputs, plan));
        }
        let working = dialog.job.is_some();
        if working {
            ctx.request_repaint_after(Duration::from_millis(100));
            if ctx.input(|input| input.viewport().close_requested()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                dialog.quit = true;
            }
        }
        let (mut close, mut apply, mut undo) = (false, false, false);
        let response = egui::Modal::new(egui::Id::unique("rename-files")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(640.0);
            about::title_bar(ui, &self.logo, tr("Rename files"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.label(RichText::new(display_path(Path::new(&dialog.dir))).color(theme::muted()));
                ui.label(tr("New name"));
                ui.add(egui::TextEdit::singleline(&mut dialog.pattern).font(TextStyle::Monospace).desired_width(f32::INFINITY));
                ui.horizontal_wrapped(|ui| {
                    for keyword in rename::KEYWORDS {
                        if ui.button(RichText::new(keyword).monospace()).on_hover_text(keyword_hint(keyword)).clicked() {
                            dialog.pattern.push_str(keyword);
                        }
                    }
                });
                ui.label(RichText::new(tr("Click a keyword to add it to the name.")).small().color(theme::muted()));
                ui.horizontal(|ui| {
                    ui.label(tr("Date format"));
                    ui.add(egui::TextEdit::singleline(&mut dialog.date_format).font(TextStyle::Monospace).desired_width(220.0))
                        .on_hover_text(tr("%Y year, %m month, %d day, %H hour, %M minute, %S second"));
                    ui.add_space(12.0);
                    ui.label(tr("Time offset"));
                    ui.add(egui::TextEdit::singleline(&mut dialog.shift).font(TextStyle::Monospace).hint_text("+0:00").desired_width(110.0))
                        .on_hover_text(tr("Moves the dates for a camera clock that was off: +1:30 is an hour and a half later, -0:00:10 ten seconds earlier, +2 0:00 two days later"));
                });
                ui.separator();
                let renames = match dialog.plan.as_ref().map(|(_, plan)| plan) {
                    Some(Ok(outcomes)) => preview(ui, &dialog.files, outcomes),
                    Some(Err(message)) => {
                        ui.label(RichText::new(message).color(theme::DANGER));
                        0
                    }
                    None => 0,
                };
                if let Some((message, failed)) = &dialog.message {
                    ui.label(RichText::new(message).color(if *failed { theme::DANGER } else { theme::text() }));
                }
                let note = trf("Picasso renames the files themselves and lists the old names in {}, so Undo can put them back.", &[&rename::LOG]);
                ui.label(RichText::new(note).small().color(theme::muted()));
                ui.add_space(4.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let wait = tr("Wait for the scan to finish");
                    let mut rename = ui.add_enabled(renames > 0 && !scanning && !working, theme::primary_button(tr("Rename")));
                    if scanning {
                        rename = rename.on_disabled_hover_text(wait);
                    }
                    if rename.clicked() {
                        apply = true;
                    }
                    if let Some(last) = dialog.history.last() {
                        let hint = trf("Put back the names {} files had before {}", &[&last.files.len(), &last.at]);
                        let mut button = ui.add_enabled(!scanning && !working, egui::Button::new(tr("Undo"))).on_hover_text(hint);
                        if scanning {
                            button = button.on_disabled_hover_text(wait);
                        }
                        if button.clicked() {
                            undo = true;
                        }
                    }
                    if ui.add_enabled(!working, egui::Button::new(tr("Close"))).clicked() {
                        close = true;
                    }
                    if working {
                        ui.label(RichText::new(tr("Renaming…")).color(theme::muted()));
                        ui.add(egui::Spinner::new());
                    }
                });
            });
        });
        if working {
            return;
        }
        if apply {
            self.rename_as_planned();
        } else if undo {
            self.undo_renaming();
        } else if close || response.should_close() {
            self.modal = Modal::None;
        }
    }

    fn rename_as_planned(&mut self) {
        let Modal::Rename(dialog) = &self.modal else { return };
        let Some((_, Ok(outcomes))) = &dialog.plan else { return };
        let mut renames: Vec<(Option<i64>, String, String)> = Vec::new();
        for (file, outcome) in dialog.files.iter().zip(outcomes) {
            if let Outcome::Rename(name) = outcome {
                renames.push((Some(file.id), file.name.clone(), name.clone()));
                for companion in &file.companions {
                    renames.push((None, companion.clone(), rename::companion_name(companion, &file.name, name)));
                }
            }
        }
        self.start(renames, Purpose::Rename);
    }

    /// Puts back the names of the newest renaming the folder lists. Files
    /// that are no longer there are left out, and files whose earlier name
    /// another file has now stay listed for later.
    fn undo_renaming(&mut self) {
        let Modal::Rename(dialog) = &self.modal else { return };
        let Some(last) = dialog.history.last() else { return };
        let names: Vec<String> = std::fs::read_dir(&dialog.dir)
            .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        let undo = rename::undo_plan(last, &names);
        let ids: HashMap<String, i64> = dialog.files.iter().map(|file| (rename::key(&file.name), file.id)).collect();
        let renames = undo.back.into_iter().map(|(from, to)| (ids.get(&rename::key(&from)).copied(), from, to)).collect();
        self.start(renames, Purpose::Undo { left: undo.left, gone: undo.gone });
    }

    /// Renames files of the dialog's folder on another thread; `finish`
    /// takes it from there.
    fn start(&mut self, renames: Vec<(Option<i64>, String, String)>, purpose: Purpose) {
        let Modal::Rename(dialog) = &mut self.modal else { return };
        let (sender, result) = mpsc::channel();
        if renames.is_empty() {
            let _ = sender.send(Ok(()));
        } else {
            let dir = PathBuf::from(&dialog.dir);
            let pairs: Vec<(String, String)> = renames.iter().map(|(_, from, to)| (from.clone(), to.clone())).collect();
            std::thread::spawn(move || {
                let renamed = std::panic::catch_unwind(|| rename::apply(&dir, &pairs)).unwrap_or_else(|panic| {
                    Err(panic.downcast_ref::<&str>().map(|text| text.to_string()).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_default())
                });
                let _ = sender.send(renamed);
            });
        }
        dialog.message = None;
        dialog.job = Some(Job { renames, purpose, result });
    }

    /// Moves the renamed photos in the library to their new names, or the
    /// files back when that fails, and lists the renaming in the folder.
    fn finish(&mut self, result: Result<(), String>) {
        let Modal::Rename(dialog) = &mut self.modal else { return };
        let Some(job) = dialog.job.take() else { return };
        let dir = PathBuf::from(&dialog.dir);
        let pairs: Vec<(String, String)> = job.renames.iter().map(|(_, from, to)| (from.clone(), to.clone())).collect();
        let rows: Vec<(i64, String, String)> = job
            .renames
            .iter()
            .filter_map(|(id, _, to)| Some(((*id)?, dir.join(to).to_string_lossy().into_owned(), to.clone())))
            .collect();
        let result = result.and_then(|()| {
            self.db.rename_photos(&rows).map_err(|err| {
                let back: Vec<(String, String)> = pairs.iter().map(|(from, to)| (to.clone(), from.clone())).collect();
                match rename::apply(&dir, &back) {
                    Ok(()) => err,
                    Err(stuck) => format!("{err}; {stuck}"),
                }
            })
        });
        if let Err(err) = result {
            (dialog.files, dialog.others) = folder_files(&self.db, &dialog.dir);
            dialog.plan = None;
            dialog.message = Some((trf("Couldn't rename the files: {}", &[&err]), true));
            return;
        }
        if let Err(err) = crate::geotag::Sidecar::rename(&dir, &pairs) {
            self.activity = format!("{}: {err}", crate::geotag::SIDECAR);
        }
        for (id, ..) in &rows {
            self.thumb_failed.remove(id);
        }
        if let Some(dupes) = &mut self.dupes {
            dupes.renamed(&rows);
        }
        let (mut parts, listed, remember) = match job.purpose {
            Purpose::Rename => {
                let count = rows.len();
                let message = trf(if count == 1 { "Renamed {} file" } else { "Renamed {} files" }, &[&count]);
                (vec![message], rename::record(&dir, &pairs), true)
            }
            Purpose::Undo { left, gone } => {
                let mut parts = Vec::new();
                if !pairs.is_empty() {
                    parts.push(tr("Put back the earlier names").to_string());
                }
                if gone > 0 {
                    parts.push(trf("{} files weren't there anymore", &[&gone]));
                }
                if !left.is_empty() {
                    parts.push(trf("{} files kept their names: another file has the earlier name", &[&left.len()]));
                }
                (parts, rename::undone(&dir, &left), false)
            }
        };
        if let Err(err) = &listed {
            parts.push(err.clone());
        }
        self.renamed(parts.join(" · "), remember, listed.is_err());
    }

    /// Shows `message` after a renaming or an undo, with the folder and its
    /// list read again. `remember` keeps the pattern for next time; `failed`
    /// means the list couldn't be written.
    fn renamed(&mut self, message: String, remember: bool, failed: bool) {
        let Modal::Rename(dialog) = &mut self.modal else { return };
        (dialog.files, dialog.others) = folder_files(&self.db, &dialog.dir);
        dialog.plan = None;
        dialog.history = rename::history(Path::new(&dialog.dir)).unwrap_or_default();
        if remember {
            self.config.set("rename_pattern", dialog.pattern.clone());
            self.config.set("rename_date_format", dialog.date_format.clone());
        }
        self.activity = message.clone();
        dialog.message = Some((message, failed));
        self.reload();
    }
}

/// Each photo's name and what becomes of it, in rows drawn only when in
/// view. Returns how many will be renamed.
fn preview(ui: &mut egui::Ui, files: &[rename::File], outcomes: &[Outcome]) -> usize {
    let renames = outcomes.iter().filter(|outcome| matches!(outcome, Outcome::Rename(_))).count();
    if files.is_empty() {
        ui.label(RichText::new(tr("No photos in this folder to rename")).color(theme::muted()));
        return 0;
    }
    let row_height = ui.text_style_height(&TextStyle::Body) + 4.0;
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 2.0);
    egui::ScrollArea::vertical().max_height(280.0).auto_shrink([false, true]).show_rows(ui, row_height, files.len(), |ui, rows| {
        for (file, outcome) in files[rows.clone()].iter().zip(&outcomes[rows]) {
            ui.horizontal(|ui| {
                let size = egui::vec2(270.0, row_height);
                ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(&file.name).truncate());
                });
                let (mut text, color) = match outcome {
                    Outcome::Rename(name) => (format!("→  {name}"), theme::text()),
                    Outcome::Same => (tr("unchanged").to_string(), theme::muted()),
                    Outcome::Fails(reason) => (reason.clone(), theme::DANGER),
                };
                if !file.companions.is_empty() && matches!(outcome, Outcome::Rename(_)) {
                    let extensions: Vec<String> = file
                        .companions
                        .iter()
                        .map(|companion| Path::new(companion).extension().map(|ext| format!(".{}", ext.to_string_lossy())).unwrap_or_default())
                        .collect();
                    text = format!("{text}  + {}", extensions.join(" "));
                }
                ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
            });
        }
    });
    ui.spacing_mut().item_spacing.y = spacing;
    ui.label(trf("{} of {} files will be renamed", &[&renames, &files.len()]));
    renames
}

fn keyword_hint(keyword: &str) -> &'static str {
    match keyword {
        "${DateTimeOriginal}" => tr("Date taken, in the date format below"),
        "${FileModifyDate}" => tr("Date the file was last changed, in the date format below"),
        "${Model}" => tr("Camera, as the Info panel names it"),
        "%f" => tr("Original file name, without the extension"),
        "%-03c" => tr("Copy number when the name is taken: nothing for the first, then -001, -002, and so on"),
        "%le" => tr("Extension in lowercase"),
        _ => tr("Extension as it is"),
    }
}

/// The library's photos directly in `dir` that are there on disk, by name,
/// each with the files that go with it, and the names of everything else in
/// the folder.
fn folder_files(db: &crate::db::Database, dir: &str) -> (Vec<rename::File>, Vec<String>) {
    let on_disk: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    let present: HashSet<&str> = on_disk.iter().map(String::as_str).collect();
    let mut files: Vec<rename::File> = db
        .photos(&Nav::Dir(dir.to_string()), "", Sort::Name)
        .into_iter()
        .filter(|photo| Path::new(&photo.path).parent() == Some(Path::new(dir)) && present.contains(photo.filename.as_str()))
        .map(|photo| rename::File {
            id: photo.id,
            name: photo.filename,
            taken_at: photo.taken_at,
            modified_at: photo.modified_at,
            camera: photo.camera,
            companions: Vec::new(),
        })
        .collect();
    files.sort_by_key(|file| file.name.to_lowercase());
    let names: HashSet<&str> = files.iter().map(|file| file.name.as_str()).collect();
    let others = on_disk.iter().filter(|name| !names.contains(name.as_str())).cloned().collect();
    let others = rename::attach(&mut files, others);
    (files, others)
}
