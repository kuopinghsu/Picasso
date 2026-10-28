//! The Story editor: a Trip's narrative as an ordered list of blocks, with
//! undo and redo for the session and autosave to the library.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, Margin, Modifiers, Pos2, Rect, RichText, Sense, Vec2};

use crate::i18n::{tr, trf};
use crate::trip::{grid_layout_for, Block, DayFilter, MediaUse, MemberQuery, Story, StoryBlock, GRID_LAYOUTS};
use crate::story_html;
use crate::types::{Nav, Photo};

use super::{about, theme, Modal, PicassoApp, UiEvent};

/// A pause in typing this long saves.
const SAVE_PAUSE: Duration = Duration::from_secs(1);
/// Changes are saved at the latest this long after the first unsaved one.
const SAVE_WITHIN: Duration = Duration::from_secs(5);
const PHOTO_HEIGHT: f32 = 180.0;
const GALLERY_TILE: f32 = 96.0;
/// Photos in the strip to pick from.
const PICK_TILE: f32 = 72.0;
/// Most photos the strip shows at once; a Day narrows it.
const PICK_LIMIT: usize = 2000;

#[derive(Clone, PartialEq)]
struct Snapshot {
    story: Story,
    blocks: Vec<StoryBlock>,
}

pub(super) struct StoryEditor {
    story: Story,
    blocks: Vec<StoryBlock>,
    /// The cover as last saved, so a save changes it only when it changed.
    saved_cover: Option<i64>,
    /// When the oldest unsaved change was made, and the latest.
    unsaved: Option<(Instant, Instant)>,
    error: Option<String>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The text field being typed in, so a run of keystrokes is one undo step.
    typing: Option<(String, u8)>,
    /// Catalog photo of each media reference; `None` while unavailable.
    photos: HashMap<i64, Option<Photo>>,
    /// The preview or export being made, to show how far it is and stop it.
    job: Option<Arc<story_html::Progress>>,
    /// The block to scroll to on the next frame.
    reveal: Option<usize>,
    /// The gallery or grid photos are being picked for, from the Trip.
    picking: Option<Picker>,
}

/// Picking the Trip's photos into one gallery or grid.
struct Picker {
    /// The block's id, which stays while blocks move.
    uid: String,
    day: DayFilter,
    /// The Trip's photos on show, as media references.
    choices: Vec<i64>,
}

impl StoryEditor {
    fn snapshot(&self) -> Snapshot {
        Snapshot { story: self.story.clone(), blocks: self.blocks.clone() }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        // The revision belongs to the library, not to the edit being undone.
        let revision = self.story.revision;
        self.story = snapshot.story;
        self.story.revision = revision;
        self.blocks = snapshot.blocks;
        self.typing = None;
        self.touch();
    }

    fn touch(&mut self) {
        let now = Instant::now();
        self.unsaved = Some((self.unsaved.map_or(now, |(first, _)| first), now));
        self.error = None;
    }

    fn due(&self) -> bool {
        self.error.is_none()
            && self.unsaved.is_some_and(|(first, last)| last.elapsed() >= SAVE_PAUSE || first.elapsed() >= SAVE_WITHIN)
    }
}

/// What a page of a Story is made from.
struct PageJob {
    story: Story,
    blocks: Vec<StoryBlock>,
    sources: HashMap<i64, story_html::Source>,
    dates: Option<String>,
    words: story_html::Words,
}

impl PageJob {
    fn page(&self) -> story_html::Page<'_> {
        story_html::Page { story: &self.story, dates: self.dates.as_deref(), blocks: &self.blocks, sources: &self.sources, words: &self.words }
    }
}

/// The choices of the Export dialog.
pub(super) struct ExportDialog {
    format: story_html::Format,
    profile: story_html::Profile,
    folder: String,
    error: String,
    /// What the check before export found, when the dialog opened.
    check: story_html::Preflight,
    /// The user chose to export unavailable photos as unavailable.
    skip_missing: bool,
}

/// How photos added to a Story together are placed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    /// A photo block each.
    Separate,
    Gallery,
    Grid,
}

/// What a block's controls asked for, applied after the blocks are drawn.
enum Action {
    Up(usize),
    Down(usize),
    Duplicate(usize),
    Remove(usize),
    Insert(usize, Block),
    Cover(Option<i64>),
    /// Takes photo `n` out of the gallery or grid at `i`.
    RemoveFromGallery(usize, usize),
    /// Lays out the photo, gallery, or grid at `i` as `Shape`.
    Arrange(usize, Shape),
    /// Adds a photo to the gallery or grid at `i`, or takes it out.
    Toggle(usize, i64),
    /// Puts a dragged photo at place `n` of the gallery or grid at `i`.
    Drop(Drag, usize, usize),
}

/// How a block's photos are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Photo,
    Gallery,
    Grid(u8, u8),
}

/// A photo being dragged: from place `n` of the gallery or grid at `i`,
/// or from the strip of the Trip's photos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Placed(usize, usize),
    New(i64),
}

/// Lets a photo be dragged, dimmed where it was while it moves.
fn drag_source(ui: &egui::Ui, response: &egui::Response, drag: Drag) {
    if response.drag_started() || response.dragged() {
        response.dnd_set_drag_payload(drag);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.painter().rect_filled(response.rect, 4, theme::sidebar().gamma_multiply(0.7));
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
}

/// Takes a photo dropped on place `n` of the gallery or grid at `i`: before
/// it on its left half, after it on its right. A bar shows where while one
/// is held over it.
fn drop_target(ui: &egui::Ui, response: &egui::Response, i: usize, n: usize, actions: &mut Vec<Action>) {
    let rect = response.rect;
    let after = response.hover_pos().is_some_and(|pointer| pointer.x > rect.center().x);
    if response.dnd_hover_payload::<Drag>().is_some() {
        let x = if after { rect.right() } else { rect.left() - 3.0 };
        let bar = egui::Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(3.0, rect.height()));
        ui.painter().rect_filled(bar, 1, theme::ACCENT);
    }
    if let Some(drag) = response.dnd_release_payload::<Drag>() {
        actions.push(Action::Drop(*drag, i, n + usize::from(after)));
    }
}

impl PicassoApp {
    /// Opens Story `id`, saving the one open before.
    pub(super) fn open_story(&mut self, id: i64) {
        if !self.close_story() {
            return;
        }
        let Some(story) = self.db.story(id) else { return };
        let blocks = self.db.story_blocks(id);
        self.selection.clear();
        self.story = Some(StoryEditor {
            saved_cover: story.cover,
            story,
            blocks,
            unsaved: None,
            error: None,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: None,
            photos: HashMap::new(),
            job: None,
            reveal: None,
            picking: None,
        });
    }

    /// Saves and closes the open Story. Returns false, leaving it open with
    /// the reason shown, when it couldn't be saved.
    pub(super) fn close_story(&mut self) -> bool {
        if !self.save_story() {
            return false;
        }
        self.story = None;
        true
    }

    /// Writes the open Story's unsaved changes. Returns whether all is saved.
    pub(super) fn save_story(&mut self) -> bool {
        let Some(editor) = &mut self.story else { return true };
        if editor.unsaved.is_none() {
            return true;
        }
        let id = editor.story.id;
        let saved = self
            .db
            .save_story_details(&editor.story, editor.story.revision)
            .and_then(|revision| {
                if editor.story.cover == editor.saved_cover {
                    Ok(revision)
                } else {
                    self.db.set_story_cover(id, editor.story.cover, revision)
                }
            })
            .and_then(|revision| self.db.save_story_blocks(id, revision, &editor.blocks));
        match saved {
            Ok(revision) => {
                editor.story.revision = revision;
                editor.saved_cover = editor.story.cover;
                editor.unsaved = None;
                editor.error = None;
                true
            }
            Err(err) => {
                editor.error = Some(err);
                false
            }
        }
    }

