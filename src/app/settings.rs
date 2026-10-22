//! Settings dialog, opened from the gear in the header or with Ctrl+,.

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use eframe::egui::{self, Margin, RichText, Sense, Vec2};

use crate::i18n::{self, tr, trf, Lang};
use crate::thumbs::ThumbStore;
use crate::types::{format_bytes, FileType};
use crate::worker::Command;

use super::theme::{self, ThemeMode};
use super::{about, help, Modal, PicassoApp};

const NAV_WIDTH: f32 = 150.0;
const PAGE_WIDTH: f32 = 470.0;
const PAGE_HEIGHT: f32 = 380.0;

#[derive(Clone, Copy, Default, PartialEq)]
enum Page {
    #[default]
    General,
    Folders,
    FileTypes,
    Storage,
}

impl Page {
    const ALL: [Page; 4] = [Page::General, Page::Folders, Page::FileTypes, Page::Storage];

    fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Folders => "Folders",
            Page::FileTypes => "File types",
            Page::Storage => "Storage",
        }
    }
}

/// Bytes on disk, measured on a background thread.
#[derive(Clone, Copy)]
struct Usage {
    library: u64,
    thumbs: u64,
    tiles: u64,
}

/// What the dialog keeps between frames.
#[derive(Default)]
pub(super) struct SettingsState {
    page: Page,
    usage: Option<Usage>,
    /// Sizes being measured, or the cache being cleared and then measured.
    pending: Option<Receiver<Usage>>,
    clearing: bool,
    error: String,
    /// File types ticked while the dialog is open, applied when it closes so
    /// a box unticked by mistake removes nothing from the library.
    file_types: Option<Vec<FileType>>,
}

impl PicassoApp {
    pub(super) fn open_settings(&mut self, ctx: &egui::Context) {
        self.settings.error.clear();
        self.settings.file_types = Some(self.config.file_types());        self.measure_storage(ctx, false);
        self.modal = Modal::Settings;
    }

    /// Saves the file types picked in a dialog that just closed and rescans
    /// every folder, which adds files of new types and drops the others.
    pub(super) fn settings_closed(&mut self) {
        let Some(picked) = self.settings.file_types.take() else { return };
        let picked: Vec<FileType> = FileType::ALL.into_iter().filter(|kind| picked.contains(kind)).collect();
        if picked.is_empty() || picked == self.config.file_types() {
            return;
        }
        FileType::set_enabled(&picked);
        self.config.set_file_types(&picked);
        for folder in &self.folders {
            let _ = self.cmd_tx.send(Command::Scan(folder.id));
        }
    }

