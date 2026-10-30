//! The Story editor: a Trip's narrative as an ordered list of blocks, with
//! undo and redo for the session and autosave to the library.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, Margin, Modifiers, Pos2, Rect, RichText, Sense, Vec2};

use crate::i18n::{tr, trf};
use crate::inline::{self, Mark};
use crate::trip::{grid_layout_for, Block, DayFilter, MediaUse, MemberQuery, Story, StoryBlock, GRID_LAYOUTS};
use crate::story_html;
use crate::types::{Nav, Photo};

use super::{about, theme, Modal, PicassoApp, UiEvent};

/// A pause in typing this long saves.
const SAVE_PAUSE: Duration = Duration::from_secs(1);
/// Changes are saved at the latest this long after the first unsaved one.
const SAVE_WITHIN: Duration = Duration::from_secs(5);
/// Tallest a single photo is shown in the editor.
const PHOTO_MAX_HEIGHT: f32 = 520.0;
/// Height of the tools floating above a block.
const TOOLS_HEIGHT: f32 = 30.0;
/// Height of the gap between blocks, where a block is added or dropped.
const GAP: f32 = 14.0;
/// Photos in a row of a gallery in the editor.
const GALLERY_COLUMNS: u8 = 3;
/// The width of the Story's text and photos, like a page.
const PAGE_WIDTH: f32 = 720.0;
/// The tallest the cover is drawn across the top of the Story.
const COVER_MAX_HEIGHT: f32 = 300.0;
/// The room between a block's outline and its text.
const BLOCK_INSET: i8 = 10;
/// Words read in a minute, for the reading time.
const WORDS_A_MINUTE: usize = 200;
/// Photos in the strip to pick from.
const PICK_TILE: f32 = 72.0;
/// Most photos the strip shows at once; a Day narrows it.
const PICK_LIMIT: usize = 2000;

#[derive(Clone, PartialEq)]
struct Snapshot {
    pub(super) story: Story,
    blocks: Vec<StoryBlock>,
}

pub(super) struct StoryEditor {
    pub(super) story: Story,
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
    /// Each route source's elevation, read from its track files once:
    /// a track's path, or "" for all the Trip's tracks.
    profiles: HashMap<String, Option<crate::elevation::Profile>>,
    /// The preview or export being made, to show how far it is and stop it.
    job: Option<Arc<story_html::Progress>>,
    /// The block to scroll to on the next frame.
    reveal: Option<usize>,
    /// The gallery or grid photos are being picked for, from the Trip.
    picking: Option<Picker>,
    /// The block last clicked, whose tools stay shown, by id.
    active: Option<String>,
    /// The block under the pointer, by id.
    hovered: Option<String>,
    /// The caret and the format buttons of the text fields.
    text: TextCtl,
}

/// A route's elevation in the bottom-right corner of its drawing, as the
/// page's map picture has it (`src/elevation.rs`).
fn elevation_chart(painter: &egui::Painter, rect: Rect, profile: &[[f32; 2]]) {
    let (Some(first), Some(last)) = (profile.first(), profile.last()) else { return };
    let panel = Rect::from_min_size(rect.right_bottom() - Vec2::new(230.0 + 12.0, 96.0 + 12.0), Vec2::new(230.0, 96.0));
    if !rect.contains_rect(panel) {
        return;
    }
    painter.rect_filled(panel, 6, egui::Color32::from_white_alpha(crate::elevation::VEIL));
    let low = profile.iter().map(|point| point[1]).fold(f32::INFINITY, f32::min);
    let high = profile.iter().map(|point| point[1]).fold(f32::NEG_INFINITY, f32::max);
    let span = (high - low).max(20.0);
    let chart = Rect::from_min_max(panel.min + Vec2::new(62.0, 10.0), panel.max - Vec2::new(10.0, 10.0));
    let length = last[0] - first[0];
    let line: Vec<egui::Pos2> = profile
        .iter()
        .map(|&[along, ele]| egui::pos2(chart.left() + (along - first[0]) / length * chart.width(), chart.bottom() - (ele - low) / span * chart.height()))
        .collect();
    // The area under the line, as thin columns, since egui fills only convex shapes.
    let fill = theme::ACCENT.gamma_multiply(0.27);
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        painter.add(egui::Shape::convex_polygon(vec![a, b, egui::pos2(b.x, chart.bottom()), egui::pos2(a.x, chart.bottom())], fill, egui::Stroke::NONE));
    }
    painter.add(egui::Shape::line(line, egui::Stroke::new(2.0, theme::ACCENT)));
    let ink = egui::Color32::from_gray(58);
    let font = egui::FontId::proportional(12.0);
    painter.text(egui::pos2(panel.left() + 10.0, chart.top()), egui::Align2::LEFT_TOP, format!("{} m", high.round() as i64), font.clone(), ink);
    painter.text(egui::pos2(panel.left() + 10.0, chart.bottom()), egui::Align2::LEFT_BOTTOM, format!("{} m", low.round() as i64), font, ink);
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
pub(super) struct PageJob {
    pub(super) story: Story,
    pub(super) blocks: Vec<StoryBlock>,
    pub(super) sources: HashMap<i64, story_html::Source>,
    dates: Option<String>,
    words: story_html::Words,
    pub(super) map: story_html::MapTiles,
    pub(super) profiles: HashMap<String, crate::elevation::Profile>,
}

impl PageJob {
    pub(super) fn page(&self) -> story_html::Page<'_> {
        story_html::Page { story: &self.story, dates: self.dates.as_deref(), blocks: &self.blocks, sources: &self.sources, words: &self.words, map: Some(&self.map), profiles: &self.profiles }
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

#[derive(Clone, Copy)]
enum Section {
    Introduction,
    Conclusion,
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
    /// Splits the text block at `i` at character `n`: the rest goes to a
    /// new paragraph below, with the caret.
    Split(usize, usize),
    /// Removes the empty text block at `i`, the caret going to the end of
    /// the text block before it.
    RemoveEmpty(usize),
    /// Makes the block at `i` another kind, as chosen with "/".
    Replace(usize, Block),
    /// Moves the block at `i` to the gap before block `n`.
    Move(usize, usize),
    /// Splits item `n` of the list at `i` at character `at`: the rest is
    /// a new item below, with the caret.
    ListSplit(usize, usize, usize),
    /// Ends the list at `i` at its empty item `n`, with a paragraph after.
    ListEnd(usize, usize),
    /// Removes the empty item `n` of the list at `i`; the only one turns
    /// the list back into a paragraph.
    ListBack(usize, usize),
    /// Puts a style around the selection of the field last typed in.
    Format(Mark),
    /// Moves the caret from the block at `i` to the text block before it
    /// (true) or after it.
    Step(usize, bool),
    /// Joins the text block at `i` to the end of the text block before it.
    Merge(usize),
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
            profiles: HashMap::new(),
            job: None,
            reveal: None,
            picking: None,
            active: None,
            hovered: None,
            text: TextCtl::default(),
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

    /// Whether the page is about Stories rather than photos: a Story open,
    /// or a Trip's Stories tab. Such a page has no grid to size and no
    /// selected photo for the side panel.
    pub(super) fn stories_page_shown(&self) -> bool {
        self.story_shown() || matches!(self.nav, Nav::Trip(id) if self.trip_stories.contains(&id))
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
        let mut publish = false;
        // Undo and redo of the Story, while no text field has the keys.
        if !ui.ctx().egui_wants_keyboard_input() {
            ui.ctx().input_mut(|input| {
                redo = input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::Z) || input.consume_key(Modifiers::COMMAND, egui::Key::Y);
                undo = !redo && input.consume_key(Modifiers::COMMAND, egui::Key::Z);
            });
        }

