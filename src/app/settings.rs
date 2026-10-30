//! Settings dialog, opened from the gear in the header or with Ctrl+,.

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use eframe::egui::{self, Margin, RichText, Sense, Vec2};

use crate::backup;
use crate::config::Config;
use crate::geo::MapSource;
use crate::i18n::{self, tr, trf, Lang};
use crate::secret;
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
    Map,
    Tracks,
    Publishing,
    Storage,
}

impl Page {
    const ALL: [Page; 7] = [Page::General, Page::Folders, Page::FileTypes, Page::Map, Page::Tracks, Page::Publishing, Page::Storage];

    fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Folders => "Folders",
            Page::FileTypes => "File types",
            Page::Map => "Map",
            Page::Tracks => "GPS tracks",
            Page::Publishing => "Publishing",
            Page::Storage => "Storage",
        }
    }
}

const CARTO_KEY_URL: &str = "https://carto.com/basemaps/apikey/";
/// Shown in the key field in place of a saved key.
const KEY_MASK: &str = "********";

/// The CARTO key saved in `config`, or an empty one when there is none or it
/// was sealed on another machine. A key an older build saved in the clear is
/// sealed now.
pub(super) fn load_carto_key(config: &mut Config) -> String {
    let stored = config.text("carto_key").unwrap_or_default().to_string();
    if secret::is_sealed(&stored) {
        return secret::open(&stored).unwrap_or_default();
    }
    if let Some(sealed) = secret::seal(&stored).filter(|_| !stored.is_empty()) {
        config.set("carto_key", sealed);
    }
    stored
}

/// What the key field starts with: the mask when a key is saved.
fn masked(key: &str) -> String {
    if key.is_empty() { String::new() } else { KEY_MASK.to_string() }
}

fn source_note(source: MapSource) -> &'static str {
    match source {
        MapSource::OpenFreeMap => "Free, with no key. Picasso draws it from OpenStreetMap data.",
        MapSource::OpenStreetMap => "The standard OpenStreetMap map.",
        MapSource::CartoVoyager | MapSource::CartoPositron | MapSource::CartoDarkMatter => {
            "Clean maps in three styles. Needs a free CARTO API key."
        }
        MapSource::EsriImagery => "Satellite and aerial photos.",
    }
}

fn style_note(style: MapSource) -> &'static str {
    match style {
        MapSource::CartoVoyager => "Colorful and detailed.",
        MapSource::CartoDarkMatter => "Dark, suited to the dark theme.",
        _ => "Light and quiet.",
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
    /// CARTO key being typed, saved when the field or the dialog closes.
    /// It holds [`KEY_MASK`] rather than a saved key, and is `None` while
    /// the dialog is closed, so the saved key is left alone.
    carto_key: Option<String>,
    /// A backup being made on a thread.
    backing_up: Option<Receiver<Result<PathBuf, String>>>,
    /// The outcome of the latest Back up now or Restore.
    backup_note: String,
    /// A backup picked for Restore, waiting for the user to confirm.
    restore: Option<RestoreAsk>,
    /// Track cleanup settings on the GPS tracks page, read again each time
    /// the dialog opens.
    tracks: Option<crate::track_edit::Settings>,
    /// The Publishing page's sites being connected or checked.
    pub(super) publishing: super::publishing::PublishingState,
}

struct RestoreAsk {
    path: PathBuf,
    photos: i64,
}

impl PicassoApp {
    pub(super) fn open_settings(&mut self, ctx: &egui::Context) {
        self.settings.error.clear();
        self.settings.file_types = Some(self.config.file_types());
        self.settings.carto_key = Some(masked(&self.carto_key));
        self.measure_storage(ctx, false);
        self.modal = Modal::Settings;
    }

