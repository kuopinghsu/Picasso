//! The files a scan could not read, listed from the status bar link.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use eframe::egui::{self, Align, Layout, Margin, RichText, Stroke, Vec2};

use crate::i18n::{tr, trf};
use crate::shell;
use crate::types::Unreadable;

use super::{about, theme, Modal, PicassoApp};

const ROW_HEIGHT: f32 = 46.0;

enum Action {
    Reveal(String),
    Trash(Vec<String>),
}

impl PicassoApp {
    pub(super) fn unreadable_count(&self) -> usize {
        self.unreadable.values().map(Vec::len).sum()
    }

    pub(super) fn unreadable_modal(&mut self, ctx: &egui::Context) {
        let Modal::Unreadable { confirm_all } = self.modal else { return };
        let mut items: Vec<&Unreadable> = self.unreadable.values().flatten().collect();
        items.sort_by(|a, b| a.path.cmp(&b.path));
        let files: Vec<String> = items.iter().filter(|item| !item.folder).map(|item| item.path.clone()).collect();
        let mut confirm = confirm_all;
        let mut close = false;
        let mut action = None;
        let response = egui::Modal::new(egui::Id::unique("unreadable")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(700.0);
            about::title_bar(ui, &self.logo, tr("Files that could not be read"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.label(
                    RichText::new(tr(
                        "Picasso skips these files. Open their folder to check them, or move the ones you don't need to the trash.",
                    ))
                    .color(theme::muted()),
                );
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .auto_shrink([false, true])
                    .show_rows(ui, ROW_HEIGHT, items.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for item in &items[range] {
                            if let Some(chosen) = row(ui, item) {
                                action = Some(chosen);
                            }
                        }
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if confirm {
                        ui.label(trf("Move {} files to the system trash?", &[&files.len()]));
                        if ui.add(theme::danger_button(tr("Move to trash"))).clicked() {
                            action = Some(Action::Trash(files.clone()));
                            confirm = false;
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            confirm = false;
                        }
                        return;
                    }
                    if !files.is_empty() && ui.add(theme::danger_button(tr("Move all to trash"))).clicked() {
                        if self.confirm_trash {
                            confirm = true;
                        } else {
                            action = Some(Action::Trash(files.clone()));
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(tr("Close")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        });
        match action {
            Some(Action::Reveal(path)) => shell::reveal(&path),
            Some(Action::Trash(paths)) => self.trash_unreadable(&paths),
            None => {}
        }
        self.modal = if close || response.should_close() || self.unreadable.is_empty() {
            Modal::None
        } else {
            Modal::Unreadable { confirm_all: confirm }
        };
    }

    /// Moves the listed files among `paths` to the system trash, with the
    /// status bar's Undo, and drops them from the list.
    fn trash_unreadable(&mut self, paths: &[String]) {
        let wanted: HashSet<&String> = paths.iter().collect();
        let mut trashed = Vec::new();
        let mut failed = None;
        for (folder, items) in &mut self.unreadable {
            items.retain(|item| {
                if item.folder || !wanted.contains(&item.path) {
                    return true;
                }
                match trash::delete(&item.path) {
                    Ok(()) => {
                        trashed.push((item.path.clone(), *folder));
                        false
                    }
                    Err(err) => {
                        failed = Some(trf("Couldn't trash {}: {}", &[&file_name(&item.path), &err]));
                        true
                    }
                }
            });
        }
        self.unreadable.retain(|_, items| !items.is_empty());
        if let Some(error) = failed {
            self.activity = error;
        }
        if trashed.is_empty() {
            return;
        }
        let paths: Vec<String> = trashed.iter().map(|(path, _)| path.clone()).collect();
        self.db.forget_unreadable(&paths);
        if shell::CAN_RESTORE {
            self.last_trashed = Some((Instant::now(), trashed));
        } else {
            self.activity = trf("Moved {} to trash", &[&trashed.len()]);
        }
    }
}

/// One file or folder with its reason and buttons. Returns the button clicked.
fn row(ui: &mut egui::Ui, item: &Unreadable) -> Option<Action> {
    let plain = shell::plain_path(&item.path);
    let parent = Path::new(&plain).parent().map(|path| path.display().to_string()).unwrap_or_default();
    let detail = if item.folder {
        format!("{}  ·  {}  ·  {}", tr("Folder"), item.reason, parent)
    } else {
        format!("{}  ·  {}", item.reason, parent)
    };
    let mut action = None;
    let size = Vec2::new(ui.available_width(), ROW_HEIGHT);
    let response = ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
        ui.set_min_size(size);
        if !item.folder && ui.button(tr("Move to trash")).clicked() {
            action = Some(Action::Trash(vec![item.path.clone()]));
        }
        if ui.button(tr("Show in folder")).clicked() {
            action = Some(Action::Reveal(item.path.clone()));
        }
        ui.add_space(8.0);
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(5.0);
            let name = RichText::new(file_name(&item.path)).size(13.5).color(theme::text());
            ui.add(egui::Label::new(name).truncate()).on_hover_text(&plain);
            let reason = RichText::new(&detail).size(12.0).color(theme::muted());
            ui.add(egui::Label::new(reason).truncate()).on_hover_text(&detail);
        });
    });
    let rect = response.response.rect;
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::hairline()));
    action
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned())
}