        let editor = self.story.as_ref().unwrap();
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("‹ {trip_title}")).on_hover_text(tr("Back to the trip's photos")).clicked() {
                back = true;
            }
            let (status, color) = match (&editor.error, editor.unsaved) {
                (Some(err), _) => (trf("Couldn't save: {}", &[err]), theme::DANGER),
                (None, Some(_)) => (tr("Unsaved").to_string(), theme::muted()),
                (None, None) => (tr("Saved").to_string(), theme::muted()),
            };
            ui.label(RichText::new(status).size(13.0).color(color));
            let words = story_words(editor);
            let minutes = words.div_ceil(WORDS_A_MINUTE).max(1);
            ui.label(RichText::new(trf("{} words · {} min read", &[&words, &minutes])).size(13.0).color(theme::muted()));
            if editor.error.is_some() && ui.button(tr("Try again")).clicked() {
                retry = true;
            }
            theme::end_actions(ui, "story-bar", |ui| {
                if ui.add_enabled(!editor.undo.is_empty(), egui::Button::new(tr("Undo"))).clicked() {
                    undo = true;
                }
                if ui.add_enabled(!editor.redo.is_empty(), egui::Button::new(tr("Redo"))).clicked() {
                    redo = true;
                }
                ui.add_space(8.0);
                if let Some(job) = &editor.job {
                    ui.add(egui::Spinner::new().size(13.0));
                    let (done, total) = (job.done.load(Ordering::Relaxed), job.total.load(Ordering::Relaxed));
                    ui.label(RichText::new(trf("Rendering photos: {} of {}", &[&done, &total])).size(13.0).color(theme::muted()));
                    if ui.button(tr("Cancel")).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                } else {
                    let button = ui.button(tr("Preview"));
                    if button.on_hover_text(tr("Open the story in your web browser, as it will look when exported")).clicked() {
                        preview = true;
                    }
                    if ui.button(tr("Export…")).clicked() {
                        export = true;
                    }
                }
                let more = theme::named(ui.button("⋯"), tr("More"));
                egui::Popup::menu(&more).show(|ui| {
                    if ui.add_enabled(editor.job.is_none(), egui::Button::new(tr("Publish to WordPress…"))).clicked() {
                        publish = true;
                    }
                    ui.separator();
                    let label = if editor.story.archived { tr("Restore story") } else { tr("Archive story") };
                    if ui.button(label).clicked() {
                        archive = true;
                    }
                    ui.separator();
                    if ui.button(tr("Delete story…")).clicked() {
                        delete = true;
                    }
                });
            });
        });
        ui.add_space(6.0);

        let mut actions = Vec::new();
        let mut typed: Option<(String, u8)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |outer| {
            let ui = &mut *outer;
            // A column the width of a page, in the middle like a document.
            let side = ((ui.available_width() - PAGE_WIDTH) / 2.0).max(0.0);
            let column = Rect::from_min_size(ui.cursor().min + Vec2::new(side, 0.0), Vec2::new(ui.available_width().min(PAGE_WIDTH), f32::INFINITY));
            let mut page = ui.new_child(egui::UiBuilder::new().max_rect(column).layout(Layout::top_down(Align::Min)));
            let ui = &mut page;
            // The title and sections start where the blocks' text does.
            egui::Frame::new().inner_margin(Margin::symmetric(BLOCK_INSET, 0)).show(ui, |ui| {
                self.story_details(ui, &mut actions, &mut typed);
                ui.add_space(10.0);
                self.story_section(ui, Section::Introduction, &mut typed);
            });
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
                inserter(ui, index, &mut actions);
                self.story_block(ui, index, count, &mut actions, &mut typed);
            }
            inserter(ui, count, &mut actions);
            // Like the end of a page: a click starts a new paragraph there.
            let start = egui::Button::new(RichText::new(tr("Start writing, or type / to choose a block")).size(15.0).color(theme::muted())).frame(false);
            if ui.add(start).clicked() {
                actions.push(Action::Insert(count, Block::Paragraph(String::new())));
            }
            ui.add_space(10.0);
            egui::Frame::new().inner_margin(Margin::symmetric(BLOCK_INSET, 0)).show(ui, |ui| {
                self.story_section(ui, Section::Conclusion, &mut typed);
            });
            ui.add_space(40.0);
            let used = page.min_rect();
            outer.allocate_rect(used, Sense::hover());
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
        if publish {
            self.ask_publish_story();
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
    pub(super) fn start_story_job(&mut self) -> Arc<story_html::Progress> {
        let progress = Arc::new(story_html::Progress::default());
        if let Some(editor) = &mut self.story {
            editor.job = Some(Arc::clone(&progress));
        }
        progress
    }

    /// Marks the job of Story `story_id` as over, if that Story is still open.
    pub(super) fn end_story_job(&mut self, story_id: i64) {
        if let Some(editor) = self.story.as_mut().filter(|editor| editor.story.id == story_id) {
            editor.job = None;
        }
    }

    /// What a page of the open Story is made from, read on this thread.
    pub(super) fn story_job(&mut self) -> Option<PageJob> {
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
            contents: tr("Contents").into(),
        };
        let map = story_html::MapTiles { source: self.map_source, key: self.carto_key.clone(), cache: self.cache_dir.clone() };
        let mut profiles = HashMap::new();
        for block in &blocks {
            let Block::Route { source, route, .. } = &block.block else { continue };
            if let Some(profile) = self.route_profile(source).filter(|_| !route.is_empty()) {
                profiles.insert(block.uid.clone(), profile);
            }
        }
        Some(PageJob { story, blocks, sources, dates, words, map, profiles })
    }

    /// The elevation of a route drawn from `source`, the one track's path or
    /// "" for all the Trip's tracks, read once while the Story is open.
    fn route_profile(&mut self, source: &str) -> Option<crate::elevation::Profile> {
        let editor = self.story.as_ref()?;
        if let Some(known) = editor.profiles.get(source) {
            return known.clone();
        }
        let paths: Vec<String> = if source.is_empty() {
            self.db.trip_tracks(editor.story.trip_id).into_iter().filter(|link| link.track_id.is_some()).map(|link| link.path).collect()
        } else {
            vec![source.to_string()]
        };
        let paths: Vec<&std::path::Path> = paths.iter().map(std::path::Path::new).collect();
        let profile = crate::elevation::profile_of(&paths);
        self.story.as_mut()?.profiles.insert(source.to_string(), profile.clone());
        profile
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
                if check.low_res > 0 {
                    ui.label(
                        RichText::new(trf("{} photos are small and may look soft across the page.", &[&check.low_res])).color(theme::muted()),
                    );
                    show_link(ui, check.first[4], &mut show);
                }
                if check.no_cover && !check.empty {
                    ui.label(RichText::new(tr("The story has no cover; it starts with its title. Right-click a photo in the story to make it the cover.")).color(theme::muted()));
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

    /// The top of the Story as it reads: the cover across the page, the
    /// title and subtitle in large type, and the byline and settings in a
    /// quiet row below them.
    fn story_details(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>, typed: &mut Option<(String, u8)>) {
        let cover = self.story.as_ref().and_then(|editor| editor.story.cover);
        let width = ui.available_width();
        match cover {
            Some(media_id) => {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(width, (width * 0.42).min(COVER_MAX_HEIGHT)), Sense::click());
                self.paint_media(ui, media_id, rect, true);
                photo_access(ui, &response, rect, tr("Cover"));
                response.context_menu(|ui| {
                    if ui.button(tr("Remove cover")).clicked() {
                        actions.push(Action::Cover(None));
                        ui.close();
                    }
                });
                ui.add_space(16.0);
            }
            None => {
                let hint = format!("{} · {}", tr("No cover"), tr("Right-click a photo in the story to make it the cover"));
                ui.label(RichText::new(hint).size(12.0).color(theme::muted()));
                ui.add_space(6.0);
            }
        }
        let Some(editor) = &mut self.story else { return };
        let story = &mut editor.story;
        let title = egui::TextEdit::singleline(&mut story.title)
            .hint_text(RichText::new(tr("Untitled story")).size(32.0).color(theme::muted()))
            .font(egui::FontId::proportional(32.0))
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY);
        if ui.add(title).changed() {
            *typed = Some((String::new(), 0));
        }
        let subtitle = egui::TextEdit::singleline(&mut story.subtitle)
            .hint_text(RichText::new(tr("Subtitle")).size(18.0).color(theme::muted()))
            .font(egui::FontId::proportional(18.0))
            .text_color(theme::muted())
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY);
        if ui.add(subtitle).changed() {
            *typed = Some((String::new(), 1));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let author = egui::TextEdit::singleline(&mut story.author).hint_text(tr("Author")).frame(egui::Frame::NONE).desired_width(180.0);
            if ui.add(author).changed() {
                *typed = Some((String::new(), 2));
            }
            ui.label(RichText::new("·").color(theme::muted()));
            ui.checkbox(&mut story.show_date, tr("Show the trip's dates"));
            ui.label(RichText::new("·").color(theme::muted()));
            ui.checkbox(&mut story.finished, tr("Finished")).on_hover_text(tr("A story is a draft until you mark it finished."));
            ui.label(RichText::new("·").color(theme::muted()));
            egui::ComboBox::from_id_salt("story-template")
                .selected_text(tr(crate::trip::template_name(&story.template)))
                .show_ui(ui, |ui| {
                    for code in crate::trip::TEMPLATES {
                        ui.selectable_value(&mut story.template, code.to_string(), tr(crate::trip::template_name(code)));
                    }
                })
                .response
                .on_hover_text(tr("How the preview and export look. The blocks stay the same."));
        });
        ui.add_space(8.0);
        ui.separator();
    }

    /// The optional Introduction before the blocks or Conclusion after
    /// them; left empty, the export leaves it out.
    fn story_section(&mut self, ui: &mut egui::Ui, section: Section, typed: &mut Option<(String, u8)>) {
        let Some(StoryEditor { story, text: ctl, .. }) = &mut self.story else { return };
        let (label, hint, text, field) = match section {
            Section::Introduction => (tr("Introduction"), tr("How the story begins (optional)"), &mut story.introduction, 3),
            Section::Conclusion => (tr("Conclusion"), tr("How the story ends (optional)"), &mut story.conclusion, 4),
        };
        ui.label(RichText::new(label).size(13.0).color(theme::muted()));
        if styled_field(ui, text, field_id("story", usize::from(field)), 16.0, true, hint, ctl, None).0.changed() {
            *typed = Some((String::new(), field));
        }
        ui.add_space(6.0);
    }

    /// One block as it reads on the page, with no frame until it is pointed
    /// at or chosen; its tools float above it then.
    fn story_block(&mut self, ui: &mut egui::Ui, index: usize, count: usize, actions: &mut Vec<Action>, typed: &mut Option<(String, u8)>) {
        let Some(editor) = &self.story else { return };
        let uid = editor.blocks[index].uid.clone();
        let active = editor.active.as_deref() == Some(uid.as_str());
        let stroke = if active {
            egui::Stroke::new(1.0, theme::ACCENT)
        } else if editor.hovered.as_deref() == Some(uid.as_str()) {
            egui::Stroke::new(1.0, theme::muted().gamma_multiply(0.4))
        } else {
            egui::Stroke::NONE
        };
        let frame = egui::Frame::new().stroke(stroke).corner_radius(4).inner_margin(Margin::symmetric(BLOCK_INSET, 6));
        let shown = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.block_body(ui, index, &uid, actions, typed);
        });
        let rect = shown.response.rect;
        // The block, or its tools as last drawn: they float over the gap
        // above, so pointing at the gap itself leaves them to its +.
        let tools = egui::Id::unique(("block-tools", &uid));
        // The tools are a layer above the page, so the pointer is checked
        // against where they are, not against what is on top there.
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let over_tools = ui.ctx().memory(|memory| memory.area_rect(tools)).zip(pointer).is_some_and(|(tools, at)| tools.contains(at));
        let pointed = ui.rect_contains_pointer(rect) || over_tools;
        let Some(editor) = &mut self.story else { return };
        if pointed {
            editor.hovered = Some(uid.clone());
            if ui.input(|input| input.pointer.primary_pressed()) {
                editor.active = Some(uid.clone());
            }
        } else if editor.hovered.as_deref() == Some(uid.as_str()) {
            editor.hovered = None;
        }
        if active || pointed {
            let top = (rect.top() - TOOLS_HEIGHT).max(ui.clip_rect().top());
            egui::Area::new(tools)
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(rect.left(), top))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
                        ui.horizontal(|ui| self.block_tools(ui, index, count, &uid, actions));
                    });
                });
        }
        if let Some(editor) = self.story.as_mut().filter(|editor| editor.reveal == Some(index)) {
            editor.reveal = None;
            ui.scroll_to_rect(rect, Some(Align::Center));
        }
    }

    /// The tools of the block pointed at or chosen: a handle to drag it by,
    /// its kind and layout, moving it, and the rest in ⋯.
    fn block_tools(&mut self, ui: &mut egui::Ui, index: usize, count: usize, uid: &str, actions: &mut Vec<Action>) {
        let Some(editor) = &self.story else { return };
        let block = &editor.blocks[index].block;
        let media = block.media().len();
        let gallery = matches!(block, Block::Gallery(_));
        let grid = match block {
            Block::Grid { columns, rows, .. } => Some((*columns, *rows)),
            _ => None,
        };
        let photos = matches!(block, Block::Photo(_)) || gallery || grid.is_some();
        let label = block_label(block);
        let handle = ui.add(egui::Label::new(RichText::new("⋮⋮").size(15.0).color(theme::muted())).sense(Sense::drag()));
        if handle.drag_started() || handle.dragged() {
            handle.dnd_set_drag_payload(MoveBlock(index));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if handle.on_hover_text(tr("Drag to move this block")).hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        ui.label(RichText::new(tr(label)).size(12.0).color(theme::muted()));
        if photos {
            layout_menu(ui, index, uid, media, grid, gallery, actions);
        }
        if is_text(block) {
            text_tools(ui, index, block, actions);
        }
        if theme::named(ui.add_enabled(index > 0, egui::Button::new("↑").small()), tr("Move up")).clicked() {
            actions.push(Action::Up(index));
        }
        if theme::named(ui.add_enabled(index + 1 < count, egui::Button::new("↓").small()), tr("Move down")).clicked() {
            actions.push(Action::Down(index));
        }
        let more = theme::named(ui.add(egui::Button::new("⋯").small()), tr("More"));
        egui::Popup::menu(&more).show(|ui| {
            if ui.button(tr("Duplicate")).clicked() {
                actions.push(Action::Duplicate(index));
            }
            ui.menu_button(tr("Add below"), |ui| block_menu(ui, index + 1, actions));
            ui.separator();
            if ui.button(tr("Remove")).clicked() {
                actions.push(Action::Remove(index));
            }
        });
    }

    /// What a block holds, drawn the width of the page: photos at their
    /// size, and text in the type it will have.
    fn block_body(&mut self, ui: &mut egui::Ui, index: usize, uid: &str, actions: &mut Vec<Action>, typed: &mut Option<(String, u8)>) {
        let Some(editor) = &self.story else { return };
        let block = &editor.blocks[index].block;
        let media: Vec<i64> = block.media().iter().map(|media| media.media_id).collect();
        let names: Vec<String> = block.media().iter().enumerate().map(|(n, media)| photo_name(&media.alt, n)).collect();
        let (photo, gallery) = (matches!(block, Block::Photo(_)), matches!(block, Block::Gallery(_)));
        let grid = match block {
            Block::Grid { columns, rows, .. } => Some((*columns, *rows)),
            _ => None,
        };
        let width = ui.available_width();
        let gap = 4.0;
        let mut pick = false;
        if photo {
            let aspect = self.media_width(media[0], 1.0);
            let height = (width / aspect).min(PHOTO_MAX_HEIGHT);
            let (rect, response) = ui.allocate_exact_size(Vec2::new((height * aspect).min(width), height), Sense::click_and_drag());
            self.paint_media(ui, media[0], rect, false);
            photo_access(ui, &response, rect, &names[0]);
            drag_source(ui, &response, Drag::Placed(index, 0));
            drop_target(ui, &response, index, 0, actions);
            response.context_menu(|ui| {
                if ui.button(tr("Use as cover")).clicked() {
                    actions.push(Action::Cover(Some(media[0])));
                    ui.close();
                }
            });
        } else if let Some((columns, rows)) = grid {
            // The grid as it will be laid out: places in order, more rows
            // for photos beyond them, and empty places outlined.
            let cell = (width - gap * f32::from(columns - 1)) / f32::from(columns);
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
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Add photos…")));
                            theme::focus_ring(ui, &response, rect);
                            drop_target(ui, &response, index, media.len(), actions);
                            if response.on_hover_text(tr("Add photos from the trip, or drop one here")).clicked() {
                                pick = true;
                            }
                            continue;
                        };
                        self.paint_media(ui, media_id, rect, true);
                        photo_access(ui, &response, rect, &names[n]);
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
            let tile = (width - gap * f32::from(GALLERY_COLUMNS - 1)) / f32::from(GALLERY_COLUMNS);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(gap);
                for (n, &media_id) in media.iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(tile), Sense::click_and_drag());
                    self.paint_media(ui, media_id, rect, true);
                    photo_access(ui, &response, rect, &names[n]);
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
        if gallery || grid.is_some() {
            self.photo_picker(ui, index, uid, &media, pick, actions);
        }
        let mut chosen: Option<(String, Vec<Vec<[i32; 2]>>)> = None;
        if let Some(Block::Route { route, source, .. }) = self.story.as_ref().map(|editor| &editor.blocks[index].block) {
            let (route, source) = (route.clone(), source.clone());
            let profile = self.route_profile(&source);
            self.route_drawing(ui, &route, profile.as_deref(), width);
            let open = self.story.as_ref().is_some_and(|editor| editor.active.as_deref() == Some(uid));
            if open || route.is_empty() {
                chosen = self.route_chooser(ui, uid, &source, route.is_empty());
            }
        }
        // Only the Trip's links allowed to be published are offered.
        let shareable: Vec<crate::trip::TripResource> = match self.story.as_ref() {
            Some(editor) if matches!(editor.blocks[index].block, Block::Link { .. }) => self
                .db
                .trip_resources(editor.story.trip_id)
                .into_iter()
                .filter(|resource| resource.kind == crate::trip::ResourceKind::Link && resource.publish)
                .collect(),
            _ => Vec::new(),
        };
        let Some(editor) = &mut self.story else { return };
        // Empty captions show on the chosen block only, so pointing at
        // blocks doesn't make the page below jump.
        let open = editor.active.as_deref() == Some(uid);
        let StoryEditor { blocks, text: ctl, .. } = editor;
        let uid = uid.to_string();
        match &mut blocks[index].block {
            Block::Heading(text) => text_block(ui, text, TextKind::Heading, &uid, index, ctl, actions, typed),
            Block::Subheading(text) => text_block(ui, text, TextKind::Subheading, &uid, index, ctl, actions, typed),
            Block::Paragraph(text) => text_block(ui, text, TextKind::Paragraph, &uid, index, ctl, actions, typed),
            Block::Credits(text) => text_block(ui, text, TextKind::Credits, &uid, index, ctl, actions, typed),
            Block::List { ordered, items } => list_block(ui, *ordered, items, &uid, index, ctl, actions, typed),
            Block::Quote { text, citation } => quote_block(ui, text, citation, &uid, ctl, typed),
            Block::Info { title, items } => info_block(ui, title, items, &uid, typed),
            Block::Link { url, title, note } => link_block(ui, url, title, note, &shareable, &uid, typed),
            Block::Photo(media) => media_words(ui, media, &uid, 0, open, typed),
            Block::Grid { columns, rows, caption, media } => {
                let places = usize::from(*columns) * usize::from(*rows);
                if media.len() > places {
                    ui.label(RichText::new(trf("{} more photos than places: they make more rows", &[&(media.len() - places)])).size(12.0).color(theme::muted()));
                }
                if open || !caption.is_empty() {
                    let edit = egui::TextEdit::multiline(caption)
                        .hint_text(tr("Description of these photos"))
                        .text_color(theme::muted())
                        .frame(egui::Frame::NONE)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY);
                    if ui.add(edit).changed() {
                        *typed = Some((uid.clone(), 0));
                    }
                }
                if !open {
                    return;
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
                for (n, item) in items.iter_mut().enumerate().filter(|(_, item)| open || !item.caption.is_empty()) {
                    let caption = egui::TextEdit::singleline(&mut item.caption)
                        .hint_text(trf("Caption of photo {}", &[&(n + 1)]))
                        .text_color(theme::muted())
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY);
                    if ui.add(caption).changed() {
                        *typed = Some((uid.clone(), n as u8));
                    }
                }
            }
            Block::Separator => {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
            }
            Block::Route { caption, source, route } => {
                if let Some((new_source, new_route)) = chosen {
                    (*source, *route) = (new_source, new_route);
                    *typed = Some((uid.clone(), 0));
                }
                if open || !caption.is_empty() {
                    let edit = egui::TextEdit::singleline(caption)
                        .hint_text(tr("Description of this route"))
                        .text_color(theme::muted())
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY);
                    if ui.add(edit).changed() {
                        *typed = Some((uid.clone(), 0));
                    }
                }
            }
            Block::Unknown { .. } => {
                ui.label(
                    RichText::new(tr("Made with a newer version of Picasso. It is kept as it is and can be moved or removed."))
                        .color(theme::muted()),
                );
            }
        }
    }

    /// A Route block's line on the map, the closest that shows all of it,
    /// from a dot where it starts to a ring where it ends, with the map's
    /// credit and scale.
    fn route_drawing(&mut self, ui: &mut egui::Ui, route: &[Vec<[i32; 2]>], profile: Option<&[[f32; 2]]>, width: f32) {
        let height = (width * 0.55).min(400.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6, theme::placeholder());
        let Some((view, shape)) = crate::trip::route_shape(route, rect.width(), rect.height(), 28.0) else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr("No route yet"), egui::FontId::proportional(14.0), theme::muted());
            return;
        };
        let (lat, lon) = crate::geo::world_to_latlon(view.x, view.y, view.zoom);
        let map = super::places::MapView { lat, lon, zoom: view.zoom, pan: Vec2::ZERO };
        self.paint_tiles(&painter, rect, map);
        let at = |[x, y]: [f32; 2]| rect.min + Vec2::new(x, y);
        for (width, color) in [(6.0, egui::Color32::WHITE), (3.5, theme::ACCENT)] {
            for segment in &shape {
                painter.add(egui::Shape::line(segment.iter().map(|&point| at(point)).collect(), egui::Stroke::new(width, color)));
            }
        }
        let ends = [shape.first().and_then(|segment| segment.first()), shape.last().and_then(|segment| segment.last())];
        for (n, &point) in ends.into_iter().flatten().enumerate() {
            let (fill, ring) = if n == 0 { (theme::ACCENT, egui::Color32::WHITE) } else { (egui::Color32::WHITE, theme::ACCENT) };
            painter.circle(at(point), 6.0, fill, egui::Stroke::new(2.5, ring));
        }
        self.paint_map_notes(&painter, rect, map);
        let length = crate::track::format_distance(crate::trip::route_meters(route));
        let galley = painter.layout_no_wrap(length, egui::FontId::proportional(12.0), egui::Color32::from_gray(40));
        let corner = Rect::from_min_size(rect.left_top() + Vec2::new(8.0, 8.0), galley.size() + Vec2::new(10.0, 4.0));
        painter.rect_filled(corner, 4, egui::Color32::from_white_alpha(210));
        painter.galley(corner.min + Vec2::new(5.0, 2.0), galley, egui::Color32::from_gray(40));
        if let Some(profile) = profile {
            elevation_chart(&painter, rect, profile);
        }
    }

    /// Which of the Trip's tracks a Route block draws: all of them or one.
    /// Returns the choice made, as the block's source and points.
    fn route_chooser(&mut self, ui: &mut egui::Ui, uid: &str, source: &str, empty: bool) -> Option<(String, Vec<Vec<[i32; 2]>>)> {
        let trip_id = self.story.as_ref()?.story.trip_id;
        let links: Vec<_> = self.db.trip_tracks(trip_id).into_iter().filter(|link| link.track_id.is_some()).collect();
        if links.is_empty() {
            ui.label(RichText::new(tr("Link a GPS track to the trip first, in its Tracks row.")).size(12.0).color(theme::muted()));
            return None;
        }
        let days = self.db.trip_days(trip_id);
        let name = |link: &crate::trip::TripTrack| match link.day_id.and_then(|id| days.iter().find(|day| day.id == id)) {
            Some(day) => format!("{} · {}", link.name, super::trips::day_name(day)),
            None => link.name.clone(),
        };
        let all = tr("All the trip's tracks").to_string();
        let shown = if empty {
            tr("Choose a track…").to_string()
        } else if source.is_empty() {
            all.clone()
        } else {
            links.iter().find(|link| link.path == source).map_or_else(|| source.to_string(), name)
        };
        let mut picked: Option<Option<usize>> = None;
        egui::ComboBox::from_id_salt(("route-track", uid)).selected_text(shown).height(320.0).show_ui(ui, |ui| {
            if ui.selectable_label(!empty && source.is_empty(), &all).clicked() {
                picked = Some(None);
            }
            for (n, link) in links.iter().enumerate() {
                if ui.selectable_label(!empty && link.path == source, name(link)).clicked() {
                    picked = Some(Some(n));
                }
            }
        });
        let picked = picked?;
        let chosen: Vec<_> = match picked {
            Some(n) => vec![&links[n]],
            None => links.iter().collect(),
        };
        let tracks: Vec<crate::track::Track> = chosen.iter().filter_map(|link| self.db.track(link.track_id?)).collect();
        let source = picked.map_or_else(String::new, |n| links[n].path.clone());
        Some((source, crate::trip::route_of(&tracks)))
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
                for (n, media_id) in choices.into_iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(PICK_TILE), Sense::click_and_drag());
                    self.paint_media(ui, media_id, rect, true);
                    drag_source(ui, &response, Drag::New(media_id));
                    let chosen = media.contains(&media_id);
                    let name = trf("Photo {}", &[&(n + 1)]);
                    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, chosen, &name));
                    theme::focus_ring(ui, &response, rect);
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

    pub(super) fn story_photo(&mut self, media_id: i64) -> Option<Photo> {
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

/// A block being dragged by its handle, from its place in the Story.
#[derive(Clone, Copy, Debug)]
struct MoveBlock(usize);

/// The kinds of text block, each with its own type size.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextKind {
    Heading,
    Subheading,
    Paragraph,
    Credits,
}