    /// Saves the open Story once its changes are due.
    pub(super) fn autosave_story(&mut self, ctx: &egui::Context) {
        let Some(editor) = &self.story else { return };
        if editor.due() {
            self.save_story();
        } else if editor.unsaved.is_some() && editor.error.is_none() {
            ctx.request_repaint_after(SAVE_PAUSE);
        }
    }

    /// Whether the open Story is shown in place of the Trip's photos.
    pub(super) fn story_shown(&self) -> bool {
        self.story.as_ref().is_some_and(|editor| self.nav == Nav::Trip(editor.story.trip_id))
    }

    pub(super) fn story_ui(&mut self, ui: &mut egui::Ui) {
        let Some(editor) = &self.story else { return };
        let before = editor.snapshot();
        let trip_title = self.trip_title(editor.story.trip_id);
        let mut back = false;
        let mut undo = false;
        let mut redo = false;
        let mut retry = false;
        let mut archive = false;
        let mut delete = false;
        let mut preview = false;
        let mut export = false;
        // Undo and redo of the Story, while no text field has the keys.
        if !ui.ctx().egui_wants_keyboard_input() {
            ui.ctx().input_mut(|input| {
                redo = input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::Z) || input.consume_key(Modifiers::COMMAND, egui::Key::Y);
                undo = !redo && input.consume_key(Modifiers::COMMAND, egui::Key::Z);
            });
        }

