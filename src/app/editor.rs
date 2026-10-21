//! Edit panel of the viewer. Each slider release, toggle, or reset becomes a
//! step in the photo's history, saved in the library; the original file is
//! never written. Steps can be undone and redone one at a time or all at
//! once, and the result is exported to a new file.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Sense, Vec2};
use image::RgbaImage;

use crate::edit::{self, Control, EditJob, History, Settings, Step, GROUPS};
use crate::i18n::{tr, trf};
use crate::types::Photo;
use crate::worker::UiEvent;

use super::theme::{self, ACCENT, VIEWER_MUTED, VIEWER_PANEL, WHITE};
use super::PicassoApp;

pub(super) struct EditSession {
    pub id: i64,
    history: History,
    /// Slider value while it is dragged; it becomes a step on release.
    live: Option<(Control, f32)>,
    base: Option<Arc<RgbaImage>>,
    error: Option<String>,
    preview: Option<egui::TextureHandle>,
    original: Option<egui::TextureHandle>,
    requested: Option<Settings>,
    seq: u64,
    shown_seq: u64,
    /// Show the original instead of the edit.
    pub before: bool,
    /// Steps changed during this session, so the full-size view is stale.
    changed: bool,
}

impl EditSession {
    fn preview_settings(&self) -> Settings {
        let mut settings = self.history.settings();
        if let Some((control, value)) = self.live {
            settings.set(control, value);
        }
        settings
    }

    /// Texture for the stage, when the editor has one ready.
    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        if self.before {
            self.original.as_ref()
        } else {
            self.preview.as_ref()
        }
    }
}

enum Action {
    Push(Step),
    Undo,
    Redo,
    UndoAll,
    RedoAll,
    Jump(usize),
    Export,
    Done,
}

impl PicassoApp {
    /// Settings thumbnails and full-size views of `id` are rendered with.
    pub(super) fn edit_settings(&self, id: i64) -> Settings {
        self.edits.get(&id).copied().unwrap_or_default()
    }

    pub(super) fn toggle_editing(&mut self) {
        self.editing = !self.editing;
        if !self.editing {
            self.end_edit_session();
        }
    }

    /// Opens a session for `photo` and keeps its preview rendered with the
    /// current steps. `None` closes the session.
    pub(super) fn sync_edit(&mut self, photo: Option<&Photo>) {
        let Some(photo) = photo.filter(|_| self.editing) else {
            self.end_edit_session();
            return;
        };
        if self.edit.as_ref().map(|session| session.id) != Some(photo.id) {
            self.end_edit_session();
            let history = self
                .db
                .edit_history(photo.id)
                .map(|(steps, applied)| History::from_json(&steps, applied))
                .unwrap_or_default();
            let _ = self.edit_tx.send(EditJob::Load {
                id: photo.id,
                path: photo.path.clone(),
            });
            self.edit = Some(EditSession {
                id: photo.id,
                history,
                live: None,
                base: None,
                error: None,
                preview: None,
                original: None,
                requested: None,
                seq: 0,
                shown_seq: 0,
                before: false,
                changed: false,
            });
        }
        let Some(session) = self.edit.as_mut() else { return };
        let wanted = session.preview_settings();
        if let Some(base) = session.base.clone() {
            if session.requested != Some(wanted) {
                session.seq += 1;
                session.requested = Some(wanted);
                let _ = self.edit_tx.send(EditJob::Render {
                    id: session.id,
                    seq: session.seq,
                    base,
                    settings: wanted,
                });
            }
        }
    }

    pub(super) fn end_edit_session(&mut self) {
        let Some(session) = self.edit.take() else { return };
        if session.changed {
            self.forget_display(session.id);
        }
    }