/// The caret and the format buttons of the Story's text fields.
#[derive(Default)]
struct TextCtl {
    /// The field to put the caret in on the next frame: its block's id, the
    /// field in the block, and the character to put it before (`END` for
    /// after the last).
    focus: Option<(String, usize, usize)>,
    /// The field last typed in, and its selection in characters, which the
    /// format buttons act on: the click takes the caret away from it.
    last: Option<(egui::Id, usize, usize)>,
    /// A format button pressed, for the last field on the next frame.
    pending: Option<Mark>,
    /// The choice picked with ↑ and ↓ in the "/" menu.
    slash: usize,
}

/// A caret position after the last character of a field.
const END: usize = usize::MAX;

fn field_id(uid: &str, field: usize) -> egui::Id {
    egui::Id::unique(("block-text", uid, field))
}

/// The text with its styles: the words between marks bold, italic,
/// struck, as code, or as a link, and the marks dimmed while the field is
/// being edited and hidden otherwise, so it reads as it will on the page.
fn styled_job(ui: &egui::Ui, text: &str, size: f32, wrap: f32, marks: bool) -> egui::text::LayoutJob {
    let visuals = ui.visuals();
    let (normal, strong) = (visuals.text_color(), visuals.strong_text_color());
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap;
    let plain = egui::TextFormat::simple(egui::FontId::proportional(size), normal);
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            job.append("\n", 0.0, plain.clone());
        }
        for span in inline::spans(line) {
            let style = &span.style;
            let mut format = plain.clone();
            if span.mark && !marks {
                format.font_id = egui::FontId::proportional(0.5);
                format.color = egui::Color32::TRANSPARENT;
            } else if span.mark {
                format.color = theme::muted().gamma_multiply(0.6);
            } else {
                if style.code {
                    format.font_id = egui::FontId::monospace(size * 0.9);
                    format.background = visuals.faint_bg_color;
                }
                if style.bold {
                    format.color = strong;
                    format.coords = egui::epaint::text::VariationCoords::new([(b"wght", 700.0)]);
                }
                format.italics = style.italic;
                if style.strike {
                    format.strikethrough = egui::Stroke::new(1.0, format.color);
                }
                if style.link.is_some() {
                    format.color = theme::ACCENT;
                    format.underline = egui::Stroke::new(1.0, theme::ACCENT);
                }
            }
            job.append(span.text, 0.0, format);
        }
    }
    // An empty field still has a line of the text's height.
    if job.sections.is_empty() {
        job.append("", 0.0, plain);
    }
    job
}