    /// Saves the file types picked in a dialog that just closed and rescans
    /// every folder, which adds files of new types and drops the others.
    pub(super) fn settings_closed(&mut self) {
        self.save_carto_key();
        self.settings.carto_key = None;
        if let Some(tracks) = self.settings.tracks.take() {
            tracks.save(&mut self.config);
        }
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
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(cache.join(format!("{}{suffix}", super::dupes::HASHES_FILE)));
                }
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
        self.reset_tiles();
        self.measure_storage(ctx, true);
    }

    fn set_map_source(&mut self, source: MapSource) {
        if source != self.map_source {
            self.map_source = source;
            self.config.set("map_source", source.code());
            if source.needs_key() {
                self.config.set("carto_style", source.code());
            }
            self.reset_tiles();
        }
    }

    /// The CARTO style picked last, for when CARTO is chosen again.
    fn carto_style(&self) -> MapSource {
        self.config
            .text("carto_style")
            .and_then(MapSource::from_code)
            .filter(|style| style.needs_key())
            .unwrap_or(MapSource::CartoPositron)
    }

    /// Saves a newly typed key. An empty field keeps the saved key, which
    /// only the Remove button clears.
    fn save_carto_key(&mut self) {
        let Some(typed) = &self.settings.carto_key else { return };
        let key = typed.trim().to_string();
        if key.is_empty() || key == KEY_MASK {
            return;
        }
        self.settings.carto_key = Some(KEY_MASK.to_string());
        self.store_carto_key(key);
    }

    fn store_carto_key(&mut self, key: String) {
        if key == self.carto_key {
            return;
        }
        let sealed = if key.is_empty() { Some(String::new()) } else { secret::seal(&key) };
        if let Some(sealed) = sealed {
            self.config.set("carto_key", sealed);
        }
        self.carto_key = key;
        if self.map_source.needs_key() {
            self.reset_tiles();
        }
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
                            Page::Map => self.map_page(ui),
                            Page::Tracks => self.tracks_page(ui),
                            Page::Publishing => self.publishing_page(ui),
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
            let response = ui.add(egui::Slider::new(&mut size, super::MIN_THUMB..=super::MAX_THUMB).suffix(" px").fixed_decimals(0));
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

        heading(ui, tr("Updates"));
        self.update_settings(ui);
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

    fn map_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, tr("Map source"));
        note(ui, tr("Places and the Map panel show this map."));
        ui.add_space(2.0);
        let carto = MapSource::CARTO_STYLES[0];
        for source in MapSource::ALL.into_iter().filter(|source| !source.needs_key() || *source == carto) {
            let chosen = if source.needs_key() { self.map_source.needs_key() } else { self.map_source == source };
            let name = if source.needs_key() { "CARTO" } else { source.name() };
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if ui.radio(chosen, RichText::new(name).strong()).clicked() && !chosen {
                    let picked = if source.needs_key() { self.carto_style() } else { source };
                    self.set_map_source(picked);
                }
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    note(ui, tr(source_note(source)));
                });
            });
            if source.needs_key() && chosen {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.label(tr("Style"));
                    for style in MapSource::CARTO_STYLES {
                        let button = ui.selectable_label(self.map_source == style, style.style_name());
                        if button.on_hover_text(tr(style_note(style))).clicked() {
                            self.set_map_source(style);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.label(tr("CARTO API key"));
                    let typed = self.settings.carto_key.get_or_insert_with(|| masked(&self.carto_key));
                    let field = ui.add(
                        egui::TextEdit::singleline(typed)
                            .hint_text(tr("Paste your key here"))
                            .desired_width(200.0),
                    );
                    if field.gained_focus() && typed == KEY_MASK {
                        typed.clear();
                    }
                    if field.lost_focus() {
                        self.save_carto_key();
                        if self.settings.carto_key.as_deref().is_some_and(|typed| typed.trim().is_empty()) {
                            self.settings.carto_key = Some(masked(&self.carto_key));
                        }
                    }
                    if !self.carto_key.is_empty() && ui.button(tr("Remove")).clicked() {
                        self.store_carto_key(String::new());
                        self.settings.carto_key = Some(String::new());
                    }
                    ui.hyperlink_to(tr("Get a free key"), CARTO_KEY_URL);
                });
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    note(ui, tr("The key is saved encrypted and works only on this computer."));
                });
                if self.carto_key.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label(RichText::new(tr("The map stays blank until you add a key.")).size(12.5).color(theme::DANGER));
                    });
                }
            }
            ui.add_space(4.0);
        }
        note(ui, self.map_source.attribution());

        heading(ui, tr("Place names"));
        note(ui, tr("Edit place can look up a name for a Place's position. Picasso sends only that position, rounded to about 100 m, to OpenStreetMap Nominatim, and only when you press Look up name. Answers are kept on this computer."));
        let mut allowed = self.config.flag("place_lookup").unwrap_or(false);
        if ui.checkbox(&mut allowed, tr("Look up place names online")).changed() {
            self.config.set("place_lookup", allowed);
        }
    }

    fn tracks_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, tr("Track cleanup"));
        note(ui, tr("The track editor cleans each GPX, KML, and Polarsteps track up with these settings when it opens one. Tracks saved by Picasso or GPXViewer open as saved."));
        ui.add_space(2.0);
        let settings = self.settings.tracks.get_or_insert_with(|| crate::track_edit::Settings::load(&self.config));
        super::track_editor::settings_form(ui, "settings-tracks", settings);
        ui.add_space(4.0);
        if ui.button(tr("Defaults")).clicked() {
            *settings = crate::track_edit::Settings::default();
        }
        // Saved once a drag on a number ends rather than on every step.
        if !ui.input(|input| input.pointer.any_down()) && *settings != crate::track_edit::Settings::load(&self.config) {
            settings.save(&mut self.config);
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

        heading(ui, tr("Excluded folders"));
        note(ui, tr("Picasso skips these folders and everything inside them. Their photos leave the library; the files stay on disk."));
        let excluded = self.excluded.clone();
        if !excluded.is_empty() {
            egui::Frame::new()
                .fill(theme::tray())
                .stroke(egui::Stroke::new(1.0, theme::hairline()))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().id_salt("settings-excluded").max_height(110.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        for (dir, _) in excluded {
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button(tr("Include again")).clicked() {
                                        self.include_dir(&dir);
                                    }
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                        ui.add(egui::Label::new(display_path(Path::new(&dir))).truncate());
                                    });
                                });
                            });
                        }
                    });
                });
        }
        let start = self.folders.first().map(|folder| PathBuf::from(display_path(Path::new(&folder.path))));
        if ui.add_enabled(start.is_some(), egui::Button::new(tr("Exclude a folder…"))).clicked() {
            let mut dialog = rfd::FileDialog::new().set_title(tr("Pick a folder for Picasso to skip"));
            if let Some(start) = start {
                dialog = dialog.set_directory(start);
            }
            if let Some(picked) = dialog.pick_folder() {
                match self.exclude_dir(&picked) {
                    Ok(()) => self.settings.error.clear(),
                    Err(err) => self.settings.error = err,
                }
            }
        }
        if !self.settings.error.is_empty() {
            ui.label(RichText::new(&self.settings.error).color(theme::DANGER));
        }
    }

    fn folder_row(&mut self, ui: &mut egui::Ui, id: i64, path: &str, count: i64) {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr("Remove…")).on_hover_text(tr("Remove from Picasso…")).clicked() {
                    self.remove_asked = Some(id);
                }
                if ui.button(tr("Refresh")).on_hover_text(tr("Rescan this folder")).clicked() {
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

        self.backups_section(ui);

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

        heading(ui, tr("Settings"));
        note(ui, tr("The choices in this dialog, kept apart from the library so they apply to whichever library you open."));
        settings_file(ui, self.config.path());
    }

    fn backups_section(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.settings.backing_up.as_ref().and_then(|done| done.try_recv().ok()) {
            self.settings.backing_up = None;
            self.settings.backup_note = match result {
                Ok(_) => tr("Backup saved").to_string(),
                Err(err) => trf("Couldn't back up the library: {}", &[&err]),
            };
        }
        heading(ui, tr("Backups"));
        note(
            ui,
            &trf(
                "Picasso copies the library once a week and keeps the last {}. Restoring one brings back the albums, stars, captions, and edits it holds.",
                &[&backup::KEEP],
            ),
        );
        let backups = backup::list(&self.data_dir);
        if let Some(ask) = &self.settings.restore {
            let name = ask.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let question = trf(
                "Restore the library from {}? It holds {} photos. Picasso restarts, and backs up the current library first.",
                &[&name, &ask.photos],
            );
            ui.add(egui::Label::new(RichText::new(question).color(theme::text())).wrap());
            let mut confirmed = false;
            let mut cancel = false;
            ui.horizontal(|ui| {
                confirmed = ui.add(theme::danger_button(tr("Restore and restart"))).clicked();
                cancel = ui.button(tr("Cancel")).clicked();
            });
            if cancel {
                self.settings.restore = None;
            } else if confirmed {
                let path = ask.path.clone();
                self.settings.restore = None;
                self.restore_backup(ui.ctx(), &path);
            }
            return;
        }
        let busy = self.settings.backing_up.is_some();
        ui.horizontal(|ui| {
            let last = backups.first().map_or_else(
                || tr("No backup yet").to_string(),
                |newest| trf("Last backup: {} · {}", &[&local_time(newest.made_at), &format_bytes(newest.bytes)]),
            );
            ui.label(RichText::new(last).color(theme::muted()));
            if busy {
                ui.add(egui::Spinner::new().size(14.0));
            } else if !self.settings.backup_note.is_empty() {
                ui.label(RichText::new(&self.settings.backup_note).color(theme::muted()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr("Open folder")).clicked() {
                    let folder = backup::dir(&self.data_dir);
                    let _ = std::fs::create_dir_all(&folder);
                    let _ = open::that(folder);
                }
                if ui.button(tr("Restore…")).on_hover_text(tr("Choose a backup to bring back")).clicked() {
                    self.pick_backup();
                }
                if ui.add_enabled(!busy, egui::Button::new(tr("Back up now"))).clicked() {
                    self.back_up_now(ui.ctx());
                }
            });
        });
    }

    fn back_up_now(&mut self, ctx: &egui::Context) {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (data_dir, ctx) = (self.data_dir.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("picasso-backup".into()).spawn(move || {
            let _ = sender.send(backup::create(&data_dir));
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.settings.backup_note.clear();
            self.settings.backing_up = Some(receiver);
        }
    }

    fn pick_backup(&mut self) {
        let folder = backup::dir(&self.data_dir);
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("Choose a backup to bring back"))
            .set_directory(&folder)
            .add_filter(tr("Picasso library"), &["db"])
            .pick_file()
        else {
            return;
        };
        match backup::check(&path) {
            Ok(photos) => self.settings.restore = Some(RestoreAsk { path, photos }),
            Err(err) => self.settings.backup_note = trf("Couldn't use this backup: {}", &[&err]),
        }
    }

    /// Stages `path` and restarts Picasso, which swaps it in at startup.
    fn restore_backup(&mut self, ctx: &egui::Context, path: &Path) {
        if let Err(err) = backup::stage_restore(&self.data_dir, path) {
            self.settings.backup_note = trf("Couldn't use this backup: {}", &[&err]);
            return;
        }
        match backup::relaunch() {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(_) => self.settings.backup_note = tr("Restart Picasso to finish restoring the backup").to_string(),
        }
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
        if selected { theme::accent() } else { theme::text() },
    );
    response
}

/// Section title, with room above it unless it opens the page.
pub(super) fn heading(ui: &mut egui::Ui, title: &str) {
    let first = ui.min_rect().height() < 1.0;
    ui.add_space(if first { 0.0 } else { 12.0 });
    ui.label(RichText::new(title).size(14.5).strong().color(theme::text()));
}

pub(super) fn note(ui: &mut egui::Ui, text: &str) {
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

/// The settings file's path with its size and a button that shows it in
/// the file manager.
fn settings_file(ui: &mut egui::Ui, path: &Path) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tr("Show in folder")).clicked() {
                crate::shell::reveal(&path.display().to_string());
            }
            ui.label(RichText::new(format_bytes(file_size(path))).strong().color(theme::text()));
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

/// Unix seconds as local date and time, e.g. `2026-10-23 09:58`.
pub(super) fn local_time(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|time| time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

pub(super) fn display_path(path: &Path) -> String {
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