    /// Editor results from the background: the decoded base, a rendered
    /// preview, or a finished export.
    pub(super) fn on_edit_event(&mut self, event: UiEvent, ctx: &egui::Context) {
        match event {
            UiEvent::EditBase { id, image } => {
                let Some(session) = self.edit.as_mut().filter(|session| session.id == id) else { return };
                session.original = Some(ctx.load_texture(format!("edit-original-{id}"), color_image(&image), egui::TextureOptions::LINEAR));
                session.base = Some(image);
                session.requested = None;
            }
            UiEvent::EditFailed { id, error } => {
                if let Some(session) = self.edit.as_mut().filter(|session| session.id == id) {
                    session.error = Some(error);
                }
            }
            UiEvent::EditPreview { id, seq, w, h, rgba } => {
                let Some(session) = self.edit.as_mut().filter(|session| session.id == id) else { return };
                if seq <= session.shown_seq || rgba.len() < w * h * 4 {
                    return;
                }
                session.shown_seq = seq;
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba[..w * h * 4]);
                match session.preview.as_mut() {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => session.preview = Some(ctx.load_texture(format!("edit-{id}"), image, egui::TextureOptions::LINEAR)),
                }
            }
            UiEvent::Exported(result) => {
                self.activity = match result {
                    Ok(path) => trf("Exported {}", &[&crate::types::folder_label(&path.to_string_lossy())]),
                    Err(err) => trf("Couldn't export: {}", &[&err]),
                };
            }
            _ => {}
        }
    }

    pub(super) fn edit_undo(&mut self) {
        self.edit_history(History::undo);
    }

    pub(super) fn edit_redo(&mut self) {
        self.edit_history(History::redo);
    }

    pub(super) fn toggle_before(&mut self) {
        if let Some(session) = self.edit.as_mut() {
            session.before = !session.before;
        }
    }

    fn edit_history(&mut self, change: impl FnOnce(&mut History)) {
        let Some(session) = self.edit.as_mut() else { return };
        let before = session.history.clone();
        session.live = None;
        change(&mut session.history);
        if session.history != before {
            self.edit_changed();
        }
    }

    /// Saves the steps and refreshes everything drawn from them.
    fn edit_changed(&mut self) {
        let Some(session) = self.edit.as_mut() else { return };
        session.changed = true;
        let id = session.id;
        let history = &session.history;
        self.db
            .save_edit_history(id, &history.steps_json(), history.steps().len(), history.applied());
        let settings = history.settings();
        if settings.is_identity() {
            self.edits.remove(&id);
        } else {
            self.edits.insert(id, settings);
        }
        self.forget_thumb(id);
    }

    fn export_edit(&mut self, ctx: &egui::Context) {
        let Some(session) = self.edit.as_ref() else { return };
        let Some(photo) = self.db.photo(session.id) else { return };
        let settings = session.history.settings();
        let source = PathBuf::from(&photo.path);
        let folder = PathBuf::from(photo.path.trim_start_matches(r"\\?\"));
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr("Export edited photo"))
            .set_file_name(edit::export_name(&source))
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("PNG", &["png"])
            .add_filter("TIFF", &["tif", "tiff"]);
        if let Some(dir) = folder.parent() {
            dialog = dialog.set_directory(dir);
        }
        let Some(target) = dialog.save_file() else { return };
        self.activity = trf("Exporting {}…", &[&crate::types::folder_label(&target.to_string_lossy())]);
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let rotation = photo.rotation;
        let _ = std::thread::Builder::new().name("picasso-export".into()).spawn(move || {
            let result = edit::export(&source, &target, &settings, rotation).map(|()| target);
            let _ = events.send(UiEvent::Exported(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn edit_panel(&mut self, ui: &mut egui::Ui, height: f32, photo: Option<&Photo>) {
        let mut actions = Vec::new();
        let frame = egui::Frame::new().fill(VIEWER_PANEL).inner_margin(egui::Margin::same(12));
        frame.show(ui, |ui| {
            ui.set_min_size(Vec2::new(276.0, height - 24.0));
            ui.set_max_width(276.0);
            // The viewer row lays out left to right; the panel stacks downward.
            let layout = egui::Layout::top_down(egui::Align::Min);
            ui.with_layout(layout, |ui| self.edit_panel_contents(ui, height, photo, &mut actions));
        });
        for action in actions {
            match action {
                Action::Push(step) => {
                    if let Some(session) = self.edit.as_mut() {
                        session.live = None;
                        if session.history.push(step) {
                            self.edit_changed();
                        }
                    }
                }
                Action::Undo => self.edit_history(History::undo),
                Action::Redo => self.edit_history(History::redo),
                Action::UndoAll => self.edit_history(History::undo_all),
                Action::RedoAll => self.edit_history(History::redo_all),
                Action::Jump(count) => self.edit_history(|history| history.jump(count)),
                Action::Export => self.export_edit(ui.ctx()),
                Action::Done => self.toggle_editing(),
            }
        }
    }

    fn edit_panel_contents(&mut self, ui: &mut egui::Ui, height: f32, photo: Option<&Photo>, actions: &mut Vec<Action>) {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tr("Edit")).size(16.0).strong().color(WHITE));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("Done")).clicked() {
                        actions.push(Action::Done);
                    }
                });
            });
            ui.add_space(6.0);
            if photo.is_some_and(Photo::is_video) {
                ui.label(egui::RichText::new(tr("Videos can't be edited")).color(VIEWER_MUTED));
                return;
            }
            let Some(session) = self.edit.as_mut() else {
                ui.label(egui::RichText::new(tr("Loading…")).color(VIEWER_MUTED));
                return;
            };
            if let Some(error) = &session.error {
                ui.label(egui::RichText::new(error).color(theme::DANGER));
                return;
            }
            history_buttons(ui, &session.history, &mut actions);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.toggle_value(&mut session.before, tr("Before")).on_hover_text(tr("Show the original (\\)"));
                let reset = ui.add_enabled(!session.history.settings().is_identity(), egui::Button::new(tr("Reset all")));
                if reset.clicked() {
                    actions.push(Action::Push(Step::Reset));
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("edit-panel")
                .auto_shrink([false, false])
                .max_height((height - 180.0).max(120.0))
                .show(ui, |ui| {
                    for (title, controls) in GROUPS {
                        section(ui, tr(title));
                        for control in controls {
                            slider_row(ui, session, *control, &mut actions);
                        }
                        if title == "Color" {
                            let mut mono = session.history.settings().get(Control::Monochrome) >= 0.5;
                            if ui.checkbox(&mut mono, tr(Control::Monochrome.label())).changed() {
                                actions.push(Action::Push(Step::Set(Control::Monochrome, if mono { 1.0 } else { 0.0 })));
                            }
                        }
                    }
                    section(ui, tr("History"));
                    history_list(ui, &session.history, &mut actions);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(tr("Edits are saved as steps in the library. The original file is never changed."))
                            .size(11.0)
                            .color(VIEWER_MUTED),
                    );
                });
            ui.add_space(8.0);
            let export = ui
                .add_sized([ui.available_width(), 30.0], theme::primary_button(tr("Export…")))
                .on_hover_text(tr("Save the edited photo as a new file"));
            if export.clicked() {
                actions.push(Action::Export);
            }
        });
        for action in actions {
            match action {
                Action::Push(step) => {
                    if let Some(session) = self.edit.as_mut() {
                        session.live = None;
                        if session.history.push(step) {
                            self.edit_changed();
                        }
                    }
                }
                Action::Undo => self.edit_history(History::undo),
                Action::Redo => self.edit_history(History::redo),
                Action::UndoAll => self.edit_history(History::undo_all),
                Action::RedoAll => self.edit_history(History::redo_all),
                Action::Jump(count) => self.edit_history(|history| history.jump(count)),
                Action::Export => self.export_edit(ui.ctx()),
                Action::Done => self.toggle_editing(),
            }
        }
    }

    pub(super) fn forget_thumb(&mut self, id: i64) {
        self.thumbs.map.remove(&id);
        self.thumbs.order.retain(|item| *item != id);
        self.thumb_failed.remove(&id);
    }

    pub(super) fn forget_display(&mut self, id: i64) {
        self.displays.retain(|item| item.id != id);
        self.display_inflight.remove(&id);
        self.display_failed.remove(&id);
    }
}