/// A text field of the Story, drawn with its styles. ⌘B, ⌘I, ⌘⇧X, ⌘E,
/// and ⌘K put bold, italic, struck, code, or a link around the
/// selection, and so do the format buttons, for the field last typed in.
#[allow(clippy::too_many_arguments)]
fn styled_field(
    ui: &mut egui::Ui,
    text: &mut String,
    id: egui::Id,
    size: f32,
    multiline: bool,
    hint: &str,
    ctl: &mut TextCtl,
    focus: Option<usize>,
) -> (egui::Response, Rect) {
    let selection = |ui: &egui::Ui| {
        egui::TextEdit::load_state(ui.ctx(), id).and_then(|state| state.cursor.char_range()).map(|range| {
            let [a, b] = [range.primary.index.0, range.secondary.index.0];
            (a.min(b), a.max(b))
        })
    };
    let mut select: Option<(usize, usize)> = None;
    let mut mark = ctl.pending.filter(|_| ctl.last.is_some_and(|(last, ..)| last == id));
    if mark.is_some() {
        ctl.pending = None;
    } else if ui.memory(|memory| memory.has_focus(id)) {
        let keys = [
            (Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::X, Mark::Strike),
            (Modifiers::COMMAND, egui::Key::B, Mark::Bold),
            (Modifiers::COMMAND, egui::Key::I, Mark::Italic),
            (Modifiers::COMMAND, egui::Key::E, Mark::Code),
            (Modifiers::COMMAND, egui::Key::K, Mark::Link),
        ];
        mark = ui.input_mut(|input| keys.into_iter().find(|&(modifiers, key, _)| input.consume_key(modifiers, key)).map(|(.., mark)| mark));
    }
    if let Some(mark) = mark {
        let (start, end) = ctl.last.filter(|(last, ..)| *last == id).map(|(_, start, end)| (start, end)).or_else(|| selection(ui)).unwrap_or((0, 0));
        let count = text.chars().count();
        select = Some(inline::wrap(text, start.min(count), end.min(count), mark));
    }
    let marks = select.is_some() || focus.is_some() || ui.memory(|memory| memory.has_focus(id));
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap: f32| ui.painter().layout_job(styled_job(ui, buffer.as_str(), size, wrap, marks));
    let edit = if multiline { egui::TextEdit::multiline(text).desired_rows(1) } else { egui::TextEdit::singleline(text) };
    let edit = edit.id(id).frame(egui::Frame::NONE).font(egui::FontId::proportional(size)).hint_text(RichText::new(hint).size(size).color(theme::muted())).desired_width(f32::INFINITY).layouter(&mut layouter);
    let output = edit.show(ui);
    let response = output.response.response.clone();
    // Where the first line of text is drawn, for marks set beside it.
    let first = output.galley.rows.first().map_or(response.rect, |row| row.rect().translate(output.galley_pos.to_vec2()));
    let count = text.chars().count();
    if let Some((start, end)) = select.or(focus.map(|at| (at.min(count), at.min(count)))) {
        response.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let range = egui::text::CCursorRange::two(egui::text::CCursor::new(start), egui::text::CCursor::new(end));
            state.cursor.set_char_range(Some(range));
            state.store(ui.ctx(), id);
        }
    }
    if response.has_focus() || select.is_some() {
        let (start, end) = select.or_else(|| selection(ui)).unwrap_or((0, 0));
        ctl.last = Some((id, start, end));
    }
    (response, first)
}