    /// Measures the library and cache on a thread, first clearing the cache
    /// when `clear` is set.
    fn measure_storage(&mut self, ctx: &egui::Context, clear: bool) {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (data, cache, ctx) = (self.data_dir.clone(), self.cache_dir.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("picasso-storage".into()).spawn(move || {
            if clear {
                ThumbStore::open(&cache).clear();
                let tiles = cache.join("tiles");
                let _ = std::fs::remove_dir_all(&tiles);
                let _ = std::fs::create_dir_all(&tiles);
            }
            let usage = Usage {
                library: dir_size(&data),
                thumbs: ["thumbs.db", "thumbs.db-wal", "thumbs.db-shm"]
                    .iter()
                    .map(|name| file_size(&cache.join(name)))
                    .sum::<u64>()
                    + dir_size(&cache.join("thumbs")),
                tiles: dir_size(&cache.join("tiles")),
            };
            let _ = sender.send(usage);
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.settings.pending = Some(receiver);
            self.settings.clearing = clear;
        }
    }

    fn clear_cache(&mut self, ctx: &egui::Context) {
        self.tiles.clear();
        self.tile_wait.clear();
        self.tile_fail.clear();
        self.measure_storage(ctx, true);
    }

    pub(super) fn settings_modal(&mut self, ctx: &egui::Context) {
        if let Some(usage) = self.settings.pending.as_ref().and_then(|pending| pending.try_recv().ok()) {
            if self.settings.clearing {
                self.activity = tr("Cache cleared").to_string();
            }
            self.settings.usage = Some(usage);
            self.settings.pending = None;
            self.settings.clearing = false;
        }
        let mut close = false;
        let response = egui::Modal::new(egui::Id::unique("settings")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(NAV_WIDTH + PAGE_WIDTH + 52.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            help::title_bar(ui, tr("Settings"), &mut close);
            egui::Frame::new().inner_margin(Margin { left: 12, right: 20, top: 14, bottom: 18 }).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(16.0, 8.0);
                    ui.vertical(|ui| {
                        ui.set_width(NAV_WIDTH);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for page in Page::ALL {
                            if nav_item(ui, tr(page.label()), self.settings.page == page).clicked() {
                                self.settings.page = page;
                            }
                        }
                    });
                    ui.vertical(|ui| {
                        ui.set_width(PAGE_WIDTH);
                        ui.set_min_height(PAGE_HEIGHT);
                        ui.spacing_mut().item_spacing.y = 8.0;
                        match self.settings.page {
                            Page::General => self.general_page(ui),
                            Page::Folders => self.folders_page(ui),
                            Page::FileTypes => self.file_types_page(ui),
                            Page::Storage => self.storage_page(ui),
                        }
                    });
                });
            });
        });
        if close || response.should_close() {
            self.modal = Modal::None;
        }
    }

    fn general_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, tr("Appearance"));
        egui::Grid::new("settings-appearance").num_columns(2).spacing([24.0, 10.0]).show(ui, |ui| {
            ui.label(tr("Language"));
            let system = trf("System language ({})", &[&i18n::system().native_name()]);
            let current = self.language.map_or(system.clone(), |lang| lang.native_name().to_string());
            let mut picked = None;
            egui::ComboBox::from_id_salt("settings-language")
                .selected_text(current)
                .width(220.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.language.is_none(), system).clicked() {
                        picked = Some(None);
                    }
                    for lang in Lang::ALL {
                        if ui.selectable_label(self.language == Some(lang), lang.native_name()).clicked() {
                            picked = Some(Some(lang));
                        }
                    }
                });
            if let Some(language) = picked {
                self.set_language(language);
            }
            ui.end_row();

            ui.label(tr("Color theme"));
            ui.horizontal(|ui| {
                for mode in ThemeMode::ALL {
                    if ui.selectable_label(self.theme_mode == mode, tr(mode.label())).clicked() {
                        self.set_theme_mode(ui.ctx(), mode);
                    }
                }
            });
            ui.end_row();
        });

        heading(ui, tr("Browsing"));
        egui::Grid::new("settings-browsing").num_columns(2).spacing([24.0, 10.0]).show(ui, |ui| {
            ui.label(tr("Thumbnail size"));
            let mut size = self.thumb_size;
            let response = ui.add(egui::Slider::new(&mut size, 96.0..=220.0).suffix(" px").fixed_decimals(0));
            self.set_thumb_size(size, &response);
            ui.end_row();

            ui.label(tr("Slideshow interval"));
            let mut interval = self.slideshow_interval;
            let response = ui.add(egui::Slider::new(&mut interval, 1.0..=15.0).suffix(" s").fixed_decimals(0));
            self.set_slideshow_interval(interval, &response);
            ui.end_row();
        });
        ui.add_space(4.0);
        let mut confirm = self.confirm_trash;
        if ui.checkbox(&mut confirm, tr("Ask before moving photos to the trash")).changed() {
            self.confirm_trash = confirm;
            self.config.set("confirm_trash", confirm);
        }
    }

    fn file_types_page(&mut self, ui: &mut egui::Ui) {
        let picked = self.settings.file_types.get_or_insert_with(|| FileType::ALL.to_vec());
        heading(ui, tr("File types"));
        note(ui, tr("Picasso adds files of the ticked types when it scans your folders."));
        ui.add_space(2.0);

        let mut all = picked.len() == FileType::ALL.len();
        let some = !picked.is_empty() && !all;
        if ui.add(egui::Checkbox::new(&mut all, RichText::new(tr("Select all")).strong()).indeterminate(some)).changed() {
            *picked = if all { FileType::ALL.to_vec() } else { Vec::new() };
        }
        egui::Grid::new("settings-file-types").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
            for kind in FileType::ALL {
                let mut on = picked.contains(&kind);
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    if ui.checkbox(&mut on, tr(kind.label())).changed() {
                        picked.retain(|other| *other != kind);
                        if on {
                            picked.push(kind);
                        }
                    }
                });
                ui.label(RichText::new(extension_list(kind)).size(12.0).color(theme::muted()));
                ui.end_row();
            }
        });

        ui.add_space(4.0);
        if picked.is_empty() {
            ui.label(RichText::new(tr("Select at least one file type.")).color(theme::DANGER));
        } else {
            note(
                ui,
                tr("Turning a type off removes those files from the library, with their stars, captions, and edits, when the dialog closes. The files stay on disk."),
            );
        }
    }

    fn folders_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, tr("Photo folders"));
        note(ui, tr("Picasso watches this folder and adds new photos as they appear."));
        let folders: Vec<(i64, String, i64)> =
            self.folders.iter().map(|folder| (folder.id, folder.path.clone(), folder.count)).collect();
        egui::Frame::new()
            .fill(theme::tray())
            .stroke(egui::Stroke::new(1.0, theme::hairline()))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    if folders.is_empty() {
                        ui.label(RichText::new(tr("No folders yet")).color(theme::muted()));
                    }
                    for (id, path, count) in folders {
                        self.folder_row(ui, id, &path, count);
                    }
                });
            });
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button(tr("+  Import"))).on_hover_text(tr("Add Folder to Picasso…")).clicked() {
                if let Some(picked) = rfd::FileDialog::new().set_title(tr("Add a photo folder")).pick_folder() {
                    match self.add_folder(&picked) {
                        Ok(()) => self.settings.error.clear(),
                        Err(err) => self.settings.error = err,
                    }
                }
            }
            if ui.button(tr("Rescan all folders")).clicked() {
                for folder in &self.folders {
                    let _ = self.cmd_tx.send(Command::Scan(folder.id));
                }
            }
        });
        if !self.settings.error.is_empty() {
            ui.label(RichText::new(&self.settings.error).color(theme::DANGER));
        }
    }

    fn folder_row(&mut self, ui: &mut egui::Ui, id: i64, path: &str, count: i64) {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr("Remove")).on_hover_text(tr("Remove from Picasso…")).clicked() {
                    self.remove_folder(id);
                }
                if ui.button(tr("Refresh")).on_hover_text(tr("Refresh Thumbnails")).clicked() {
                    let _ = self.cmd_tx.send(Command::Scan(id));
                }
                if ui.button(tr("Open folder")).clicked() {
                    let _ = open::that(path);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(display_path(Path::new(path))).color(theme::text())).truncate())
                        .on_hover_text(super::photo_count(count as usize));
                });
            });
        });
    }

    fn storage_page(&mut self, ui: &mut egui::Ui) {
        let usage = self.settings.usage;
        let busy = self.settings.pending.is_some();
        let size = |pick: fn(&Usage) -> u64| usage.as_ref().map_or_else(|| tr("Measuring…").to_string(), |usage| format_bytes(pick(usage)));

        heading(ui, tr("Library database"));
        note(ui, tr("Albums, stars, captions, and edits. Your photo files stay where they are."));
        location(ui, &self.data_dir, &size(|usage| usage.library));

        heading(ui, tr("Cache"));
        note(ui, tr("Thumbnails and map tiles. Safe to clear; Picasso makes them again when needed."));
        location(ui, &self.cache_dir, &size(|usage| usage.thumbs + usage.tiles));
        egui::Grid::new("settings-cache").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new(tr("Thumbnails")).color(theme::muted()));
            ui.label(size(|usage| usage.thumbs));
            ui.end_row();
            ui.label(RichText::new(tr("Map tiles")).color(theme::muted()));
            ui.label(size(|usage| usage.tiles));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            let clear = ui.add_enabled(!busy, theme::danger_button(tr("Clear cache")));
            if clear.clicked() {
                self.clear_cache(ui.ctx());
            }
            if self.settings.clearing {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(tr("Clearing…")).color(theme::muted()));
            }
        });
    }

    pub(super) fn set_thumb_size(&mut self, size: f32, response: &egui::Response) {
        if (size - self.thumb_size).abs() > 0.1 {
            self.thumb_size = size;
        }
        if released(response) {
            self.config.set("thumb_size", self.thumb_size.round());
        }
    }

    pub(super) fn set_slideshow_interval(&mut self, interval: f32, response: &egui::Response) {
        self.slideshow_interval = interval;
        if let Some(show) = self.slideshow.as_mut() {
            show.interval = interval;
        }
        if released(response) {
            self.config.set("slideshow_interval", interval.round());
        }
    }
}