fn history_buttons(ui: &mut egui::Ui, history: &History, actions: &mut Vec<Action>) {
    ui.columns(4, |columns| {
        let buttons = [
            (history.can_undo(), "Undo all", "Back to the original", Action::UndoAll),
            (history.can_undo(), "Undo", "Undo (Ctrl+Z)", Action::Undo),
            (history.can_redo(), "Redo", "Redo (Ctrl+Y)", Action::Redo),
            (history.can_redo(), "Redo all", "Apply every step again", Action::RedoAll),
        ];
        for (column, (enabled, label, hint, action)) in columns.iter_mut().zip(buttons) {
            let button = egui::Button::new(egui::RichText::new(tr(label)).size(12.0));
            let response = column.add_enabled(enabled, button.min_size(Vec2::new(column.available_width(), 26.0)));
            if response.on_hover_text(tr(hint)).clicked() {
                actions.push(action);
            }
        }
    });
}

fn slider_row(ui: &mut egui::Ui, session: &mut EditSession, control: Control, actions: &mut Vec<Action>) {
    let committed = session.history.settings().get(control);
    let mut value = session
        .live
        .filter(|(live, _)| *live == control)
        .map_or(committed, |(_, value)| value);
    ui.horizontal(|ui| {
        let name = egui::Label::new(egui::RichText::new(tr(control.label())).size(12.5).color(WHITE)).sense(Sense::click());
        if ui.add(name).on_hover_text(tr("Double-click to reset")).double_clicked() {
            actions.push(Action::Push(Step::Set(control, 0.0)));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let color = if value != 0.0 { ACCENT } else { VIEWER_MUTED };
            ui.label(egui::RichText::new(control.format(value)).size(12.0).color(color));
        });
    });
    let (min, max) = control.range();
    ui.spacing_mut().slider_width = ui.available_width();
    let step = if control == Control::Exposure { 0.01 } else { 1.0 };
    let response = ui.add(egui::Slider::new(&mut value, min..=max).show_value(false).step_by(step));
    if response.changed() {
        session.live = Some((control, value));
    }
    if !response.dragged() {
        if let Some((live, value)) = session.live.filter(|(live, _)| *live == control) {
            session.live = None;
            actions.push(Action::Push(Step::Set(live, value)));
        }
    }
    ui.add_space(2.0);
}

fn history_list(ui: &mut egui::Ui, history: &History, actions: &mut Vec<Action>) {
    let applied = history.applied();
    let rows = std::iter::once(tr("Original").to_string()).chain(history.steps().iter().map(Step::label));
    for (index, text) in rows.enumerate() {
        let color = if index > applied { VIEWER_MUTED.gamma_multiply(0.7) } else { WHITE };
        let label = egui::RichText::new(text).size(12.0).color(color);
        if ui.selectable_label(index == applied, label).clicked() {
            actions.push(Action::Jump(index));
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title.to_uppercase()).size(11.0).strong().color(VIEWER_MUTED));
    ui.add_space(2.0);
}

fn color_image(image: &RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw())
}