/// Whether field `field` of block `uid` is to get the caret, and where.
fn take_focus(ctl: &mut TextCtl, uid: &str, field: usize) -> Option<usize> {
    let (_, _, caret) = ctl.focus.take_if(|(wanted, at, _)| wanted == uid && *at == field)?;
    Some(caret)
}

/// The caret of a field, in characters, when nothing is selected.
fn caret(ui: &egui::Ui, id: egui::Id) -> Option<usize> {
    let range = egui::TextEdit::load_state(ui.ctx(), id)?.cursor.char_range()?;
    (range.primary.index == range.secondary.index).then_some(range.primary.index.0)
}

/// ↑ with the caret at the start of a field, or ↓ with it at the end,
/// goes on to the field before or after, as in one page of text.
fn arrow_step(ui: &egui::Ui, id: egui::Id, text: &str) -> Option<bool> {
    let at = caret(ui, id)?;
    ui.input_mut(|input| {
        if at == 0 && input.consume_key(Modifiers::NONE, egui::Key::ArrowUp) {
            Some(true)
        } else if at == text.chars().count() && input.consume_key(Modifiers::NONE, egui::Key::ArrowDown) {
            Some(false)
        } else {
            None
        }
    })
}

/// The blocks named by what follows "/", floating under the paragraph: an
/// icon, the name, and what it is for each, the chosen one marked.
fn slash_menu(ui: &egui::Ui, query: &str, below: Rect, index: usize, ctl: &mut TextCtl, actions: &mut Vec<Action>) {
    const ROW: f32 = 44.0;
    let choices = slash_choices(query);
    if choices.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::unique(("slash-menu", index)))
        .order(egui::Order::Foreground)
        .fixed_pos(below.left_bottom() + Vec2::new(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(Margin::same(6)).corner_radius(10).show(ui, |ui| {
                ui.set_width(300.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(tr("Blocks")).size(11.0).color(theme::muted()));
                for (n, (label, block)) in choices.into_iter().take(SLASH_SHOWN).enumerate() {
                    let (rect, row) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
                    row.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                    if row.hovered() {
                        ctl.slash = n;
                    }
                    let painter = ui.painter();
                    let visuals = ui.visuals();
                    if n == ctl.slash {
                        painter.rect_filled(rect, 6, theme::ACCENT.gamma_multiply(0.14));
                    }
                    let icon = Rect::from_center_size(egui::pos2(rect.left() + 22.0, rect.center().y), Vec2::splat(30.0));
                    painter.rect_filled(icon, 6, visuals.faint_bg_color);
                    painter.rect_stroke(icon, 6, egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
                    painter.text(icon.center(), egui::Align2::CENTER_CENTER, block_icon(&block), egui::FontId::proportional(14.0), visuals.strong_text_color());
                    let left = icon.right() + 10.0;
                    painter.text(egui::pos2(left, rect.top() + 7.0), egui::Align2::LEFT_TOP, &label, egui::FontId::proportional(14.0), visuals.strong_text_color());
                    painter.text(egui::pos2(left, rect.top() + 25.0), egui::Align2::LEFT_TOP, tr(block_hint(&block)), egui::FontId::proportional(11.5), theme::muted());
                    if n == ctl.slash {
                        painter.text(egui::pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, "↵", egui::FontId::proportional(13.0), theme::muted());
                    }
                    if row.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        actions.push(Action::Replace(index, block));
                    }
                }
                ui.add_space(4.0);
                ui.separator();
                ui.label(RichText::new(tr("↑ ↓ to choose, Enter to add")).size(11.0).color(theme::muted()));
            });
        });
}

/// Gives a photo in the Story its name for screen readers, its alt text
/// when it has one, and the focus ring while the keyboard is on it.
fn photo_access(ui: &egui::Ui, response: &egui::Response, rect: Rect, name: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, name));
    theme::focus_ring(ui, response, rect);
}

/// What a photo in a block is called: its alt text, or its place.
fn photo_name(alt: &str, n: usize) -> String {
    match alt.trim() {
        "" => trf("Photo {}", &[&(n + 1)]),
        alt => alt.to_string(),
    }
}

/// The mark of a kind of block in the "/" menu.
fn block_icon(block: &Block) -> &'static str {
    match block {
        Block::Heading(_) => "H2",
        Block::Subheading(_) => "H3",
        Block::Paragraph(_) => "¶",
        Block::List { ordered: false, .. } => "•",
        Block::List { ordered: true, .. } => "1.",
        Block::Quote { .. } => "“",
        Block::Separator => "—",
        Block::Credits(_) => "©",
        Block::Route { .. } => "🗺",
        Block::Info { .. } => "i",
        Block::Link { .. } => "🔗",
        _ => "▦",
    }
}

/// What a kind of block is for, in the "/" menu.
fn block_hint(block: &Block) -> &'static str {
    match block {
        Block::Heading(_) => "A large title for a part of the story",
        Block::Subheading(_) => "A smaller title inside a part",
        Block::Paragraph(_) => "Plain text",
        Block::List { ordered: false, .. } => "Points with bullets",
        Block::List { ordered: true, .. } => "Steps or points in order",
        Block::Quote { .. } => "Words someone said, and who said them",
        Block::Separator => "A line between parts",
        Block::Credits(_) => "Small print, such as photo credits",
        Block::Route { .. } => "A GPS track of the trip, drawn as a line",
        Block::Info { .. } => "Facts to plan with: how to get there, prices, hours",
        Block::Link { .. } => "A web page to read more, with a note",
        _ => "Photos in rows and columns",
    }
}

/// The room before a list's items for their numbers or dots.
const LIST_INDENT: f32 = 24.0;

/// How many blocks the "/" menu shows.
const SLASH_SHOWN: usize = 8;