/// True once a slider has its final value: the drag ended, or it was set by
/// a click or the keyboard.
fn released(response: &egui::Response) -> bool {
    response.drag_stopped() || (response.changed() && !response.dragged())
}

fn nav_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(NAV_WIDTH, 32.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if selected {
        ui.painter().rect_filled(rect, theme::RADIUS, theme::select());
    } else if response.hovered() {
        ui.painter().rect_filled(rect, theme::RADIUS, theme::hover());
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(12.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.5),
        if selected { theme::ACCENT } else { theme::text() },
    );
    response
}

/// Section title, with room above it unless it opens the page.
fn heading(ui: &mut egui::Ui, title: &str) {
    let first = ui.min_rect().height() < 1.0;
    ui.add_space(if first { 0.0 } else { 12.0 });
    ui.label(RichText::new(title).size(14.5).strong().color(theme::text()));
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(theme::muted())).wrap());
}

/// Folder path with its size and a button that opens it.
fn location(ui: &mut egui::Ui, path: &Path, size: &str) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tr("Open folder")).clicked() {
                let _ = std::fs::create_dir_all(path);
                let _ = open::that(path);
            }
            ui.label(RichText::new(size).strong().color(theme::text()));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let text = display_path(path);
                ui.add(egui::Label::new(RichText::new(&text).monospace().size(11.5).color(theme::muted())).truncate())
                    .on_hover_text(text);
            });
        });
    });
}

/// `.jpg  .jpeg`, shortened for the long RAW and video lists.
fn extension_list(kind: FileType) -> String {
    const SHOWN: usize = 6;
    let extensions = kind.extensions();
    let mut text = extensions.iter().take(SHOWN).map(|ext| format!(".{ext}")).collect::<Vec<_>>().join("  ");
    if extensions.len() > SHOWN {
        text.push_str(&format!("  +{}", extensions.len() - SHOWN));
    }
    text
}

fn display_path(path: &Path) -> String {
    path.display().to_string().trim_start_matches(r"\\?\").to_string()
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => stack.push(entry.path()),
                Ok(meta) => total += meta.len(),
                Err(_) => {}
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_folders_recursively() {
        let dir = std::env::temp_dir().join(format!("picasso-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("one"), [0; 10]).unwrap();
        std::fs::write(dir.join("a/b/two"), [0; 32]).unwrap();
        assert_eq!(dir_size(&dir), 42);
        assert_eq!(dir_size(&dir.join("missing")), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