        let editor = self.story.as_ref().unwrap();
        ui.horizontal(|ui| {
            if ui.button(format!("‹ {trip_title}")).on_hover_text(tr("Back to the trip's photos")).clicked() {
                back = true;
            }
            let (status, color) = match (&editor.error, editor.unsaved) {
                (Some(err), _) => (trf("Couldn't save: {}", &[err]), theme::DANGER),
                (None, Some(_)) => (tr("Unsaved").to_string(), theme::muted()),
                (None, None) => (tr("Saved").to_string(), theme::muted()),
            };
            ui.label(RichText::new(status).size(13.0).color(color));
            if editor.error.is_some() && ui.button(tr("Try again")).clicked() {
                retry = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let more = theme::named(ui.button("⋯"), tr("More"));
                egui::Popup::menu(&more).show(|ui| {
                    let label = if editor.story.archived { tr("Restore story") } else { tr("Archive story") };
                    if ui.button(label).clicked() {
                        archive = true;
                    }
                    if ui.button(tr("Delete story…")).clicked() {
                        delete = true;
                    }
                });
                if let Some(job) = &editor.job {
                    if ui.button(tr("Cancel")).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    let (done, total) = (job.done.load(Ordering::Relaxed), job.total.load(Ordering::Relaxed));
                    ui.label(RichText::new(trf("Rendering photos: {} of {}", &[&done, &total])).size(13.0).color(theme::muted()));
                    ui.add(egui::Spinner::new().size(13.0));
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                } else {
                    if ui.button(tr("Export…")).clicked() {
                        export = true;
                    }
                    let button = ui.button(tr("Preview"));
                    if button.on_hover_text(tr("Open the story in your web browser, as it will look when exported")).clicked() {
                        preview = true;
                    }
                }
                ui.add_space(8.0);
                if ui.add_enabled(!editor.redo.is_empty(), egui::Button::new(tr("Redo"))).clicked() {
                    redo = true;
                }
                if ui.add_enabled(!editor.undo.is_empty(), egui::Button::new(tr("Undo"))).clicked() {
                    undo = true;
                }
            });
        });
        ui.add_space(6.0);

        let mut actions = Vec::new();
        let mut typed: Option<(String, u8)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(760.0);
            self.story_details(ui, &mut actions, &mut typed);
            ui.add_space(10.0);
            let count = self.story.as_ref().map_or(0, |editor| editor.blocks.len());
            if count == 0 {
                ui.label(
                    RichText::new(tr(
                        "This story is empty. Add text below, or select photos in the trip and choose Add to Story.",
                    ))
                    .color(theme::muted()),
                );
            }
            for index in 0..count {
                self.story_block(ui, index, count, &mut actions, &mut typed);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Add")).size(13.0).color(theme::muted()));
                for (label, block) in new_blocks() {
                    if ui.button(tr(label)).clicked() {
                        actions.push(Action::Insert(count, block));
                    }
                }
                grid_menu(ui, count, &mut actions);
            });
            ui.add_space(20.0);
        });

        let Some(editor) = &mut self.story else { return };
        let structural = !actions.is_empty();
        for action in actions {
            apply(editor, action);
        }
        let now = editor.snapshot();
        if now != before {
            // A run of typing in one field is one step; anything else is its own.
            if structural || typed.is_none() || typed != editor.typing {
                editor.undo.push(before);
                editor.redo.clear();
            }
            editor.typing = if structural { None } else { typed };
            editor.touch();
        }
        if undo {
            if let Some(previous) = editor.undo.pop() {
                editor.redo.push(editor.snapshot());
                editor.restore(previous);
            }
        } else if redo {
            if let Some(next) = editor.redo.pop() {
                editor.undo.push(editor.snapshot());
                editor.restore(next);
            }
        }
        if retry {
            editor.error = None;
            self.save_story();
        }
        if archive {
            if self.save_story() {
                let editor = self.story.as_ref().unwrap();
                self.db.set_story_archived(editor.story.id, !editor.story.archived);
                self.story = None;
            }
            return;
        }
        if delete {
            let editor = self.story.as_ref().unwrap();
            self.modal = Modal::DeleteStory(editor.story.id, story_name(&editor.story));
            return;
        }
        if preview {
            self.preview_story(ui.ctx());
        }
        if export {
            self.ask_export_story();
        }
        if back {
            self.close_story();
        }
    }

    /// Writes the Story as a web page in a private folder of the cache, with
    /// its photos rendered from their edits, and opens it in the browser.
    fn preview_story(&mut self, ctx: &egui::Context) {
        if !self.save_story() {
            return;
        }
        let Some(job) = self.story_job() else { return };
        let progress = self.start_story_job();
        let story_id = job.story.id;
        let folder = self.cache_dir.join("story-preview").join(job.story.id.to_string());
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-preview".into()).spawn(move || {
            // The folder is the preview's own; an earlier preview is replaced.
            let _ = std::fs::remove_dir_all(&folder);
            let outcome = story_html::write_story(&folder, story_html::Format::Html, story_html::Profile::default(), &job.page(), &progress).and_then(|(index, outcome)| {
                open::that(&index).map_err(|err| err.to_string())?;
                Ok(outcome)
            });
            let _ = events.send(UiEvent::StoryPreviewed(story_id, outcome));
            ctx.request_repaint();
        });
    }

    /// Marks a preview or export of the open Story as started.
    fn start_story_job(&mut self) -> Arc<story_html::Progress> {
        let progress = Arc::new(story_html::Progress::default());
        if let Some(editor) = &mut self.story {
            editor.job = Some(Arc::clone(&progress));
        }
        progress
    }

    /// Marks the job of Story `story_id` as over, if that Story is still open.
    fn end_story_job(&mut self, story_id: i64) {
        if let Some(editor) = self.story.as_mut().filter(|editor| editor.story.id == story_id) {
            editor.job = None;
        }
    }

    /// What a page of the open Story is made from, read on this thread.
    fn story_job(&mut self) -> Option<PageJob> {
        let editor = self.story.as_ref()?;
        let (story, blocks) = (editor.story.clone(), editor.blocks.clone());
        let media: Vec<i64> = story.cover.into_iter().chain(blocks.iter().flat_map(|block| block.block.media().iter().map(|m| m.media_id))).collect();
        let mut sources = HashMap::new();
        for media_id in media {
            let source = self.story_photo(media_id).map(|photo| (std::path::PathBuf::from(&photo.path), self.edit_settings(photo.id), photo.rotation));
            sources.insert(media_id, source);
        }
        let dates = self.trips.iter().find(|trip| trip.id == story.trip_id).and_then(super::trips::trip_dates);
        let words = story_html::Words {
            lang: crate::i18n::current().code(),
            untitled: tr("Untitled story").into(),
            unavailable: tr("Photo unavailable").into(),
        };
        Some(PageJob { story, blocks, sources, dates, words })
    }

    /// The Export dialog, with the settings of the last Story export.
    fn ask_export_story(&mut self) {
        let format = match self.config.text("story_export_format") {
            Some("markdown") => story_html::Format::Markdown,
            _ => story_html::Format::Html,
        };
        let defaults = story_html::Profile::default();
        let edge = self.config.number("story_export_edge").map_or(defaults.edge, |edge| edge as u32);
        let quality = self.config.number("story_export_quality").map_or(defaults.quality, |quality| quality as u8);
        let folder = self.config.text("story_export_folder").map(str::to_string).unwrap_or_default();
        let Some(job) = self.story_job() else { return };
        let check = story_html::preflight(&job.page());
        self.modal = Modal::ExportStory(ExportDialog {
            format,
            profile: story_html::Profile { edge, quality },
            folder,
            error: String::new(),
            check,
            skip_missing: false,
        });
    }

    pub(super) fn export_story_modal(&mut self, ctx: &egui::Context) {
        let Modal::ExportStory(dialog) = &mut self.modal else { return };
        let mut close = false;
        let mut start = false;
        let mut browse = false;
        let mut show: Option<usize> = None;
        let response = egui::Modal::new(egui::Id::unique("export-story")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(440.0);
            about::title_bar(ui, &self.logo, tr("Export story"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.label(tr("Format"));
                    ui.radio_value(&mut dialog.format, story_html::Format::Html, tr("Web page"))
                        .on_hover_text(tr("An index.html that opens in any browser, offline"));
                    ui.radio_value(&mut dialog.format, story_html::Format::Markdown, tr("Markdown"))
                        .on_hover_text(tr("A story.md for blogs and notes that take Markdown"));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Photo size"));
                    egui::ComboBox::from_id_salt("story-edge").selected_text(trf("{} px", &[&dialog.profile.edge])).show_ui(ui, |ui| {
                        for edge in [1200, 1600, 2400] {
                            ui.selectable_value(&mut dialog.profile.edge, edge, trf("{} px", &[&edge]));
                        }
                    });
                    ui.label(tr("Quality"));
                    ui.add(egui::Slider::new(&mut dialog.profile.quality, 60..=95));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Folder"));
                    let width = ui.available_width() - 90.0;
                    ui.add(egui::TextEdit::singleline(&mut dialog.folder).desired_width(width));
                    if ui.button(tr("Choose…")).clicked() {
                        browse = true;
                    }
                });
                ui.label(
                    RichText::new(tr(
                        "A new folder is made there for the story; nothing already there is replaced. Photos are rendered from your edits, without camera details or location.",
                    ))
                    .size(12.0)
                    .color(theme::muted()),
                );
                let check = &dialog.check;
                if check.empty {
                    ui.label(RichText::new(tr("There is nothing in this story to export yet.")).color(theme::DANGER));
                }
                if check.missing > 0 {
                    ui.label(
                        RichText::new(trf("{} photos are unavailable: they are not in the library now, or their files are gone.", &[&check.missing]))
                            .color(theme::DANGER),
                    );
                    ui.checkbox(&mut dialog.skip_missing, tr("Export anyway, with “Photo unavailable” in their place"));
                    show_link(ui, check.first[0], &mut show);
                }
                if check.unreadable > 0 {
                    ui.label(
                        RichText::new(trf("{} photos can't be read: their files may be damaged or in a format Picasso can't open.", &[&check.unreadable]))
                            .color(theme::DANGER),
                    );
                    if check.missing == 0 {
                        ui.checkbox(&mut dialog.skip_missing, tr("Export anyway, with “Photo unavailable” in their place"));
                    }
                    show_link(ui, check.first[3], &mut show);
                }
                let needed = story_html::estimated_bytes(check.photos, dialog.profile);
                let free = story_html::free_space(std::path::Path::new(&dialog.folder));
                if let Some(free) = free.filter(|free| *free < needed) {
                    ui.label(
                        RichText::new(trf(
                            "The export needs about {} but only {} is free there. Choose another folder or a smaller photo size.",
                            &[&crate::types::format_bytes(needed), &crate::types::format_bytes(free)],
                        ))
                        .color(theme::DANGER),
                    );
                }
                if check.no_alt > 0 {
                    ui.label(
                        RichText::new(trf("{} photos have no description for people who can't see them. You can add one under each photo.", &[&check.no_alt]))
                            .color(theme::muted()),
                    );
                    show_link(ui, check.first[1], &mut show);
                }
                if check.unknown > 0 {
                    ui.label(RichText::new(trf("{} blocks from a newer Picasso can't be exported.", &[&check.unknown])).color(theme::muted()));
                    show_link(ui, check.first[2], &mut show);
                }
                if !dialog.error.is_empty() {
                    ui.label(RichText::new(&dialog.error).color(theme::DANGER));
                }
                ui.horizontal(|ui| {
                    if ui.link(tr("Reset to defaults")).clicked() {
                        dialog.format = story_html::Format::Html;
                        dialog.profile = story_html::Profile::default();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let roomy = free.is_none_or(|free| free >= needed);
                        let ready = roomy && !dialog.check.blocked(dialog.skip_missing);
                        if ui.add_enabled(ready, theme::primary_button(tr("Export"))).clicked() {
                            start = true;
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        });
        if browse {
            let mut picker = rfd::FileDialog::new().set_title(tr("Choose where to export the story"));
            if std::path::Path::new(&dialog.folder).is_dir() {
                picker = picker.set_directory(&dialog.folder);
            }
            if let Some(picked) = picker.pick_folder() {
                dialog.folder = picked.display().to_string();
            }
        }
        if let Some(index) = show {
            if let Some(editor) = &mut self.story {
                editor.reveal = Some(index);
            }
            close = true;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
            return;
        }
        if !start {
            return;
        }
        let parent = std::path::PathBuf::from(dialog.folder.trim());
        if !parent.is_dir() {
            dialog.error = tr("Choose a folder to export to").into();
            return;
        }
        let (format, profile) = (dialog.format, dialog.profile);
        self.modal = Modal::None;
        self.config.set("story_export_folder", parent.display().to_string());
        self.config.set("story_export_format", if format == story_html::Format::Markdown { "markdown" } else { "html" });
        self.config.set("story_export_edge", profile.edge as f64);
        self.config.set("story_export_quality", profile.quality as f64);
        if !self.save_story() {
            return;
        }
        let Some(job) = self.story_job() else { return };
        let progress = self.start_story_job();
        let story_id = job.story.id;
        let name = story_name(&job.story);
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-story-export".into()).spawn(move || {
            let outcome = story_html::export_story(&parent, &name, format, profile, &job.page(), &progress)
                .inspect(|(folder, _)| {
                    let _ = open::that(folder);
                });
            let _ = events.send(UiEvent::StoryExported(story_id, outcome));
            ctx.request_repaint();
        });
    }

    pub(super) fn on_story_exported(&mut self, story_id: i64, outcome: Result<(std::path::PathBuf, story_html::Outcome), String>) {
        self.end_story_job(story_id);
        self.activity = match outcome {
            Err(err) if err == story_html::CANCELLED => tr("Export cancelled. Nothing was written.").into(),
            Err(err) if err == story_html::SOURCE_CHANGED => tr("A photo changed while the story was being written. Nothing was written; try again.").into(),
            Ok((folder, outcome)) if outcome.missing > 0 || outcome.unknown > 0 => trf(
                "Exported to {}. {} photos are unavailable and {} blocks from a newer Picasso aren't included.",
                &[&folder.display(), &outcome.missing, &outcome.unknown],
            ),
            Ok((folder, _)) => trf("Exported to {}", &[&folder.display()]),
            Err(err) => trf("Couldn't export the story: {}", &[&err]),
        };
    }

    pub(super) fn on_story_previewed(&mut self, story_id: i64, outcome: Result<story_html::Outcome, String>) {
        self.end_story_job(story_id);
        self.activity = match outcome {
            Err(err) if err == story_html::CANCELLED => tr("Preview cancelled").into(),
            Err(err) if err == story_html::SOURCE_CHANGED => tr("A photo changed while the story was being written. Nothing was written; try again.").into(),
            Ok(outcome) if outcome.missing > 0 || outcome.unknown > 0 => trf(
                "Preview opened. {} photos are unavailable and {} blocks from a newer Picasso aren't shown.",
                &[&outcome.missing, &outcome.unknown],
            ),
            Ok(_) => tr("Preview opened in your browser").into(),
            Err(err) => trf("Couldn't make the preview: {}", &[&err]),
        };
    }

    fn story_details(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>, typed: &mut Option<(String, u8)>) {
        let cover = self.story.as_ref().and_then(|editor| editor.story.cover);
        ui.horizontal(|ui| {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(120.0, 90.0), Sense::click());
            match cover {
                Some(media_id) => {
                    self.paint_media(ui, media_id, rect, true);
                    response.context_menu(|ui| {
                        if ui.button(tr("Remove cover")).clicked() {
                            actions.push(Action::Cover(None));
                            ui.close();
                        }
                    });
                }
                None => {
                    ui.painter().rect_filled(rect, 4, theme::placeholder());
                    let hint = tr("No cover");
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, hint, egui::FontId::proportional(12.0), theme::muted());
                    response.on_hover_text(tr("Right-click a photo in the story to make it the cover"));
                }
            }
            ui.vertical(|ui| {
                let Some(editor) = &mut self.story else { return };
                let story = &mut editor.story;
                let title = egui::TextEdit::singleline(&mut story.title)
                    .hint_text(tr("Untitled story"))
                    .font(egui::FontId::proportional(22.0))
                    .desired_width(f32::INFINITY);
                if ui.add(title).changed() {
                    *typed = Some((String::new(), 0));
                }
                let subtitle = egui::TextEdit::singleline(&mut story.subtitle).hint_text(tr("Subtitle")).desired_width(f32::INFINITY);
                if ui.add(subtitle).changed() {
                    *typed = Some((String::new(), 1));
                }
                ui.horizontal(|ui| {
                    let author = egui::TextEdit::singleline(&mut story.author).hint_text(tr("Author")).desired_width(200.0);
                    if ui.add(author).changed() {
                        *typed = Some((String::new(), 2));
                    }
                    ui.checkbox(&mut story.show_date, tr("Show the trip's dates"));
                    ui.checkbox(&mut story.finished, tr("Finished")).on_hover_text(tr("A story is a draft until you mark it finished."));
                });
            });
        });
    }

    fn story_block(&mut self, ui: &mut egui::Ui, index: usize, count: usize, actions: &mut Vec<Action>, typed: &mut Option<(String, u8)>) {
        let frame = egui::Frame::new()
            .fill(theme::sidebar())
            .corner_radius(6)
            .inner_margin(Margin::symmetric(12, 8));
        let shown = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(editor) = &self.story else { return };
            let block = &editor.blocks[index].block;
            let label = block_label(block);
            let uid = editor.blocks[index].uid.clone();
            let media: Vec<i64> = block.media().iter().map(|media| media.media_id).collect();
            let (photo, gallery) = (matches!(block, Block::Photo(_)), matches!(block, Block::Gallery(_)));
            let grid = match block {
                Block::Grid { columns, rows, .. } => Some((*columns, *rows)),
                _ => None,
            };
            let mut pick = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr(label)).size(12.0).color(theme::muted()));
                if photo || gallery || grid.is_some() {
                    layout_menu(ui, index, &uid, media.len(), grid, gallery, actions);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let more = theme::named(ui.button("⋯"), tr("More"));
                    egui::Popup::menu(&more).show(|ui| {
                        if ui.add_enabled(index > 0, egui::Button::new(tr("Move up"))).clicked() {
                            actions.push(Action::Up(index));
                        }
                        if ui.add_enabled(index + 1 < count, egui::Button::new(tr("Move down"))).clicked() {
                            actions.push(Action::Down(index));
                        }
                        if ui.button(tr("Duplicate")).clicked() {
                            actions.push(Action::Duplicate(index));
                        }
                        ui.menu_button(tr("Add below"), |ui| {
                            for (label, block) in new_blocks() {
                                if ui.button(tr(label)).clicked() {
                                    actions.push(Action::Insert(index + 1, block));
                                }
                            }
                            grid_menu(ui, index + 1, actions);
                        });
                        ui.separator();
                        if ui.button(tr("Remove")).clicked() {
                            actions.push(Action::Remove(index));
                        }
                    });
                    if theme::named(ui.add_enabled(index + 1 < count, egui::Button::new("↓")), tr("Move down")).clicked() {
                        actions.push(Action::Down(index));
                    }
                    if theme::named(ui.add_enabled(index > 0, egui::Button::new("↑")), tr("Move up")).clicked() {
                        actions.push(Action::Up(index));
                    }
                });
            });
            if photo {
                {
                    let height = PHOTO_HEIGHT;
                    let width = self.media_width(media[0], height).min(ui.available_width());
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
                    self.paint_media(ui, media[0], rect, false);
                    drag_source(ui, &response, Drag::Placed(index, 0));
                    drop_target(ui, &response, index, 0, actions);
                    response.context_menu(|ui| {
                        if ui.button(tr("Use as cover")).clicked() {
                            actions.push(Action::Cover(Some(media[0])));
                            ui.close();
                        }
                    });
                }
            } else if let Some((columns, rows)) = grid {
                // The grid as it will be laid out: places in order, more rows
                // for photos beyond them, and empty places outlined.
                let gap = 4.0;
                let cell = ((ui.available_width().min(560.0) - gap * f32::from(columns - 1)) / f32::from(columns)).min(160.0);
                let height = if columns == 1 { cell * 2.0 / 3.0 } else { cell * 3.0 / 4.0 };
                let places = usize::from(columns) * usize::from(rows).max(media.len().div_ceil(usize::from(columns)));
                for row in 0..places / usize::from(columns) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for column in 0..usize::from(columns) {
                            let n = row * usize::from(columns) + column;
                            let (rect, response) = ui.allocate_exact_size(Vec2::new(cell, height), Sense::click_and_drag());
                            let Some(&media_id) = media.get(n) else {
                                let stroke = egui::Stroke::new(1.0, theme::muted().gamma_multiply(0.5));
                                ui.painter().rect_stroke(rect, 4, stroke, egui::StrokeKind::Inside);
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(20.0), theme::muted());
                                drop_target(ui, &response, index, media.len(), actions);
                                if response.on_hover_text(tr("Add photos from the trip, or drop one here")).clicked() {
                                    pick = true;
                                }
                                continue;
                            };
                            self.paint_media(ui, media_id, rect, true);
                            drag_source(ui, &response, Drag::Placed(index, n));
                            drop_target(ui, &response, index, n, actions);
                            response.context_menu(|ui| {
                                if ui.button(tr("Use as cover")).clicked() {
                                    actions.push(Action::Cover(Some(media_id)));
                                    ui.close();
                                }
                                if ui.button(tr("Remove from grid")).clicked() {
                                    actions.push(Action::RemoveFromGallery(index, n));
                                    ui.close();
                                }
                            });
                        }
                    });
                    ui.add_space(gap - ui.spacing().item_spacing.y);
                }
            } else if gallery {
                {
                    ui.horizontal_wrapped(|ui| {
                        for (n, &media_id) in media.iter().enumerate() {
                            let (rect, response) = ui.allocate_exact_size(Vec2::splat(GALLERY_TILE), Sense::click_and_drag());
                            self.paint_media(ui, media_id, rect, true);
                            drag_source(ui, &response, Drag::Placed(index, n));
                            drop_target(ui, &response, index, n, actions);
                            response.context_menu(|ui| {
                                if ui.button(tr("Use as cover")).clicked() {
                                    actions.push(Action::Cover(Some(media_id)));
                                    ui.close();
                                }
                                if ui.button(tr("Remove from gallery")).clicked() {
                                    actions.push(Action::RemoveFromGallery(index, n));
                                    ui.close();
                                }
                            });
                        }
                    });
                }
            }
            if gallery || grid.is_some() {
                self.photo_picker(ui, index, &uid, &media, pick, actions);
            }
            let Some(editor) = &mut self.story else { return };
            match &mut editor.blocks[index].block {
                Block::Heading(text) => {
                    let edit = egui::TextEdit::singleline(text)
                        .hint_text(tr("Heading"))
                        .font(egui::FontId::proportional(18.0))
                        .desired_width(f32::INFINITY);
                    if ui.add(edit).changed() {
                        *typed = Some((uid, 0));
                    }
                }
                Block::Paragraph(text) | Block::Credits(text) => {
                    let edit = egui::TextEdit::multiline(text).hint_text(tr("Write here")).desired_rows(3).desired_width(f32::INFINITY);
                    if ui.add(edit).changed() {
                        *typed = Some((uid, 0));
                    }
                }
                Block::Photo(media) => media_words(ui, media, &uid, 0, typed),
                Block::Grid { columns, rows, caption, media } => {
                    let places = usize::from(*columns) * usize::from(*rows);
                    if media.len() > places {
                        ui.label(RichText::new(trf("{} more photos than places: they make more rows", &[&(media.len() - places)])).size(12.0).color(theme::muted()));
                    }
                    let edit = egui::TextEdit::multiline(caption).hint_text(tr("Description of these photos")).desired_rows(2).desired_width(f32::INFINITY);
                    if ui.add(edit).changed() {
                        *typed = Some((uid.clone(), 0));
                    }
                    let missing = media.iter().filter(|item| item.alt.trim().is_empty()).count();
                    let title = if missing > 0 { trf("Alt text ({} missing)", &[&missing]) } else { tr("Alt text").into() };
                    egui::CollapsingHeader::new(title).id_salt(("grid-alt", &uid)).show(ui, |ui| {
                        for (n, item) in media.iter_mut().enumerate() {
                            let alt = egui::TextEdit::singleline(&mut item.alt)
                                .hint_text(trf("What photo {} shows, for people who can't see it", &[&(n + 1)]))
                                .desired_width(f32::INFINITY);
                            if ui.add(alt).changed() {
                                *typed = Some((uid.clone(), n as u8 + 1));
                            }
                        }
                    });
                }
                Block::Gallery(items) => {
                    // Each photo has its own words, in the order shown above.
                    for (n, item) in items.iter_mut().enumerate() {
                        let caption = egui::TextEdit::singleline(&mut item.caption)
                            .hint_text(trf("Caption of photo {}", &[&(n + 1)]))
                            .desired_width(f32::INFINITY);
                        if ui.add(caption).changed() {
                            *typed = Some((uid.clone(), n as u8));
                        }
                    }
                }
                Block::Separator => {
                    ui.separator();
                }
                Block::Unknown { .. } => {
                    ui.label(
                        RichText::new(tr("Made with a newer version of Picasso. It is kept as it is and can be moved or removed."))
                            .color(theme::muted()),
                    );
                }
            }
        });
        if let Some(editor) = self.story.as_mut().filter(|editor| editor.reveal == Some(index)) {
            editor.reveal = None;
            ui.scroll_to_rect(shown.response.rect, Some(Align::Center));
        }
        ui.add_space(6.0);
    }

    /// The Add photos button of a gallery or grid, and while it is on, a
    /// strip of the Trip's photos, one Day or all: a click puts a photo in
    /// the block, or takes it out again.
    fn photo_picker(&mut self, ui: &mut egui::Ui, index: usize, uid: &str, media: &[i64], start: bool, actions: &mut Vec<Action>) {
        let Some(editor) = &self.story else { return };
        let trip_id = editor.story.trip_id;
        let open = editor.picking.as_ref().is_some_and(|picker| picker.uid == uid);
        let mut day = editor.picking.as_ref().filter(|_| open).map_or(DayFilter::Any, |picker| picker.day);
        let mut toggle = start && !open;
        ui.horizontal(|ui| {
            let label = if open { tr("Done") } else { tr("Add photos…") };
            if ui.button(label).on_hover_text(tr("Pick photos of the trip for this block")).clicked() {
                toggle = true;
            }
            if open {
                let days = self.db.trip_days(trip_id);
                let shown = match day {
                    DayFilter::Day(id) => days.iter().find(|d| d.id == id).map(super::trips::day_name).unwrap_or_default(),
                    _ => tr("All days").to_string(),
                };
                egui::ComboBox::from_id_salt(("pick-day", uid)).selected_text(shown).height(320.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut day, DayFilter::Any, tr("All days"));
                    for d in &days {
                        ui.selectable_value(&mut day, DayFilter::Day(d.id), super::trips::day_name(d));
                    }
                });
                ui.label(RichText::new(tr("Click a photo to add it, again to take it out.")).size(12.0).color(theme::muted()));
            }
        });
        let Some(editor) = &mut self.story else { return };
        if toggle {
            editor.picking = if open { None } else { Some(Picker { uid: uid.into(), day: DayFilter::Any, choices: Vec::new() }) };
            if let Some(picker) = &mut editor.picking {
                picker.choices = trip_photos(&self.db, trip_id, picker.day);
            }
            return;
        }
        let Some(picker) = editor.picking.as_mut().filter(|picker| picker.uid == uid) else { return };
        if picker.day != day {
            picker.day = day;
            picker.choices = trip_photos(&self.db, trip_id, day);
        }
        let choices = picker.choices.clone();
        if choices.is_empty() {
            ui.label(RichText::new(tr("No photos here.")).color(theme::muted()));
            return;
        }
        egui::ScrollArea::horizontal().id_salt(("pick-strip", uid)).show(ui, |ui| {
            ui.horizontal(|ui| {
                for media_id in choices {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(PICK_TILE), Sense::click_and_drag());
                    self.paint_media(ui, media_id, rect, true);
                    drag_source(ui, &response, Drag::New(media_id));
                    let chosen = media.contains(&media_id);
                    if chosen {
                        let painter = ui.painter();
                        painter.rect_stroke(rect, 4, egui::Stroke::new(3.0, theme::ACCENT), egui::StrokeKind::Inside);
                        painter.circle_filled(rect.right_top() + Vec2::new(-11.0, 11.0), 8.0, theme::ACCENT);
                        painter.text(rect.right_top() + Vec2::new(-11.0, 11.0), egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(11.0), egui::Color32::WHITE);
                    }
                    let hint = if chosen { tr("In this block. Click to take it out.") } else { tr("Click to add") };
                    if response.on_hover_text(hint).clicked() {
                        actions.push(Action::Toggle(index, media_id));
                    }
                }
            });
        });
    }

    /// Width of a media reference's photo shown `height` tall.
    fn media_width(&mut self, media_id: i64, height: f32) -> f32 {
        let Some(photo) = self.story_photo(media_id) else { return height };
        let (mut w, mut h) = (photo.width.max(1) as f32, photo.height.max(1) as f32);
        if photo.rotation.rem_euclid(180) == 90 {
            std::mem::swap(&mut w, &mut h);
        }
        height * w / h
    }

    fn story_photo(&mut self, media_id: i64) -> Option<Photo> {
        let editor = self.story.as_mut()?;
        let db = &self.db;
        editor.photos.entry(media_id).or_insert_with(|| db.media_photo(media_id).and_then(|id| db.photo(id))).clone()
    }

    /// Draws a Story photo in `rect`, cropped to fill it when `fill`, or a
    /// placeholder that says it is unavailable.
    fn paint_media(&mut self, ui: &egui::Ui, media_id: i64, rect: Rect, fill: bool) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4, theme::placeholder());
        let Some(photo) = self.story_photo(media_id) else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr("Unavailable"), egui::FontId::proportional(12.0), theme::muted());
            return;
        };
        self.ensure_thumb(photo.id, &photo.path);
        if let Some((texture, w, h)) = self.thumb_texture(photo.id) {
            let uv = if fill { theme::cover_uv(w, h) } else { Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)) };
            theme::paint_image_rounded(&painter, &texture, rect, uv, photo.rotation, 4.0);
        }
    }

    pub(super) fn delete_story_modal(&mut self, ctx: &egui::Context) {
        let Modal::DeleteStory(_, name) = &self.modal else { return };
        let mut close = false;
        let mut delete = false;
        let response = egui::Modal::new(egui::Id::unique("delete-story")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(400.0);
            about::title_bar(ui, &self.logo, &trf("Delete “{}”?", &[name]), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.label(tr("Its text and layout are deleted. Its photos stay in the trip and in your library."));
                ui.label(RichText::new(tr("To keep it but hide it, archive it instead.")).color(theme::muted()));
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(theme::danger_button(tr("Delete story"))).clicked() {
                        delete = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if delete {
            let Modal::DeleteStory(id, _) = std::mem::replace(&mut self.modal, Modal::None) else { return };
            if self.story.as_ref().is_some_and(|editor| editor.story.id == id) {
                self.story = None;
            }
            if let Err(err) = self.db.delete_story(id) {
                self.activity = err;
            }
            return;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
        }
    }

    /// Adds the selected photos to the end of Story `story_id`, each as a
    /// photo block or all as one gallery. Photos not in the Trip yet join it.
    pub(super) fn add_selection_to_story(&mut self, story_id: i64, group: Group) {
        let Some(story) = self.db.story(story_id) else { return };
        let photos = self.selection_in_order();
        let media = match self.db.add_to_trip(story.trip_id, &photos) {
            Ok(media) => media,
            Err(err) => {
                self.activity = err;
                return;
            }
        };
        let mut blocks = self.db.story_blocks(story_id);
        let added = media.len();
        if group == Group::Gallery && added > 1 {
            blocks.push(StoryBlock::new(Block::Gallery(media.into_iter().map(MediaUse::new).collect())));
        } else if group == Group::Grid && added > 1 {
            let (columns, rows) = grid_layout_for(added);
            blocks.push(StoryBlock::new(Block::Grid { columns, rows, caption: String::new(), media: media.into_iter().map(MediaUse::new).collect() }));
        } else {
            blocks.extend(media.into_iter().map(|id| StoryBlock::new(Block::Photo(MediaUse::new(id)))));
        }
        self.activity = match self.db.save_story_blocks(story_id, story.revision, &blocks) {
            Ok(_) => trf("Added {} photos to “{}”", &[&added, &story_name(&story)]),
            Err(err) => err,
        };
    }
}

/// A link to the first block with a problem, which closes the dialog to show it.
fn show_link(ui: &mut egui::Ui, first: Option<usize>, show: &mut Option<usize>) {
    if let Some(index) = first {
        if ui.link(trf("Show block {}", &[&(index + 1)])).clicked() {
            *show = Some(index);
        }
    }
}

/// The caption and alt text of one photo, as used in this Story.
fn media_words(ui: &mut egui::Ui, media: &mut MediaUse, uid: &str, field: u8, typed: &mut Option<(String, u8)>) {
    let caption = egui::TextEdit::singleline(&mut media.caption).hint_text(tr("Caption")).desired_width(f32::INFINITY);
    if ui.add(caption).changed() {
        *typed = Some((uid.to_string(), field));
    }
    let alt = egui::TextEdit::singleline(&mut media.alt)
        .hint_text(tr("Description for people who can't see the photo"))
        .desired_width(f32::INFINITY);
    if ui.add(alt).changed() {
        *typed = Some((uid.to_string(), field + 1));
    }
}

fn apply(editor: &mut StoryEditor, action: Action) {
    let blocks = &mut editor.blocks;
    match action {
        Action::Up(i) if i > 0 => blocks.swap(i, i - 1),
        Action::Down(i) if i + 1 < blocks.len() => blocks.swap(i, i + 1),
        Action::Duplicate(i) => {
            let copy = StoryBlock::new(blocks[i].block.clone());
            blocks.insert(i + 1, copy);
        }
        Action::Remove(i) => {
            blocks.remove(i);
        }
        Action::Insert(i, block) => blocks.insert(i.min(blocks.len()), StoryBlock::new(block)),
        Action::Cover(media) => editor.story.cover = media,
        Action::RemoveFromGallery(i, n) => match &mut blocks[i].block {
            Block::Gallery(items) => {
                items.remove(n);
                match items.len() {
                    0 => {
                        blocks.remove(i);
                    }
                    1 => blocks[i].block = Block::Photo(items.remove(0)),
                    _ => {}
                }
            }
            Block::Grid { media, .. } => {
                media.remove(n);
                if media.is_empty() {
                    blocks.remove(i);
                }
            }
            _ => {}
        },
        Action::Toggle(i, media_id) => {
            let at = blocks[i].block.media().iter().position(|item| item.media_id == media_id);
            match (at, &mut blocks[i].block) {
                // A grid being filled stays, even empty, with its layout.
                (Some(n), Block::Grid { media, .. }) => {
                    media.remove(n);
                }
                (Some(n), _) => apply(editor, Action::RemoveFromGallery(i, n)),
                (None, Block::Gallery(items) | Block::Grid { media: items, .. }) => items.push(MediaUse::new(media_id)),
                _ => {}
            }
        }
        Action::Drop(drag, i, n) => {
            // A photo block dropped on itself, or a photo on its own place,
            // changes nothing.
            match drag {
                Drag::Placed(from, _) if from == i && matches!(blocks[i].block, Block::Photo(_)) => return,
                Drag::Placed(from, at) if (from, at) == (i, n) => return,
                _ => {}
            }
            if !matches!(blocks[i].block, Block::Photo(_) | Block::Gallery(_) | Block::Grid { .. }) {
                return;
            }
            let mut emptied = false;
            let item = match drag {
                Drag::New(media_id) => {
                    // Already in this block: the drop moves it instead.
                    match blocks[i].block.media().iter().position(|item| item.media_id == media_id) {
                        Some(at) => return apply(editor, Action::Drop(Drag::Placed(i, at), i, n)),
                        None => MediaUse::new(media_id),
                    }
                }
                Drag::Placed(from, at) => match &mut blocks[from].block {
                    Block::Gallery(items) | Block::Grid { media: items, .. } if at < items.len() => items.remove(at),
                    Block::Photo(media) => {
                        emptied = true;
                        media.clone()
                    }
                    _ => return,
                },
            };
            // Within one block, the places after the one taken move up.
            let n = match drag {
                Drag::Placed(from, at) if from == i && at < n => n - 1,
                _ => n,
            };
            match &mut blocks[i].block {
                Block::Gallery(items) | Block::Grid { media: items, .. } => items.insert(n.min(items.len()), item),
                // Two single photos make a grid of two side by side.
                Block::Photo(there) => {
                    let media = if n == 0 { vec![item, there.clone()] } else { vec![there.clone(), item] };
                    blocks[i].block = Block::Grid { columns: 2, rows: 1, caption: String::new(), media };
                }
                _ => {}
            }
            // A photo block whose photo moved goes; a gallery left with one
            // photo is a photo, and with none goes; a grid keeps its places.
            if let Drag::Placed(from, _) = drag {
                match &mut blocks[from].block {
                    _ if emptied => {
                        blocks.remove(from);
                    }
                    Block::Gallery(items) if items.is_empty() => {
                        blocks.remove(from);
                    }
                    Block::Gallery(items) if items.len() == 1 => blocks[from].block = Block::Photo(items.remove(0)),
                    _ => {}
                }
            }
        }
        Action::Arrange(i, shape) => {
            let count = blocks[i].block.media().len();
            let fits = match shape {
                Shape::Photo => count == 1,
                Shape::Gallery => count >= 2,
                Shape::Grid(..) => true,
            };
            if !fits || !matches!(blocks[i].block, Block::Photo(_) | Block::Gallery(_) | Block::Grid { .. }) {
                return;
            }
            // Each photo keeps its own caption in a grid, unseen, so it is
            // back when the grid is a gallery or a photo again; the grid's
            // description becomes a paragraph below it.
            let (mut media, caption) = match std::mem::replace(&mut blocks[i].block, Block::Separator) {
                Block::Photo(one) => (vec![one], String::new()),
                Block::Gallery(items) => (items, String::new()),
                Block::Grid { caption, media, .. } => (media, caption),
                _ => unreachable!("checked above"),
            };
            blocks[i].block = match shape {
                Shape::Grid(columns, rows) => Block::Grid { columns, rows, caption, media },
                Shape::Gallery | Shape::Photo => {
                    if !caption.trim().is_empty() {
                        blocks.insert(i + 1, StoryBlock::new(Block::Paragraph(caption)));
                    }
                    if shape == Shape::Photo {
                        Block::Photo(media.remove(0))
                    } else {
                        Block::Gallery(media)
                    }
                }
            };
        }
        Action::Up(_) | Action::Down(_) => {}
    }
}

/// The Trip's photos, without videos, in one Day or all, as media
/// references in the Trip's order.
fn trip_photos(db: &crate::db::Database, trip_id: i64, day: DayFilter) -> Vec<i64> {
    let query = MemberQuery { day, ..MemberQuery::default() };
    db.trip_members(trip_id, &query, 0, PICK_LIMIT).into_iter().filter(|member| !member.is_video).map(|member| member.media_id).collect()
}

/// The layout of a photo, gallery, or grid block, shown in its header: one
/// photo, a gallery, or a grid of a size. A choice the photos don't fit,
/// such as one photo for three, is shown but can't be picked.
fn layout_menu(ui: &mut egui::Ui, index: usize, uid: &str, count: usize, grid: Option<(u8, u8)>, gallery: bool, actions: &mut Vec<Action>) {
    let shown = match grid {
        Some((columns, rows)) => trf("Grid {} × {}", &[&columns, &rows]),
        None if gallery => tr("Gallery").to_string(),
        None => tr("Single photo").to_string(),
    };
    let current = match grid {
        Some((columns, rows)) => Shape::Grid(columns, rows),
        None if gallery => Shape::Gallery,
        None => Shape::Photo,
    };
    ui.label(RichText::new("·").color(theme::muted()));
    let combo = egui::ComboBox::from_id_salt(("block-layout", uid)).selected_text(shown).height(360.0).show_ui(ui, |ui| {
        let mut choice = |ui: &mut egui::Ui, shape: Shape, text: String, fits: bool| {
            if ui.add_enabled(fits, egui::Button::selectable(current == shape, text)).clicked() {
                actions.push(Action::Arrange(index, shape));
            }
        };
        choice(ui, Shape::Photo, tr("Single photo").into(), count == 1);
        choice(ui, Shape::Gallery, tr("Gallery").into(), count >= 2);
        ui.separator();
        for (columns, rows) in GRID_LAYOUTS {
            let text = format!("{}  ·  {}", trf("Grid {} × {}", &[&columns, &rows]), trf("{} photos", &[&(columns * rows)]));
            choice(ui, Shape::Grid(columns, rows), text, true);
        }
    });
    combo.response.on_hover_text(tr("How this block's photos are laid out. Drag a photo onto another to put them together."));
}

/// A Photo grid to add at `at`, each layout its own choice; its photos are
/// picked from the Trip afterwards.
fn grid_menu(ui: &mut egui::Ui, at: usize, actions: &mut Vec<Action>) {
    ui.menu_button(tr("Photo grid"), |ui| {
        for (columns, rows) in GRID_LAYOUTS {
            let text = format!("{columns} × {rows}  ·  {}", trf("{} photos", &[&(columns * rows)]));
            if ui.button(text).clicked() {
                actions.push(Action::Insert(at, Block::Grid { columns, rows, caption: String::new(), media: Vec::new() }));
                ui.close();
            }
        }
    });
}

/// The blocks that can be added as text, with their names.
fn new_blocks() -> [(&'static str, Block); 4] {
    [
        ("Heading", Block::Heading(String::new())),
        ("Paragraph", Block::Paragraph(String::new())),
        ("Separator", Block::Separator),
        ("Credits", Block::Credits(String::new())),
    ]
}

fn block_label(block: &Block) -> &'static str {
    match block {
        Block::Heading(_) => "Heading",
        Block::Paragraph(_) => "Paragraph",
        Block::Photo(_) => "Photo",
        Block::Gallery(_) => "Gallery",
        Block::Grid { .. } => "Photo grid",
        Block::Separator => "Separator",
        Block::Credits(_) => "Credits",
        Block::Unknown { .. } => "Unknown block",
    }
}

pub(super) fn story_name(story: &Story) -> String {
    if story.title.trim().is_empty() {
        tr("Untitled story").into()
    } else {
        story.title.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(blocks: Vec<Block>) -> StoryEditor {
        let story = Story {
            id: 1,
            trip_id: 1,
            title: String::new(),
            subtitle: String::new(),
            author: String::new(),
            show_date: true,
            cover: None,
            template: crate::trip::DEFAULT_TEMPLATE.into(),
            archived: false,
            finished: false,
            revision: 3,
            updated_at: 0,
        };
        StoryEditor {
            saved_cover: None,
            story,
            blocks: blocks.into_iter().map(StoryBlock::new).collect(),
            unsaved: None,
            error: None,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: None,
            photos: HashMap::new(),
            job: None,
            reveal: None,
            picking: None,
        }
    }

    fn kinds(editor: &StoryEditor) -> Vec<&str> {
        editor.blocks.iter().map(|block| block.block.kind()).collect()
    }

    #[test]
    fn blocks_move_duplicate_and_keep_their_ids() {
        let mut editor = editor(vec![Block::Heading("Day 1".into()), Block::Paragraph("Rain".into()), Block::Separator]);
        let ids: Vec<String> = editor.blocks.iter().map(|block| block.uid.clone()).collect();
        apply(&mut editor, Action::Down(0));
        assert_eq!(kinds(&editor), ["paragraph", "heading", "separator"]);
        assert_eq!(editor.blocks[1].uid, ids[0], "a moved block keeps its id");
        apply(&mut editor, Action::Up(0));
        assert_eq!(kinds(&editor), ["paragraph", "heading", "separator"], "the first can't go higher");
        apply(&mut editor, Action::Duplicate(0));
        assert_eq!(editor.blocks[0].block, editor.blocks[1].block);
        assert_ne!(editor.blocks[0].uid, editor.blocks[1].uid, "a copy gets its own id");
        apply(&mut editor, Action::Remove(3));
        apply(&mut editor, Action::Insert(99, Block::Credits(String::new())));
        assert_eq!(kinds(&editor), ["paragraph", "paragraph", "heading", "credits"]);
    }

    #[test]
    fn a_gallery_down_to_one_photo_becomes_a_photo() {
        let mut editor = editor(vec![Block::Gallery(vec![MediaUse::new(7), MediaUse::new(8)])]);
        apply(&mut editor, Action::RemoveFromGallery(0, 0));
        assert_eq!(editor.blocks[0].block, Block::Photo(MediaUse::new(8)));
    }

    #[test]
    fn a_gallery_becomes_a_grid_and_back_without_losing_words() {
        let mut captioned = MediaUse::new(7);
        captioned.caption = "Harbour".into();
        let photos = vec![captioned, MediaUse::new(8), MediaUse::new(9)];
        let mut editor = editor(vec![Block::Gallery(photos.clone())]);
        apply(&mut editor, Action::Arrange(0, Shape::Grid(3, 1)));
        assert_eq!(editor.blocks[0].block, Block::Grid { columns: 3, rows: 1, caption: String::new(), media: photos.clone() });
        if let Block::Grid { caption, .. } = &mut editor.blocks[0].block {
            *caption = "A morning by the sea".into();
        }
        apply(&mut editor, Action::Arrange(0, Shape::Gallery));
        assert_eq!(editor.blocks[0].block, Block::Gallery(photos), "each photo's caption comes back");
        assert_eq!(editor.blocks[1].block, Block::Paragraph("A morning by the sea".into()), "the description is kept below");
        apply(&mut editor, Action::Arrange(0, Shape::Grid(2, 2)));
        apply(&mut editor, Action::RemoveFromGallery(0, 0));
        apply(&mut editor, Action::RemoveFromGallery(0, 0));
        assert!(matches!(&editor.blocks[0].block, Block::Grid { media, .. } if media.len() == 1), "a grid of one stays a grid");
        apply(&mut editor, Action::RemoveFromGallery(0, 0));
        assert_eq!(kinds(&editor), ["paragraph"], "an empty grid goes");
    }

    #[test]
    fn photos_are_picked_into_a_grid_and_out_again() {
        let mut editor = editor(vec![Block::Grid { columns: 2, rows: 1, caption: "Two".into(), media: Vec::new() }, Block::Paragraph(String::new())]);
        apply(&mut editor, Action::Toggle(0, 5));
        apply(&mut editor, Action::Toggle(0, 6));
        assert_eq!(editor.blocks[0].block.media(), &[MediaUse::new(5), MediaUse::new(6)][..]);
        apply(&mut editor, Action::Toggle(0, 5));
        apply(&mut editor, Action::Toggle(0, 6));
        assert!(matches!(&editor.blocks[0].block, Block::Grid { columns: 2, rows: 1, media, .. } if media.is_empty()), "an emptied grid keeps its place and layout");
        apply(&mut editor, Action::Toggle(1, 5));
        assert_eq!(editor.blocks[1].block, Block::Paragraph(String::new()), "text blocks take no photos");
    }

    #[test]
    fn dragged_photos_move_within_and_between_blocks() {
        let ids = |block: &Block| block.media().iter().map(|item| item.media_id).collect::<Vec<_>>();
        let grid = |media: &[i64]| Block::Grid { columns: 2, rows: 2, caption: String::new(), media: media.iter().map(|&id| MediaUse::new(id)).collect() };
        let mut editor = editor(vec![grid(&[1, 2, 3]), Block::Gallery(vec![MediaUse::new(7), MediaUse::new(8)]), grid(&[])]);
        apply(&mut editor, Action::Drop(Drag::Placed(0, 0), 0, 3));
        assert_eq!(ids(&editor.blocks[0].block), [2, 3, 1], "to the end of its own grid");
        apply(&mut editor, Action::Drop(Drag::Placed(0, 2), 0, 0));
        assert_eq!(ids(&editor.blocks[0].block), [1, 2, 3], "and back to the start");
        apply(&mut editor, Action::Drop(Drag::Placed(0, 1), 0, 1));
        assert_eq!(ids(&editor.blocks[0].block), [1, 2, 3], "dropped where it was, nothing moves");
        apply(&mut editor, Action::Drop(Drag::Placed(1, 0), 0, 1));
        assert_eq!(ids(&editor.blocks[0].block), [1, 7, 2, 3], "from the gallery into the grid");
        assert_eq!(editor.blocks[1].block, Block::Photo(MediaUse::new(8)), "a gallery of one is a photo");
        apply(&mut editor, Action::Drop(Drag::Placed(0, 3), 2, 0));
        assert_eq!((ids(&editor.blocks[0].block), ids(&editor.blocks[2].block)), (vec![1, 7, 2], vec![3]), "into another grid");
        apply(&mut editor, Action::Drop(Drag::New(9), 2, 0));
        apply(&mut editor, Action::Drop(Drag::New(3), 2, 9));
        assert_eq!(ids(&editor.blocks[2].block), [9, 3], "from the strip; a photo already there only moves");
        apply(&mut editor, Action::Drop(Drag::Placed(2, 0), 2, 9));
        apply(&mut editor, Action::Drop(Drag::Placed(2, 0), 2, 9));
        assert_eq!(ids(&editor.blocks[2].block), [9, 3], "moving to the end twice comes back round");
    }

    #[test]
    fn single_photos_dropped_together_make_a_grid() {
        let photo = |id| Block::Photo(MediaUse::new(id));
        let mut editor = editor(vec![photo(1), photo(2), Block::Paragraph(String::new()), photo(3)]);
        apply(&mut editor, Action::Drop(Drag::Placed(1, 0), 0, 1));
        assert_eq!(kinds(&editor), ["grid", "paragraph", "photo"], "the moved photo's block goes");
        assert_eq!(editor.blocks[0].block.media(), &[MediaUse::new(1), MediaUse::new(2)][..]);
        assert!(matches!(editor.blocks[0].block, Block::Grid { columns: 2, rows: 1, .. }));
        apply(&mut editor, Action::Drop(Drag::Placed(2, 0), 0, 0));
        assert_eq!(kinds(&editor), ["grid", "paragraph"]);
        assert_eq!(editor.blocks[0].block.media()[0], MediaUse::new(3), "dropped before the first");
        apply(&mut editor, Action::Drop(Drag::Placed(0, 0), 1, 0));
        assert_eq!(editor.blocks[0].block.media().len(), 3, "a paragraph takes no photos");
    }

    #[test]
    fn the_layout_menu_turns_a_block_into_another_shape() {
        let mut editor = editor(vec![Block::Photo(MediaUse::new(1))]);
        apply(&mut editor, Action::Arrange(0, Shape::Gallery));
        assert_eq!(editor.blocks[0].block, Block::Photo(MediaUse::new(1)), "one photo is not a gallery");
        apply(&mut editor, Action::Arrange(0, Shape::Grid(3, 2)));
        assert!(matches!(&editor.blocks[0].block, Block::Grid { columns: 3, rows: 2, media, .. } if media.len() == 1));
        apply(&mut editor, Action::Arrange(0, Shape::Grid(2, 2)));
        assert!(matches!(editor.blocks[0].block, Block::Grid { columns: 2, rows: 2, .. }));
        apply(&mut editor, Action::Arrange(0, Shape::Photo));
        assert_eq!(editor.blocks[0].block, Block::Photo(MediaUse::new(1)));
    }

    #[test]
    fn undo_keeps_the_saved_revision_and_marks_the_story_unsaved() {
        let mut editor = editor(vec![Block::Separator]);
        let before = editor.snapshot();
        editor.story.revision = 4;
        editor.restore(before);
        assert_eq!(editor.story.revision, 4);
        assert!(editor.unsaved.is_some());
        assert!(!editor.due(), "not saved in the middle of an edit");
    }
}