/// A heading or paragraph typed as it will read. Enter starts a new
/// paragraph from the caret on, Shift+Enter breaks the line, Backspace in
/// an empty block removes it and at the start of one joins it to the text
/// before, ↑ and ↓ at its ends go on to the blocks around it, "/" in an empty paragraph picks another
/// kind of block by name, and "- ", "1. ", or "> " at its start make it
/// a list, a numbered list, or a quote.
#[allow(clippy::too_many_arguments)]
fn text_block(
    ui: &mut egui::Ui,
    text: &mut String,
    kind: TextKind,
    uid: &str,
    index: usize,
    ctl: &mut TextCtl,
    actions: &mut Vec<Action>,
    typed: &mut Option<(String, u8)>,
) {
    let id = field_id(uid, 0);
    // A first line starting with "/" that names a block, spaces and all.
    let slash = kind == TextKind::Paragraph && text.starts_with('/') && !text.contains('\n') && !slash_choices(&text[1..]).is_empty();
    let focused = ui.memory(|memory| memory.has_focus(id));
    if !slash && focused {
        ctl.slash = 0;
    }
    if focused && slash {
        let shown = slash_choices(&text[1..]).len().min(SLASH_SHOWN);
        ui.input_mut(|input| {
            if input.consume_key(Modifiers::NONE, egui::Key::ArrowDown) {
                ctl.slash = (ctl.slash + 1) % shown;
            } else if input.consume_key(Modifiers::NONE, egui::Key::ArrowUp) {
                ctl.slash = (ctl.slash + shown - 1) % shown;
            }
        });
        ctl.slash = ctl.slash.min(shown - 1);
    }
    if focused {
        // Shift+Enter is left to the field, as a line break.
        let enter = ui.input_mut(|input| !input.modifiers.shift && input.consume_key(Modifiers::NONE, egui::Key::Enter));
        if enter && slash {
            if let Some((_, block)) = slash_choices(&text[1..]).into_iter().nth(ctl.slash) {
                actions.push(Action::Replace(index, block));
            }
        } else if let Some(up) = (!slash).then(|| arrow_step(ui, id, text)).flatten() {
            actions.push(Action::Step(index, up));
        } else if !text.is_empty() && caret(ui, id) == Some(0) && ui.input_mut(|input| input.consume_key(Modifiers::NONE, egui::Key::Backspace)) {
            actions.push(Action::Merge(index));
        } else if enter {
            let at = egui::TextEdit::load_state(ui.ctx(), id)
                .and_then(|state| state.cursor.char_range())
                .map_or(text.chars().count(), |range| range.primary.index.0);
            actions.push(Action::Split(index, at));
        } else if text.is_empty() && ui.input_mut(|input| input.consume_key(Modifiers::NONE, egui::Key::Backspace)) {
            actions.push(Action::RemoveEmpty(index));
        }
    }
    let (size, hint) = match kind {
        TextKind::Heading => (24.0, tr("Heading")),
        TextKind::Subheading => (19.0, tr("Subheading")),
        TextKind::Paragraph => (16.0, tr("Type / to choose a block")),
        TextKind::Credits => (13.0, tr("Credits")),
    };
    let focus = take_focus(ctl, uid, 0);
    let (response, _) = styled_field(ui, text, id, size, kind != TextKind::Heading && kind != TextKind::Subheading, hint, ctl, focus);
    if response.changed() {
        *typed = Some((uid.to_string(), 0));
        if kind == TextKind::Paragraph {
            let shortcut = match text.as_str() {
                "- " | "* " => Some(Block::List { ordered: false, items: vec![String::new()] }),
                "1. " => Some(Block::List { ordered: true, items: vec![String::new()] }),
                "> " => Some(Block::Quote { text: String::new(), citation: String::new() }),
                _ => None,
            };
            if let Some(block) = shortcut {
                actions.push(Action::Replace(index, block));
            }
        }
    }
    // Typing in the field may have taken the "/" away this frame.
    if let Some(query) = text.strip_prefix('/').filter(|query| slash && !query.contains('\n')) {
        slash_menu(ui, query, response.rect, index, ctl, actions);
    }
}

/// A list, one field an item, each with its bullet or number. Enter makes
/// a new item from the caret on, Enter on an empty item ends the list there
/// with a paragraph, and Backspace on an empty item removes it.
#[allow(clippy::too_many_arguments)]
fn list_block(
    ui: &mut egui::Ui,
    ordered: bool,
    items: &mut [String],
    uid: &str,
    index: usize,
    ctl: &mut TextCtl,
    actions: &mut Vec<Action>,
    typed: &mut Option<(String, u8)>,
) {
    let count = items.len();
    for (n, item) in items.iter_mut().enumerate() {
        let id = field_id(uid, n);
        if ui.memory(|memory| memory.has_focus(id)) {
            let enter = ui.input_mut(|input| !input.modifiers.shift && input.consume_key(Modifiers::NONE, egui::Key::Enter));
            if enter && item.trim().is_empty() {
                actions.push(Action::ListEnd(index, n));
            } else if enter {
                let at = egui::TextEdit::load_state(ui.ctx(), id)
                    .and_then(|state| state.cursor.char_range())
                    .map_or(item.chars().count(), |range| range.primary.index.0);
                actions.push(Action::ListSplit(index, n, at));
            } else if item.is_empty() && ui.input_mut(|input| input.consume_key(Modifiers::NONE, egui::Key::Backspace)) {
                actions.push(Action::ListBack(index, n));
            } else if let Some(up) = arrow_step(ui, id, item) {
                match (up, n) {
                    (true, 0) => actions.push(Action::Step(index, true)),
                    (true, _) => ctl.focus = Some((uid.to_string(), n - 1, END)),
                    (false, _) if n + 1 == count => actions.push(Action::Step(index, false)),
                    (false, _) => ctl.focus = Some((uid.to_string(), n + 1, 0)),
                }
            }
        }
        ui.horizontal(|ui| {
            // A column for the number or dot, drawn on the middle of the
            // item's first line once the field has its place.
            ui.add_space(LIST_INDENT);
            let focus = take_focus(ctl, uid, n);
            let (field, line) = styled_field(ui, item, id, 16.0, false, tr("List item"), ctl, focus);
            if field.changed() {
                *typed = Some((uid.to_string(), n as u8));
            }
            // Laid out like the item's text from the top of its first line,
            // so the number sits on the same baseline.
            let left = field.rect.left() - LIST_INDENT;
            let color = ui.visuals().text_color();
            if ordered {
                let font = egui::FontId::proportional(16.0);
                ui.painter().text(egui::pos2(left, line.top()), egui::Align2::LEFT_TOP, format!("{}.", n + 1), font, color);
            } else {
                ui.painter().circle_filled(egui::pos2(left + 6.0, line.center().y), 2.5, color);
            }
        });
    }
}

/// Quoted words, and below them who said or wrote them.
#[allow(clippy::too_many_arguments)]
fn quote_block(ui: &mut egui::Ui, text: &mut String, citation: &mut String, uid: &str, ctl: &mut TextCtl, typed: &mut Option<(String, u8)>) {
    let frame = egui::Frame::new().inner_margin(Margin { left: 14, ..Margin::ZERO });
    let shown = frame.show(ui, |ui| {
        let focus = take_focus(ctl, uid, 0);
        if styled_field(ui, text, field_id(uid, 0), 17.0, true, tr("Quote"), ctl, focus).0.changed() {
            *typed = Some((uid.to_string(), 0));
        }
        let focus = take_focus(ctl, uid, 1);
        if styled_field(ui, citation, field_id(uid, 1), 13.0, false, tr("Who said it (optional)"), ctl, focus).0.changed() {
            *typed = Some((uid.to_string(), 1));
        }
    });
    let rect = shown.response.rect;
    ui.painter().vline(rect.left() + 2.0, rect.y_range(), egui::Stroke::new(3.0, theme::muted().gamma_multiply(0.5)));
}

/// A Practical info block: its title, and label and value lines that can
/// be added and removed.
fn info_block(ui: &mut egui::Ui, title: &mut String, items: &mut Vec<(String, String)>, uid: &str, typed: &mut Option<(String, u8)>) {
    let frame = egui::Frame::new().stroke(egui::Stroke::new(1.0, theme::hairline())).corner_radius(6).inner_margin(Margin::symmetric(12, 10));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let edit = egui::TextEdit::singleline(title).hint_text(tr("Practical info")).font(egui::FontId::proportional(16.0)).frame(egui::Frame::NONE).desired_width(f32::INFINITY);
        if ui.add(edit).changed() {
            *typed = Some((uid.to_string(), 0));
        }
        let mut remove = None;
        for (n, (label, value)) in items.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let field = n as u8 * 2 + 1;
                if ui.add(egui::TextEdit::singleline(label).hint_text(tr("Label, such as Getting there")).desired_width(170.0)).changed() {
                    *typed = Some((uid.to_string(), field));
                }
                let width = ui.available_width() - 30.0;
                if ui.add(egui::TextEdit::singleline(value).hint_text(tr("Details")).desired_width(width)).changed() {
                    *typed = Some((uid.to_string(), field + 1));
                }
                if theme::named(ui.small_button("×"), tr("Remove this line")).clicked() {
                    remove = Some(n);
                }
            });
        }
        if let Some(n) = remove {
            items.remove(n);
            *typed = Some((String::new(), 0));
        }
        if ui.small_button(tr("Add a line")).clicked() {
            items.push((String::new(), String::new()));
            *typed = Some((String::new(), 0));
        }
    });
}

/// A Link block: the title shown, the web address, and a note. A link
/// the Trip keeps and allows to be published can fill it in.
fn link_block(ui: &mut egui::Ui, url: &mut String, title: &mut String, note: &mut String, shareable: &[crate::trip::TripResource], uid: &str, typed: &mut Option<(String, u8)>) {
    let frame = egui::Frame::new().inner_margin(Margin { left: 14, ..Margin::ZERO });
    let shown = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let edit = egui::TextEdit::singleline(title).hint_text(tr("Link title")).font(egui::FontId::proportional(16.0)).frame(egui::Frame::NONE).desired_width(f32::INFINITY);
        if ui.add(edit).changed() {
            *typed = Some((uid.to_string(), 0));
        }
        ui.horizontal(|ui| {
            let width = ui.available_width() - if shareable.is_empty() { 0.0 } else { 150.0 };
            if ui.add(egui::TextEdit::singleline(url).hint_text("https://").desired_width(width)).changed() {
                *typed = Some((uid.to_string(), 1));
            }
            if !shareable.is_empty() {
                let button = ui.button(tr("From the trip…")).on_hover_text(tr("Links kept with the trip that may be published"));
                egui::Popup::menu(&button).show(|ui| {
                    for resource in shareable {
                        if ui.button(resource.shown_name()).on_hover_text(&resource.reference).clicked() {
                            *url = resource.reference.clone();
                            *title = resource.title.clone();
                            *note = resource.notes.clone();
                            *typed = Some((String::new(), 0));
                        }
                    }
                });
            }
        });
        if !url.trim().is_empty() && crate::trip::web_address(url).is_none() {
            ui.label(RichText::new(tr("Type a web address starting with https://")).size(12.0).color(theme::DANGER));
        }
        let edit = egui::TextEdit::multiline(note).hint_text(tr("What it is for (optional)")).text_color(theme::muted()).frame(egui::Frame::NONE).desired_rows(1).desired_width(f32::INFINITY);
        if ui.add(edit).changed() {
            *typed = Some((uid.to_string(), 2));
        }
    });
    let rect = shown.response.rect;
    ui.painter().vline(rect.left() + 2.0, rect.y_range(), egui::Stroke::new(3.0, theme::ACCENT));
}

/// The format buttons of a text block: styles for the selection, the
/// heading's level, and the list's kind.
fn text_tools(ui: &mut egui::Ui, index: usize, block: &Block, actions: &mut Vec<Action>) {
    ui.separator();
    let styles = [
        (RichText::new("B").strong(), Mark::Bold, "Bold (⌘B)"),
        (RichText::new("I").italics(), Mark::Italic, "Italic (⌘I)"),
        (RichText::new("S").strikethrough(), Mark::Strike, "Strikethrough (⌘⇧X)"),
        (RichText::new("</>").monospace(), Mark::Code, "Code (⌘E)"),
        (RichText::new("🔗"), Mark::Link, "Link (⌘K)"),
    ];
    for (label, mark, name) in styles {
        if theme::named(ui.add(egui::Button::new(label).small()), tr(name)).clicked() {
            actions.push(Action::Format(mark));
        }
    }
    let other = match block {
        Block::Heading(text) => Some(("H3", "Make it a subheading", Block::Subheading(text.clone()))),
        Block::Subheading(text) => Some(("H2", "Make it a heading", Block::Heading(text.clone()))),
        Block::Paragraph(text) => {
            let items = text.split('\n').map(str::to_string).collect();
            Some(("•", "Make it a list", Block::List { ordered: false, items }))
        }
        Block::List { ordered: false, items } => Some(("1.", "Make it a numbered list", Block::List { ordered: true, items: items.clone() })),
        Block::List { ordered: true, items } => Some(("•", "Make it a bulleted list", Block::List { ordered: false, items: items.clone() })),
        _ => None,
    };
    if let Some((label, name, block)) = other {
        ui.separator();
        if theme::named(ui.add(egui::Button::new(label).small()), tr(name)).clicked() {
            actions.push(Action::Replace(index, block));
        }
    }
}

/// Blocks whose name has `query` in it, in English or as shown, for "/".
fn slash_choices(query: &str) -> Vec<(String, Block)> {
    let query = query.to_lowercase();
    let mut all: Vec<(String, String, Block)> = new_blocks().into_iter().map(|(name, block)| (tr(name).to_string(), name.to_lowercase(), block)).collect();
    for (columns, rows) in GRID_LAYOUTS {
        let label = format!("{} {columns} × {rows}", tr("Photo grid"));
        let block = Block::Grid { columns, rows, caption: String::new(), media: Vec::new() };
        all.push((label, format!("photo grid {columns}x{rows}"), block));
    }
    all.into_iter().filter(|(label, english, _)| english.contains(&query) || label.to_lowercase().contains(&query)).map(|(label, _, block)| (label, block)).collect()
}

/// The gap before block `at`: a line with + when pointed at, which opens
/// the blocks to add there, and where a block dragged by its handle lands.
fn inserter(ui: &mut egui::Ui, at: usize, actions: &mut Vec<Action>) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), GAP), Sense::click());
    let moving = response.dnd_hover_payload::<MoveBlock>().is_some();
    if moving {
        ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(3.0, theme::ACCENT));
    } else if response.hovered() {
        // The + at the right end, clear of the tools floating at the left.
        let plus = egui::pos2(rect.right() - GAP, rect.center().y);
        ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, theme::ACCENT));
        ui.painter().circle_filled(plus, GAP / 2.0 + 1.0, theme::ACCENT);
        ui.painter().text(plus, egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(14.0), egui::Color32::WHITE);
    }
    if let Some(moved) = response.dnd_release_payload::<MoveBlock>() {
        actions.push(Action::Move(moved.0, at));
    }
    let response = response.on_hover_text(tr("Add a block here"));
    egui::Popup::menu(&response).show(|ui| block_menu(ui, at, actions));
}

/// The blocks that can be added at `at`.
fn block_menu(ui: &mut egui::Ui, at: usize, actions: &mut Vec<Action>) {
    for (label, block) in new_blocks() {
        if ui.button(tr(label)).clicked() {
            actions.push(Action::Insert(at, block));
        }
    }
    grid_menu(ui, at, actions);
}

/// The caption and alt text of one photo, as used in this Story.
/// A photo's caption and alt text. Empty, they stay out of the way until
/// the block is chosen, and the alt text only shows then.
fn media_words(ui: &mut egui::Ui, media: &mut MediaUse, uid: &str, field: u8, open: bool, typed: &mut Option<(String, u8)>) {
    if open || !media.caption.is_empty() {
        let caption = egui::TextEdit::singleline(&mut media.caption)
            .hint_text(tr("Caption"))
            .text_color(theme::muted())
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY);
        if ui.add(caption).changed() {
            *typed = Some((uid.to_string(), field));
        }
    }
    if !open {
        return;
    }
    let alt = egui::TextEdit::singleline(&mut media.alt)
        .hint_text(tr("Description for people who can't see the photo"))
        .frame(egui::Frame::NONE)
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
        Action::Insert(i, block) => {
            let block = StoryBlock::new(block);
            if is_text(&block.block) {
                editor.text.focus = Some((block.uid.clone(), 0, 0));
            }
            blocks.insert(i.min(blocks.len()), block);
        }
        Action::Split(i, n) => {
            let rest = match &mut blocks[i].block {
                Block::Heading(text) | Block::Subheading(text) | Block::Paragraph(text) | Block::Credits(text) => {
                    let at = text.char_indices().nth(n).map_or(text.len(), |(at, _)| at);
                    text.split_off(at)
                }
                _ => return,
            };
            let rest = if matches!(blocks[i].block, Block::Credits(_)) { Block::Credits(rest) } else { Block::Paragraph(rest) };
            let block = StoryBlock::new(rest);
            editor.text.focus = Some((block.uid.clone(), 0, 0));
            blocks.insert(i + 1, block);
        }
        Action::RemoveEmpty(i) => {
            blocks.remove(i);
            if let Some(before) = i.checked_sub(1).and_then(|i| blocks.get(i)).filter(|block| is_text(&block.block)) {
                let last = match &before.block {
                    Block::List { items, .. } => items.len().saturating_sub(1),
                    _ => 0,
                };
                editor.text.focus = Some((before.uid.clone(), last, END));
            }
        }
        Action::Replace(i, block) => {
            if is_text(&block) {
                editor.text.focus = Some((blocks[i].uid.clone(), 0, END));
            }
            blocks[i].block = block;
        }
        Action::ListSplit(i, n, at) => {
            if let Block::List { items, .. } = &mut blocks[i].block {
                let item = &mut items[n];
                let rest = item.split_off(item.char_indices().nth(at).map_or(item.len(), |(at, _)| at));
                items.insert(n + 1, rest);
                editor.text.focus = Some((blocks[i].uid.clone(), n + 1, 0));
            }
        }
        Action::ListEnd(i, n) => {
            if let Block::List { ordered, items } = &mut blocks[i].block {
                // The items after the empty one go on as a list of their own,
                // after the new paragraph.
                let after = items.split_off(n + 1);
                items.pop();
                let ordered = *ordered;
                let paragraph = StoryBlock::new(Block::Paragraph(String::new()));
                editor.text.focus = Some((paragraph.uid.clone(), 0, 0));
                if !after.is_empty() {
                    blocks.insert(i + 1, StoryBlock::new(Block::List { ordered, items: after }));
                }
                if matches!(&blocks[i].block, Block::List { items, .. } if items.is_empty()) {
                    blocks[i] = paragraph;
                } else {
                    blocks.insert(i + 1, paragraph);
                }
            }
        }
        Action::ListBack(i, n) => {
            if let Block::List { items, .. } = &mut blocks[i].block {
                if items.len() == 1 {
                    blocks[i].block = Block::Paragraph(String::new());
                    editor.text.focus = Some((blocks[i].uid.clone(), 0, 0));
                } else {
                    items.remove(n);
                    editor.text.focus = Some((blocks[i].uid.clone(), n.saturating_sub(1), if n > 0 { END } else { 0 }));
                }
            }
        }
        Action::Format(mark) => editor.text.pending = Some(mark),
        Action::Step(i, up) => {
            let near: Box<dyn Iterator<Item = &StoryBlock>> = if up { Box::new(blocks[..i].iter().rev()) } else { Box::new(blocks[i + 1..].iter()) };
            if let Some(block) = near.into_iter().find(|block| is_text(&block.block)) {
                let field = match &block.block {
                    Block::List { items, .. } if up => items.len() - 1,
                    Block::Quote { .. } if up => 1,
                    _ => 0,
                };
                editor.text.focus = Some((block.uid.clone(), field, if up { END } else { 0 }));
            }
        }
        Action::Merge(i) => {
            let Some(before) = i.checked_sub(1) else { return };
            let rest = match &blocks[i].block {
                Block::Heading(text) | Block::Subheading(text) | Block::Paragraph(text) | Block::Credits(text) => text.clone(),
                _ => return,
            };
            let uid = blocks[before].uid.clone();
            let (field, joined) = match &mut blocks[before].block {
                Block::Heading(text) | Block::Subheading(text) | Block::Paragraph(text) | Block::Credits(text) => (0, text),
                Block::List { items, .. } => (items.len() - 1, items.last_mut().expect("a list has an item")),
                _ => return,
            };
            let at = joined.chars().count();
            joined.push_str(&rest);
            editor.text.focus = Some((uid, field, at));
            blocks.remove(i);
        }
        Action::Move(from, to) => {
            if from < blocks.len() && to != from && to != from + 1 {
                let block = blocks.remove(from);
                blocks.insert(if to > from { to - 1 } else { to }.min(blocks.len()), block);
            }
        }
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
fn new_blocks() -> [(&'static str, Block); 11] {
    [
        ("Heading", Block::Heading(String::new())),
        ("Subheading", Block::Subheading(String::new())),
        ("Paragraph", Block::Paragraph(String::new())),
        ("List", Block::List { ordered: false, items: vec![String::new()] }),
        ("Numbered list", Block::List { ordered: true, items: vec![String::new()] }),
        ("Quote", Block::Quote { text: String::new(), citation: String::new() }),
        ("Separator", Block::Separator),
        ("Credits", Block::Credits(String::new())),
        ("Route map", Block::Route { caption: String::new(), source: String::new(), route: Vec::new() }),
        ("Practical info", Block::Info { title: String::new(), items: vec![(String::new(), String::new())] }),
        ("Link", Block::Link { url: String::new(), title: String::new(), note: String::new() }),
    ]
}

fn is_text(block: &Block) -> bool {
    matches!(block, Block::Heading(_) | Block::Subheading(_) | Block::Paragraph(_) | Block::Credits(_) | Block::List { .. } | Block::Quote { .. })
}

fn block_label(block: &Block) -> &'static str {
    match block {
        Block::Heading(_) => "Heading",
        Block::Subheading(_) => "Subheading",
        Block::Paragraph(_) => "Paragraph",
        Block::List { ordered: false, .. } => "List",
        Block::List { ordered: true, .. } => "Numbered list",
        Block::Quote { .. } => "Quote",
        Block::Photo(_) => "Photo",
        Block::Gallery(_) => "Gallery",
        Block::Grid { .. } => "Photo grid",
        Block::Separator => "Separator",
        Block::Credits(_) => "Credits",
        Block::Route { .. } => "Route map",
        Block::Info { .. } => "Practical info",
        Block::Link { .. } => "Link",
        Block::Unknown { .. } => "Unknown block",
    }
}

/// The words of the Story's text: a word for each run of letters, and one
/// for each Chinese, Japanese, or Korean character, which have no spaces.
fn story_words(editor: &StoryEditor) -> usize {
    let story = &editor.story;
    let mut texts: Vec<&str> = vec![&story.title, &story.subtitle, &story.introduction, &story.conclusion];
    for block in &editor.blocks {
        match &block.block {
            Block::Heading(text) | Block::Subheading(text) | Block::Paragraph(text) | Block::Credits(text) => texts.push(text),
            Block::List { items, .. } => texts.extend(items.iter().map(String::as_str)),
            Block::Quote { text, citation } => texts.extend([text.as_str(), citation.as_str()]),
            Block::Info { title, items } => texts.extend(std::iter::once(title.as_str()).chain(items.iter().flat_map(|(label, value)| [label.as_str(), value.as_str()]))),
            Block::Link { title, note, .. } => texts.extend([title.as_str(), note.as_str()]),
            _ => {}
        }
    }
    texts.into_iter().map(|text| words(&inline::plain(text))).sum()
}

fn words(text: &str) -> usize {
    let wide = |c: char| matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF);
    let wide_count = text.chars().filter(|&c| wide(c)).count();
    let spaced = text.split(|c: char| c.is_whitespace() || wide(c)).filter(|word| word.chars().any(char::is_alphanumeric)).count();
    wide_count + spaced
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
            introduction: String::new(),
            conclusion: String::new(),
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
            profiles: HashMap::new(),
            job: None,
            reveal: None,
            picking: None,
            active: None,
            hovered: None,
            text: TextCtl::default(),
        }
    }

    fn kinds(editor: &StoryEditor) -> Vec<&str> {
        editor.blocks.iter().map(|block| block.block.kind()).collect()
    }

    fn text(editor: &StoryEditor, i: usize) -> &str {
        match &editor.blocks[i].block {
            Block::Heading(text) | Block::Subheading(text) | Block::Paragraph(text) | Block::Credits(text) => text,
            _ => "",
        }
    }

    #[test]
    fn enter_splits_a_block_at_the_caret_and_backspace_removes_an_empty_one() {
        let mut editor = editor(vec![Block::Heading("Kyōto day".into()), Block::Paragraph("ab".into())]);
        apply(&mut editor, Action::Split(0, 5));
        assert_eq!(kinds(&editor), ["heading", "paragraph", "paragraph"]);
        assert_eq!((text(&editor, 0), text(&editor, 1)), ("Kyōto", " day"), "split by character, not byte");
        assert_eq!(editor.text.focus.as_ref().map(|focus| (focus.0.as_str(), focus.2)), Some((editor.blocks[1].uid.as_str(), 0)));
        apply(&mut editor, Action::Split(2, 2));
        assert_eq!(text(&editor, 3), "");
        apply(&mut editor, Action::RemoveEmpty(3));
        assert_eq!(editor.blocks.len(), 3);
        assert_eq!(editor.text.focus.as_ref().map(|focus| (focus.0.as_str(), focus.2)), Some((editor.blocks[2].uid.as_str(), END)), "the caret goes to the end before");
    }

    #[test]
    fn enter_and_backspace_add_end_and_remove_list_items() {
        let mut editor = editor(vec![Block::List { ordered: false, items: vec!["onetwo".into()] }]);
        apply(&mut editor, Action::ListSplit(0, 0, 3));
        assert_eq!(editor.blocks[0].block, Block::List { ordered: false, items: vec!["one".into(), "two".into()] });
        assert_eq!(editor.text.focus.as_ref().map(|focus| focus.1), Some(1));
        apply(&mut editor, Action::ListSplit(0, 1, 3));
        apply(&mut editor, Action::ListEnd(0, 2));
        assert_eq!(kinds(&editor), ["list", "paragraph"], "Enter on an empty last item ends the list");
        let mut middle = super::tests::editor(vec![Block::List { ordered: true, items: vec!["a".into(), String::new(), "b".into()] }]);
        apply(&mut middle, Action::ListEnd(0, 1));
        assert_eq!(kinds(&middle), ["list", "paragraph", "list"], "Enter on an empty item in the middle puts a paragraph there");
        assert_eq!(middle.text.focus.as_ref().map(|focus| focus.0.as_str()), Some(middle.blocks[1].uid.as_str()));
        assert_eq!(editor.text.focus.as_ref().map(|focus| focus.0.as_str()), Some(editor.blocks[1].uid.as_str()));
        apply(&mut editor, Action::ListBack(0, 1));
        assert_eq!(editor.text.focus.as_ref().map(|focus| (focus.1, focus.2)), Some((0, END)));
        apply(&mut editor, Action::ListBack(0, 0));
        assert_eq!(kinds(&editor), ["paragraph", "paragraph"], "the only item turns back into a paragraph");
        assert!(slash_choices("quote").iter().any(|(_, block)| matches!(block, Block::Quote { .. })));
        assert!(slash_choices("numbered").iter().any(|(_, block)| matches!(block, Block::List { ordered: true, .. })));
    }

    #[test]
    fn words_count_runs_of_letters_and_each_wide_character() {
        assert_eq!(words("A day in Kyōto — 5 photos"), 6);
        assert_eq!(words("京都の一日"), 5);
        assert_eq!(words("Day 1：東京"), 4);
    }

    #[test]
    fn backspace_at_the_start_joins_the_text_before_and_arrows_step_between_blocks() {
        let mut editor = editor(vec![Block::Paragraph("Kyō".into()), Block::Separator, Block::Paragraph("to".into())]);
        apply(&mut editor, Action::Merge(2));
        assert_eq!(kinds(&editor), ["paragraph", "separator", "paragraph"], "a photo or separator before is left alone");
        apply(&mut editor, Action::Step(2, true));
        assert_eq!(editor.text.focus.as_ref().map(|focus| (focus.0.as_str(), focus.2)), Some((editor.blocks[0].uid.as_str(), END)), "the separator is passed over");
        editor.blocks.remove(1);
        apply(&mut editor, Action::Merge(1));
        assert_eq!(text(&editor, 0), "Kyōto");
        assert_eq!(editor.text.focus.as_ref().map(|focus| focus.2), Some(3), "the caret is where they joined");
        let mut list = super::tests::editor(vec![Block::List { ordered: false, items: vec!["a".into(), "b".into()] }, Block::Paragraph("c".into())]);
        apply(&mut list, Action::Merge(1));
        assert_eq!(list.blocks[0].block, Block::List { ordered: false, items: vec!["a".into(), "bc".into()] });
        apply(&mut list, Action::Step(0, false));
        assert!(list.text.focus.is_some());
    }

    #[test]
    fn a_block_moves_to_a_gap_and_slash_finds_blocks_by_name() {
        let mut editor = editor(vec![Block::Heading("A".into()), Block::Separator, Block::Paragraph("C".into())]);
        apply(&mut editor, Action::Move(0, 3));
        assert_eq!(kinds(&editor), ["separator", "paragraph", "heading"]);
        apply(&mut editor, Action::Move(2, 0));
        assert_eq!(kinds(&editor), ["heading", "separator", "paragraph"]);
        apply(&mut editor, Action::Move(1, 2));
        assert_eq!(kinds(&editor), ["heading", "separator", "paragraph"], "a gap next to it changes nothing");
        assert!(matches!(slash_choices("grid 3x2").as_slice(), [(_, Block::Grid { columns: 3, rows: 2, .. })]));
        assert!(matches!(slash_choices("head").first(), Some((_, Block::Heading(_)))));
        apply(&mut editor, Action::Replace(2, Block::Heading(String::new())));
        assert_eq!(kinds(&editor), ["heading", "separator", "heading"]);
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
