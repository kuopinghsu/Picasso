//! The Story editor: a Trip's narrative as an ordered list of blocks, with
//! undo and redo for the session and autosave to the library.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, Margin, Modifiers, Pos2, Rect, RichText, Sense, Vec2};

use crate::i18n::{tr, trf};
use crate::inline::{self, Mark};
use crate::trip::{grid_layout_for, Block, CoverCrop, DayFilter, MediaUse, MemberQuery, Story, StoryBlock, GRID_LAYOUTS};
use crate::story_html;
use crate::types::{Nav, Photo};

use super::{about, theme, Modal, PicassoApp, UiEvent};

/// A pause in typing this long saves.
const SAVE_PAUSE: Duration = Duration::from_secs(1);
/// Changes are saved at the latest this long after the first unsaved one.
const SAVE_WITHIN: Duration = Duration::from_secs(5);
/// Tallest a single photo is shown in the editor.
const PHOTO_MAX_HEIGHT: f32 = 520.0;
/// Height of a toolbar's buttons, on the page and on photos alike.
const TOOL: f32 = 28.0;
/// Rounding of a toolbar's buttons; the bar's is a little more, to wrap them.
const TOOL_RADIUS: u8 = 6;
/// Rounding of what frames part of the page, as of the menus: a chosen
/// block, a Practical info box, and an empty place to fill.
const CORNER: u8 = 8;
/// Height of the tools floating above a block: a toolbar's buttons, its
/// margin, and its outline.
const TOOLS_HEIGHT: f32 = TOOL + 8.0;
/// Where the chosen block was last drawn, kept to tell whether the keys
/// are in it.
const CHOSEN_BLOCK: &str = "story-chosen-block";
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

/// What Escape closed in the Story.
#[derive(Debug, PartialEq)]
enum Closed {
    SlashMenu,
    Framing,
    CoverChoices,
    PhotoStrip,
    Block,
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
    /// Route sources whose elevation is being read.
    reading: HashSet<String>,
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
    /// The cover's frame being chosen, until Done or Cancel.
    framing: Option<Framing>,
    /// The Story's photos are shown to pick a cover from.
    choosing_cover: bool,
    /// Edits asked for between frames, as by Escape, made with the next
    /// frame's own so they are saved and undone like them.
    queued: Vec<Action>,
    /// Quitting without the changes that couldn't be saved was chosen.
    leaving: bool,
}

/// A frame for the cover being chosen: its shape (an index of
/// `CoverCrop::SHAPES`), how tight it is, and its centre on the photo.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Framing {
    shape: usize,
    zoom: f32,
    centre: [f32; 2],
    /// The frame's edges as fractions while its shape is free.
    free: [f32; 4],
}

/// The shape index of a frame of any shape, after `CoverCrop::SHAPES`.
const FREE_SHAPE: usize = CoverCrop::SHAPES.len();

impl Framing {
    /// The frame of `crop` on a photo `w` × `h`, read back as closely as the
    /// offered shapes allow.
    fn of(crop: CoverCrop, w: u32, h: u32) -> Framing {
        let [left, top, right, bottom] = crop.fractions();
        let ratio = (right - left) * w.max(1) as f32 / ((bottom - top) * h.max(1) as f32).max(0.0001);
        let off = |shape: &Option<(u32, u32)>| shape.map_or(f32::MAX, |(sw, sh)| (sw as f32 / sh as f32 - ratio).abs() / ratio);
        let shape = if crop.is_full() {
            0
        } else {
            CoverCrop::SHAPES
                .iter()
                .enumerate()
                .skip(1)
                .min_by(|(_, a), (_, b)| off(a).total_cmp(&off(b)))
                .filter(|(_, shape)| off(shape) < 0.01)
                .map(|(index, _)| index)
                // Then the photo's own shape, made smaller; a frame neither
                // fits, within 1%, is free.
                .or_else(|| (off(&Some((w.max(1), h.max(1)))) < 0.01).then_some(0))
                .unwrap_or(FREE_SHAPE)
        };
        let fixed = CoverCrop::SHAPES.get(shape).copied().flatten();
        let largest = CoverCrop::framed(fixed, w, h, 1.0, [0.5, 0.5]).fractions();
        let zoom = ((right - left) / (largest[2] - largest[0]).max(0.0001)).clamp(0.2, 1.0);
        Framing { shape, zoom, centre: [(left + right) / 2.0, (top + bottom) / 2.0], free: [left, top, right, bottom] }
    }

    fn crop(self, w: u32, h: u32) -> CoverCrop {
        match CoverCrop::SHAPES.get(self.shape) {
            Some(shape) => CoverCrop::framed(*shape, w, h, self.zoom, self.centre),
            None => CoverCrop::from_fractions(self.free),
        }
    }

    /// Takes the fixed shape `self.shape` on from the free frame: centred
    /// where it was and covering about as much of the photo.
    fn leave_free(&mut self, w: u32, h: u32) {
        let [left, top, right, bottom] = self.free;
        let fixed = CoverCrop::SHAPES.get(self.shape).copied().flatten();
        let largest = CoverCrop::framed(fixed, w, h, 1.0, [0.5, 0.5]).fractions();
        let area = |[left, top, right, bottom]: [f32; 4]| (right - left) * (bottom - top);
        self.zoom = (area(self.free) / area(largest).max(0.0001)).sqrt().clamp(0.2, 1.0);
        self.centre = [(left + right) / 2.0, (top + bottom) / 2.0];
    }
}

/// The free frame `free` with the edges marked in `edges` (left, top, right,
/// bottom) moved by `moved`, as fractions of the photo. Each stays on the
/// photo and at least [`CoverCrop::SMALLEST`] from the edge opposite, so a
/// handle stops there rather than pushing or flipping the frame.
fn pull_edges(free: [f32; 4], edges: [bool; 4], moved: Vec2) -> [f32; 4] {
    let [mut left, mut top, mut right, mut bottom] = free;
    let least = CoverCrop::SMALLEST;
    if edges[0] {
        left = (left + moved.x).min(right - least).max(0.0);
    }
    if edges[1] {
        top = (top + moved.y).min(bottom - least).max(0.0);
    }
    if edges[2] {
        right = (right + moved.x).max(left + least).min(1.0);
    }
    if edges[3] {
        bottom = (bottom + moved.y).max(top + least).min(1.0);
    }
    CoverCrop::from_fractions([left, top, right, bottom]).fractions()
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
    /// Only the starred photos.
    starred: bool,
    /// Only photos not in the Story yet, besides this block's own.
    unused: bool,
    /// The Trip's photos on show, as media references.
    choices: Vec<i64>,
}

impl StoryEditor {
    fn snapshot(&self) -> Snapshot {
        Snapshot { story: self.story.clone(), blocks: self.blocks.clone() }
    }

    /// Closes the innermost thing open, for Escape, and says which it was.
    fn escape(&mut self) -> Option<Closed> {
        if let Some(id) = self.text.slash_open.take() {
            self.text.slash_shut = Some(id);
            return Some(Closed::SlashMenu);
        }
        if self.framing.take().is_some() {
            return Some(Closed::Framing);
        }
        if std::mem::take(&mut self.choosing_cover) {
            return Some(Closed::CoverChoices);
        }
        if let Some(picker) = self.picking.take() {
            // A gallery left with no photos shows nothing on the page.
            let empty = self.blocks.iter().position(|block| block.uid == picker.uid && matches!(&block.block, Block::Gallery(items) if items.is_empty()));
            self.queued.extend(empty.map(Action::Remove));
            return Some(Closed::PhotoStrip);
        }
        self.active.take().map(|_| Closed::Block)
    }

    fn restore(&mut self, snapshot: Snapshot) {
        // The revision belongs to the library, not to the edit being undone.
        let revision = self.story.revision;
        // A frame being chosen was for the cover as it was.
        if (snapshot.story.cover, snapshot.story.cover_crop) != (self.story.cover, self.story.cover_crop) {
            self.framing = None;
        }
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
    /// Route blocks, by id, whose elevation the editor hasn't read yet, with
    /// their track files.
    pub(super) unread: Vec<(String, Vec<std::path::PathBuf>)>,
}

/// The elevation along the tracks in `paths`, read from their files.
fn profile_of(paths: &[std::path::PathBuf]) -> Option<crate::elevation::Profile> {
    crate::elevation::profile_of(&paths.iter().map(std::path::PathBuf::as_path).collect::<Vec<_>>())
}

/// Reads the elevation of the `unread` Route blocks into `profiles`, on the
/// job's own thread.
pub(super) fn read_profiles(profiles: &mut HashMap<String, crate::elevation::Profile>, unread: Vec<(String, Vec<std::path::PathBuf>)>) {
    for (uid, paths) in unread {
        if let Some(profile) = profile_of(&paths) {
            profiles.insert(uid, profile);
        }
    }
}

impl PageJob {
    /// Reads what the editor hadn't yet: call on the job's thread.
    pub(super) fn read_profiles(&mut self) {
        read_profiles(&mut self.profiles, std::mem::take(&mut self.unread));
    }

    pub(super) fn page(&self) -> story_html::Page<'_> {
        story_html::Page { story: &self.story, dates: self.dates.as_deref(), blocks: &self.blocks, sources: &self.sources, words: &self.words, map: Some(&self.map), profiles: &self.profiles }
    }
}

/// A Story's last export: where it went, what it was made from, and
/// whether the Story has changed since.
pub(super) struct LastExport {
    folder: std::path::PathBuf,
    content: String,
    stale: bool,
    /// The revision and library state last compared with the export.
    compared: Option<(i64, (u64, i64))>,
    /// The newest comparison asked for; an answer to an older one is dropped.
    asked: u64,
}

impl LastExport {
    /// An export just made from `content`, replacing `earlier`, whose
    /// comparisons still under way are then dropped.
    fn new(folder: std::path::PathBuf, content: String, earlier: Option<&LastExport>) -> Self {
        Self { folder, content, stale: false, compared: None, asked: earlier.map_or(0, |earlier| earlier.asked + 1) }
    }

    /// The number of a comparison to make, if the Story's revision or the
    /// library has changed since the last one.
    fn ask(&mut self, now: (i64, (u64, i64))) -> Option<u64> {
        if self.compared == Some(now) {
            return None;
        }
        self.compared = Some(now);
        self.asked += 1;
        Some(self.asked)
    }

    fn answer(&mut self, asked: u64, content: &str) {
        if asked == self.asked {
            self.stale = content != self.content;
        }
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
    /// Shows this part of the cover.
    CoverCrop(CoverCrop),
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
        ui.painter().rect_filled(bar, 1, theme::accent());
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
            reading: HashSet::new(),
            job: None,
            reveal: None,
            picking: None,
            active: None,
            hovered: None,
            text: TextCtl::default(),
            framing: None,
            choosing_cover: false,
            queued: Vec::new(),
            leaving: false,
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
        save(&self.db, editor)
    }

    /// Saves the open Story once its changes are due, and before Picasso
    /// quits: a Story that can't be saved holds the quit back to ask.
    pub(super) fn autosave_story(&mut self, ctx: &egui::Context) {
        let Some(editor) = &self.story else { return };
        if !editor.leaving && ctx.input(|input| input.viewport().close_requested()) {
            if !self.save_story() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.modal = Modal::UnsavedStory;
            }
            return;
        }
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

    /// Escape in the Story closes the innermost thing open: the "/" menu,
    /// leaving the caret and the words; the cover's framing, as Cancel; the
    /// cover's choices; the photo strip, as Done; and last lets go of the
    /// chosen block and the keys in it. Whether there was anything to close.
    pub(super) fn story_escape(&mut self, ctx: &egui::Context) -> bool {
        match self.story.as_mut().and_then(StoryEditor::escape) {
            None => return false,
            Some(Closed::Block) => {}
            Some(_) => return true,
        }
        let block = ctx.data(|data| data.get_temp::<Rect>(egui::Id::unique(CHOSEN_BLOCK)));
        let focus = ctx.memory(|memory| memory.focused());
        let inside = focus.and_then(|id| ctx.read_response(id)).zip(block).is_some_and(|(field, block)| block.contains_rect(field.rect));
        if let Some(id) = focus.filter(|_| inside) {
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
        true
    }

    pub(super) fn story_ui(&mut self, ui: &mut egui::Ui) {
        self.compare_with_last_export(ui.ctx());
        let Some(editor) = &self.story else { return };
        let before = editor.snapshot();
        let trip_title = self.trip_title(editor.story.trip_id);
        let last_export = self.story_exports.get(&editor.story.id).map(|last| (last.folder.clone(), last.stale));
        let mut open_export = None;
        let mut back = false;
        let mut undo = false;
        let mut redo = false;
        let mut retry = false;
        let mut archive = false;
        let mut delete = false;
        let mut preview = false;
        let mut export = false;
        let mut publish = false;
        // The Story's keys are left alone while a dialog or menu is over it.
        let ctx = ui.ctx().clone();
        let covered = ctx.memory(|memory| memory.top_modal_layer().is_some()) || egui::Popup::is_any_open(&ctx);
        // Undo and redo of the Story, while no text field has the keys.
        if !covered && !ctx.egui_wants_keyboard_input() {
            ui.ctx().input_mut(|input| {
                redo = input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::Z) || input.consume_key(Modifiers::COMMAND, egui::Key::Y);
                undo = !redo && input.consume_key(Modifiers::COMMAND, egui::Key::Z);
            });
        }

        let editor = self.story.as_mut().unwrap();
        ui.horizontal_wrapped(|ui| {
            // The Story's bar has the controls of its toolbars, laid out
            // as a header.
            tool_style(ui, Surface::Page);
            ui.spacing_mut().item_spacing.x = 8.0;
            if tool_button(ui, true, format!("‹  {trip_title}")).on_hover_text(tr("Back to the trip's photos")).clicked() {
                back = true;
            }
            // Where the Story stands: a draft until it is marked finished.
            let finished = editor.story.finished;
            let (label, ink, fill) = if finished {
                (format!("✓ {}", tr("Finished")), theme::accent(), theme::select())
            } else {
                (tr("Draft").to_string(), theme::muted(), theme::well())
            };
            let pill = egui::Button::new(RichText::new(label).size(12.0).color(ink)).fill(fill).small().min_size(Vec2::new(0.0, 20.0)).corner_radius(10);
            let hint = if finished { tr("Finished. Click to make it a draft again.") } else { tr("A draft until you mark it finished. Click when it is done.") };
            if ui.add(pill).on_hover_text(hint).clicked() {
                editor.story.finished = !finished;
            }
            // Saved within seconds of a pause, so a change waiting for it is
            // already on its way.
            let (status, color) = match (&editor.error, editor.unsaved) {
                (Some(err), _) => (trf("Couldn't save: {}", &[err]), theme::DANGER),
                (None, Some(_)) => (tr("Saving…").to_string(), theme::muted()),
                (None, None) => (tr("Saved").to_string(), theme::muted()),
            };
            ui.label(RichText::new(status).size(13.0).color(color));
            let words = story_words(editor);
            let minutes = words.div_ceil(WORDS_A_MINUTE).max(1);
            ui.label(RichText::new(trf("{} words · {} min read", &[&words, &minutes])).size(13.0).color(theme::muted()));
            if let Some((folder, stale)) = &last_export {
                let (label, hint) = if *stale {
                    (tr("Changed since the export"), trf("The export in {} shows the story as it was. Click to export it again.", &[&folder.display()]))
                } else {
                    (tr("Exported"), trf("The export in {} shows the story as it is now. Click to open its folder.", &[&folder.display()]))
                };
                let link = egui::Button::new(RichText::new(label).size(13.0).color(theme::muted())).frame(false);
                if ui.add(link).on_hover_text(hint).clicked() {
                    if *stale {
                        export = true;
                    } else {
                        open_export = Some(folder.clone());
                    }
                }
            }
            if editor.error.is_some() && tool_button(ui, true, tr("Try again")).clicked() {
                retry = true;
            }
            // Editing, then the Story's own settings and menu, then what
            // takes it out of Picasso, the main action last.
            theme::end_actions(ui, "story-bar", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let undo_hint = format!("{} ({})", tr("Undo"), keys("Ctrl+Z"));
                if tool_icon(ui, !editor.undo.is_empty(), &undo_hint, |painter, at, ink| theme::paint_icon(painter, theme::Icon::Undo, at, ink)).clicked() {
                    undo = true;
                }
                let redo_hint = format!("{} ({})", tr("Redo"), keys("Ctrl+Shift+Z"));
                if tool_icon(ui, !editor.redo.is_empty(), &redo_hint, |painter, at, ink| theme::paint_icon(painter, theme::Icon::Redo, at, ink)).clicked() {
                    redo = true;
                }
                tool_divider(ui);
                story_settings(ui, &mut editor.story);
                let more = tool_icon(ui, true, tr("More"), |painter, at, ink| theme::paint_icon(painter, theme::Icon::More, at, ink));
                egui::Popup::menu(&more).show(|ui| {
                    let label = if editor.story.archived { tr("Restore story") } else { tr("Archive story") };
                    if ui.button(label).clicked() {
                        archive = true;
                    }
                    ui.separator();
                    if ui.button(tr("Delete story…")).clicked() {
                        delete = true;
                    }
                });
                tool_divider(ui);
                if let Some(job) = &editor.job {
                    ui.add(egui::Spinner::new().size(13.0));
                    ui.add_space(4.0);
                    let (done, total) = (job.done.load(Ordering::Relaxed), job.total.load(Ordering::Relaxed));
                    ui.label(RichText::new(trf("Rendering photos: {} of {}", &[&done, &total])).size(13.0).color(theme::muted()));
                    if tool_button(ui, true, tr("Cancel")).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                } else {
                    let button = tool_button(ui, true, tr("Preview"));
                    if button.on_hover_text(tr("Open the story in your web browser, as it will look when exported")).clicked() {
                        preview = true;
                    }
                    ui.add_space(4.0);
                    let share = ui.add(theme::primary_button(&format!("{}  ▾", tr("Publish"))).corner_radius(TOOL_RADIUS));
                    egui::Popup::menu(&share).show(|ui| {
                        if ui.button(tr("Export…")).on_hover_text(tr("Save the story as a web page or Markdown, with its photos")).clicked() {
                            export = true;
                        }
                        if ui.button(tr("Publish to WordPress…")).clicked() {
                            publish = true;
                        }
                    });
                }
            });
        });
        ui.add_space(6.0);

        let mut actions = self.story.as_mut().map_or_else(Vec::new, |editor| {
            editor.text.slash_open = None;
            std::mem::take(&mut editor.queued)
        });
        let mut typed: Option<(String, u8)> = None;
        // Moving and copying the chosen block, keys and all, before its
        // field would take them as a selection.
        let chosen = self.story.as_ref().and_then(|editor| editor.blocks.iter().position(|block| editor.active.as_deref() == Some(block.uid.as_str())));
        let field = ctx.memory(|memory| memory.focused()).and_then(|id| ctx.read_response(id)).map(|field| field.rect);
        let block = ctx.data(|data| data.get_temp::<Rect>(egui::Id::unique(CHOSEN_BLOCK)));
        let typing_elsewhere = ctx.egui_wants_keyboard_input() && !field.zip(block).is_some_and(|(field, block)| block.contains_rect(field));
        if let Some(index) = chosen.filter(|_| !typing_elsewhere && !covered) {
            ui.ctx().input_mut(|input| {
                if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::ArrowUp) {
                    actions.push(Action::Up(index));
                } else if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::ArrowDown) {
                    actions.push(Action::Down(index));
                } else if input.consume_key(Modifiers::COMMAND, egui::Key::D) {
                    actions.push(Action::Duplicate(index));
                }
            });
        }
        let mut add_photos = false;
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
                // What to begin with, rather than an empty page.
                egui::Frame::new().inner_margin(Margin::symmetric(BLOCK_INSET, 12)).show(ui, |ui| {
                    ui.label(RichText::new(tr("This story is empty. Begin with:")).color(theme::muted()));
                    ui.add_space(4.0);
                    // The rows blocks are offered in everywhere else.
                    ui.scope(|ui| {
                        ui.set_max_width(360.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let new = |name: &str| new_blocks().into_iter().find(|(english, _)| *english == name).map(|(_, block)| block);
                        let starts = [
                            (new("Paragraph"), tr("Start writing"), tr("Plain text")),
                            (Some(Block::Gallery(Vec::new())), tr("Add photos"), tr("Pick photos of the trip for a gallery")),
                            (new("Route map"), tr("Add a route map"), tr("The trip's tracks on a map, with their height")),
                        ];
                        for (block, label, hint) in starts.into_iter().filter_map(|(block, label, hint)| Some((block?, label, hint))) {
                            if block_row(ui, &block, label, Some(hint), None).clicked() {
                                add_photos |= matches!(block, Block::Gallery(_));
                                actions.push(Action::Insert(0, block));
                            }
                        }
                    });
                    ui.add_space(4.0);
                    let hint = tr("Or select photos in the trip and choose Add to Story.");
                    ui.label(RichText::new(hint).size(12.5).color(theme::muted()));
                });
            }
            for index in 0..count {
                inserter(ui, index, &mut actions);
                self.story_block(ui, index, count, &mut actions, &mut typed);
            }
            inserter(ui, count, &mut actions);
            // Like the end of a page: a click starts a new paragraph there.
            // An empty story offers its own beginnings above instead.
            let start = egui::Button::new(RichText::new(tr("Start writing, or type / to choose a block")).size(15.0).color(theme::muted())).frame(false);
            if count > 0 && ui.add(start).clicked() {
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
        if let Some(gallery) = editor.blocks.first().filter(|_| add_photos) {
            let choices = trip_photos(&self.db, editor.story.trip_id, DayFilter::Any, false);
            editor.picking = Some(Picker { uid: gallery.uid.clone(), day: DayFilter::Any, starred: false, unused: false, choices });
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
        if let Some(folder) = open_export {
            let _ = open::that(&folder);
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
        let Some(mut job) = self.story_job() else { return };
        let progress = self.start_story_job();
        let folder = self.cache_dir.join("story-preview").join(job.story.id.to_string());
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-preview".into()).spawn(move || {
            job.read_profiles();
            // The folder is the preview's own; an earlier preview is replaced.
            let _ = std::fs::remove_dir_all(&folder);
            let outcome = story_html::write_story(&folder, story_html::Format::Html, story_html::Profile::default(), &job.page(), &progress).and_then(|(index, outcome)| {
                open::that(&index).map_err(|err| err.to_string())?;
                Ok(outcome)
            });
            let _ = events.send(UiEvent::StoryPreviewed(progress, outcome));
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

    /// Marks `job` as over, if the open Story is still showing it: a job left
    /// running when its Story was closed never ends one started since.
    pub(super) fn end_story_job(&mut self, job: &Arc<story_html::Progress>) {
        if let Some(editor) = self.story.as_mut().filter(|editor| editor.job.as_ref().is_some_and(|shown| Arc::ptr_eq(shown, job))) {
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
        let (mut profiles, mut unread) = (HashMap::new(), Vec::new());
        for block in &blocks {
            let Block::Route { source, route, .. } = &block.block else { continue };
            if route.is_empty() {
                continue;
            }
            match self.story.as_ref()?.profiles.get(source) {
                Some(Some(profile)) => {
                    profiles.insert(block.uid.clone(), profile.clone());
                }
                Some(None) => {}
                None => unread.push((block.uid.clone(), self.route_paths(story.trip_id, source))),
            }
        }
        Some(PageJob { story, blocks, sources, dates, words, map, profiles, unread })
    }

    /// The track files a route drawn from `source` comes from: the one
    /// track's path, or for "" all the Trip's tracks.
    fn route_paths(&self, trip_id: i64, source: &str) -> Vec<std::path::PathBuf> {
        if source.is_empty() {
            self.db.trip_tracks(trip_id).into_iter().filter(|link| link.track_id.is_some()).map(|link| link.path.into()).collect()
        } else {
            vec![source.into()]
        }
    }

    /// The elevation of a route drawn from `source`, read once while the
    /// Story is open, on another thread: `None` until it has been read.
    fn route_profile(&mut self, source: &str, ctx: &egui::Context) -> Option<crate::elevation::Profile> {
        let editor = self.story.as_ref()?;
        if let Some(known) = editor.profiles.get(source) {
            return known.clone();
        }
        if editor.reading.contains(source) {
            return None;
        }
        let (story_id, paths) = (editor.story.id, self.route_paths(editor.story.trip_id, source));
        self.story.as_mut()?.reading.insert(source.to_string());
        let (source, events, ctx) = (source.to_string(), self.events_tx.clone(), ctx.clone());
        let _ = std::thread::Builder::new().name("picasso-elevation".into()).spawn(move || {
            let profile = profile_of(&paths);
            let _ = events.send(UiEvent::RouteProfile(story_id, source, profile));
            ctx.request_repaint();
        });
        None
    }

    pub(super) fn on_route_profile(&mut self, story_id: i64, source: String, profile: Option<crate::elevation::Profile>) {
        if let Some(editor) = self.story.as_mut().filter(|editor| editor.story.id == story_id) {
            editor.reading.remove(&source);
            editor.profiles.insert(source, profile);
        }
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
                    ui.label(RichText::new(tr("The story has no cover; it starts with its title. Add one with Add cover… above the title.")).color(theme::muted()));
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
        let Some(mut job) = self.story_job() else { return };
        let progress = self.start_story_job();
        let story_id = job.story.id;
        let name = story_name(&job.story);
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-story-export".into()).spawn(move || {
            job.read_profiles();
            let content = story_html::content_key(&job.page());
            let outcome = story_html::export_story(&parent, &name, format, profile, &job.page(), &progress)
                .inspect(|(folder, _)| {
                    let _ = open::that(folder);
                });
            let _ = events.send(UiEvent::StoryExported(story_id, progress, content, outcome));
            ctx.request_repaint();
        });
    }

    pub(super) fn on_story_exported(&mut self, story_id: i64, job: &Arc<story_html::Progress>, content: String, outcome: Result<(std::path::PathBuf, story_html::Outcome), String>) {
        self.end_story_job(job);
        if let Ok((folder, _)) = &outcome {
            let last = LastExport::new(folder.clone(), content, self.story_exports.get(&story_id));
            self.story_exports.insert(story_id, last);
        }
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

    /// Compares the open Story with its last export after the Story is saved
    /// or the library changes, reading the photos' files on another thread.
    fn compare_with_last_export(&mut self, ctx: &egui::Context) {
        let Some(editor) = self.story.as_ref().filter(|editor| editor.unsaved.is_none() && editor.job.is_none()) else { return };
        let story_id = editor.story.id;
        let revision = editor.story.revision;
        let now = (revision, self.db.change_stamp());
        let Some(asked) = self.story_exports.get_mut(&story_id).and_then(|last| last.ask(now)) else { return };
        let Some(job) = self.story_job() else { return };
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-story-compare".into()).spawn(move || {
            let _ = events.send(UiEvent::StoryContent(story_id, asked, story_html::content_key(&job.page())));
            ctx.request_repaint();
        });
    }

    pub(super) fn on_story_content(&mut self, story_id: i64, asked: u64, content: String) {
        if let Some(last) = self.story_exports.get_mut(&story_id) {
            last.answer(asked, &content);
        }
    }

    pub(super) fn on_story_previewed(&mut self, job: &Arc<story_html::Progress>, outcome: Result<story_html::Outcome, String>) {
        self.end_story_job(job);
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
            Some(media_id) if self.story.as_ref().is_some_and(|editor| editor.framing.is_some()) => self.frame_cover(ui, media_id, width, actions),
            Some(media_id) => {
                let crop = self.story.as_ref().map_or(CoverCrop::FULL, |editor| editor.story.cover_crop);
                let height = match (crop.is_full(), self.cover_size(media_id)) {
                    (false, Some((w, h))) => {
                        let [left, top, right, bottom] = crop.fractions();
                        width * ((bottom - top) * h as f32) / ((right - left) * w as f32).max(1.0)
                    }
                    _ => width * 0.42,
                };
                let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height.clamp(60.0, COVER_MAX_HEIGHT.max(width * 0.6))), Sense::click());
                if crop.is_full() {
                    self.paint_media(ui, media_id, rect, true);
                } else {
                    self.paint_cropped(ui, media_id, rect, crop);
                }
                photo_access(ui, &response, rect, tr("Cover"));
                if response.has_focus() && pressed_outside(ui, rect) {
                    response.surrender_focus();
                }
                // Its tools sit on the photo's bottom-right corner while it is
                // pointed at or the keyboard is on it or them, as a cover's do
                // on a page.
                let tools = egui::Id::unique("cover-tools");
                let pointer = ui.input(|input| input.pointer.hover_pos());
                let over_tools = ui.ctx().memory(|memory| memory.area_rect(tools)).zip(pointer).is_some_and(|(tools, at)| tools.contains(at));
                let tools_focused = ui.ctx().data(|data| data.get_temp::<bool>(tools)).unwrap_or(false);
                let mut frame = false;
                let mut focused = false;
                // Areas aren't clipped, so the bar goes on the part in view.
                let seen = rect.intersect(ui.clip_rect());
                if (ui.rect_contains_pointer(rect) || over_tools || response.has_focus() || tools_focused) && seen.height() >= TOOLS_HEIGHT + 20.0 {
                    let shape = if crop.is_full() { None } else { self.cover_size(media_id).map(|(w, h)| frame_name(crop, w, h)) };
                    let width = ui.ctx().memory(|memory| memory.area_rect(tools)).map_or(260.0, |area| area.width());
                    let at = seen.right_bottom() - Vec2::new(width + 10.0, TOOLS_HEIGHT + 10.0);
                    egui::Area::new(tools).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
                        tool_bar(ui, Surface::Photo, |ui| {
                            if let Some(shape) = &shape {
                                ui.add_space(6.0);
                                ui.label(RichText::new(shape).size(12.0).color(egui::Color32::from_white_alpha(200)));
                                tool_divider(ui);
                            }
                            let framer = tool_button(ui, true, format!("⛶  {}", tr("Frame cover…"))).on_hover_text(tr("Choose the part of the photo the cover shows, in this story only"));
                            frame = framer.clicked();
                            let remove = tool_button(ui, true, tr("Remove cover")).on_hover_text(tr("The photo stays in the story and the trip"));
                            if remove.clicked() {
                                actions.push(Action::Cover(None));
                            }
                            focused = framer.has_focus() || remove.has_focus();
                        });
                    });
                }
                ui.ctx().data_mut(|data| data.insert_temp(tools, focused));
                if frame {
                    self.start_framing(media_id);
                }
                ui.add_space(16.0);
            }
            None => self.cover_placeholder(ui, width, actions),
        }
        let trip_id = self.story.as_ref().map(|editor| editor.story.trip_id);
        let trip_dates = self.trips.iter().find(|trip| Some(trip.id) == trip_id).and_then(super::trips::trip_dates);
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
        // The byline as the page has it: who wrote it, and when the trip was.
        ui.horizontal(|ui| {
            let author = egui::TextEdit::singleline(&mut story.author).hint_text(tr("Author")).frame(egui::Frame::NONE).desired_width(180.0);
            if ui.add(author).changed() {
                *typed = Some((String::new(), 2));
            }
            if let Some(dates) = trip_dates.filter(|_| story.show_date) {
                ui.label(RichText::new(format!("·  {dates}")).color(theme::muted()));
            }
        });
        ui.add_space(8.0);
        ui.separator();
    }

    /// Where the cover goes while there is none: Add cover…, which shows the
    /// Story's photos to click one, or what to do first when it has none.
    fn cover_placeholder(&mut self, ui: &mut egui::Ui, width: f32, actions: &mut Vec<Action>) {
        let Some(editor) = &self.story else { return };
        // Each photo once, named as it is where it was first used.
        let mut photos: Vec<(i64, String)> = Vec::new();
        for item in editor.blocks.iter().flat_map(|block| block.block.media()) {
            if !photos.iter().any(|(media_id, _)| *media_id == item.media_id) {
                photos.push((item.media_id, photo_name(&item.alt, photos.len())));
            }
        }
        let open = editor.choosing_cover && !photos.is_empty();
        // With no photos to choose from it only says so, and takes no clicks or keys.
        let sense = if photos.is_empty() { Sense::hover() } else { Sense::click() };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 56.0), sense);
        let lit = !photos.is_empty() && (response.hovered() || response.has_focus() || open);
        let text = if photos.is_empty() {
            tr("No cover yet. Add photos to the story to choose one from them.").to_string()
        } else {
            format!("+  {}", tr("Add cover…"))
        };
        empty_slot(ui, rect, lit, &text, 14.0);
        if !photos.is_empty() {
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Add cover…")));
            theme::focus_ring(ui, &response, rect);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Choose one of the story's photos for the top of the story")).clicked() {
                if let Some(editor) = &mut self.story {
                    editor.choosing_cover = !open;
                }
            }
        }
        if open {
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Click a photo to make it the cover.")).size(12.0).color(theme::muted()));
            egui::ScrollArea::horizontal().id_salt("cover-choices").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (media_id, name) in photos {
                        let (tile, choice) = ui.allocate_exact_size(Vec2::splat(PICK_TILE), Sense::click());
                        self.paint_media(ui, media_id, tile, true);
                        choice.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                        theme::focus_ring(ui, &choice, tile);
                        if choice.hovered() {
                            ui.painter().rect_stroke(tile, 4, egui::Stroke::new(3.0, theme::ACCENT), egui::StrokeKind::Inside);
                        }
                        if choice.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            actions.push(Action::Cover(Some(media_id)));
                        }
                    }
                });
            });
        }
        ui.add_space(10.0);
    }

    /// The optional Introduction before the blocks or Conclusion after
    /// them; left empty, the export leaves it out.
    fn story_section(&mut self, ui: &mut egui::Ui, section: Section, typed: &mut Option<(String, u8)>) {
        let Some(StoryEditor { story, text: ctl, .. }) = &mut self.story else { return };
        let (label, hint, text, field) = match section {
            Section::Introduction => (tr("Introduction"), tr("How the story begins (optional)"), &mut story.introduction, 3),
            Section::Conclusion => (tr("Conclusion"), tr("How the story ends (optional)"), &mut story.conclusion, 4),
        };
        // Empty, its hint says what it is: the name only shows once it has
        // words or the keys, its room kept so the page doesn't jump then.
        let id = field_id("story", usize::from(field));
        let named = !text.is_empty() || ui.memory(|memory| memory.has_focus(id));
        ui.label(RichText::new(label).size(13.0).color(if named { theme::muted() } else { egui::Color32::TRANSPARENT }));
        if styled_field(ui, text, id, 16.0, true, hint, ctl, None).0.changed() {
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
            egui::Stroke::new(1.0, theme::accent())
        } else if editor.hovered.as_deref() == Some(uid.as_str()) {
            egui::Stroke::new(1.0, theme::muted().gamma_multiply(0.4))
        } else {
            egui::Stroke::NONE
        };
        let frame = egui::Frame::new().stroke(stroke).corner_radius(CORNER).inner_margin(Margin::symmetric(BLOCK_INSET, 6));
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
        // The keys arriving in it, by Tab or a new block, choose it too.
        let focus = ui.ctx().memory(|memory| memory.focused()).filter(|&id| ui.ctx().read_response(id).is_some_and(|field| rect.contains_rect(field.rect)));
        let Some(editor) = &mut self.story else { return };
        if pressed_outside(ui, rect) && !over_tools && editor.active.as_deref() == Some(uid.as_str()) {
            // A click elsewhere on the page lets go of the block, and of
            // the keys too, or they would choose it again.
            editor.active = None;
            if let Some(id) = focus {
                ui.ctx().memory_mut(|memory| memory.surrender_focus(id));
            }
        } else if focus.is_some() && !active {
            editor.active = Some(uid.clone());
        }
        if pointed {
            editor.hovered = Some(uid.clone());
            if ui.input(|input| input.pointer.primary_pressed()) {
                editor.active = Some(uid.clone());
            }
        } else if editor.hovered.as_deref() == Some(uid.as_str()) {
            editor.hovered = None;
        }
        let active = editor.active.as_deref() == Some(uid.as_str());
        if active {
            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::unique(CHOSEN_BLOCK), rect));
        }
        // Areas aren't clipped, so the tools of a block scrolled out of
        // view would float over the rest of the page or past its end.
        if (active || pointed) && rect.intersects(ui.clip_rect()) {
            let top = (rect.top() - TOOLS_HEIGHT).max(ui.clip_rect().top());
            egui::Area::new(tools)
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(rect.left(), top))
                .show(ui.ctx(), |ui| {
                    tool_bar(ui, Surface::Page, |ui| self.block_tools(ui, index, count, &uid, active, actions));
                });
        }
        if let Some(editor) = self.story.as_mut().filter(|editor| editor.reveal == Some(index)) {
            editor.reveal = None;
            ui.scroll_to_rect(rect, Some(Align::Center));
        }
    }

    /// The tools of the block pointed at or chosen: a handle to drag it by,
    /// its kind and layout, moving it, and the rest in ⋯. Text styles only
    /// show on the block being written in.
    fn block_tools(&mut self, ui: &mut egui::Ui, index: usize, count: usize, uid: &str, active: bool, actions: &mut Vec<Action>) {
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
        let handle = drag_handle(ui);
        if handle.drag_started() || handle.dragged() {
            handle.dnd_set_drag_payload(MoveBlock(index));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if handle.on_hover_text(tr("Drag to move this block")).hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        ui.label(RichText::new(tr(label)).size(12.0).color(theme::muted()));
        ui.add_space(4.0);
        // Groups: what the block is; how it looks; where it is; the rest.
        let styles = active && is_text(block);
        if photos || styles {
            tool_divider(ui);
        }
        if photos {
            layout_menu(ui, index, uid, media, grid, gallery, actions);
        }
        if styles {
            text_tools(ui, index, block, actions);
        }
        tool_divider(ui);
        let up = format!("{} ({})", tr("Move up"), keys("Ctrl+Shift+↑"));
        if theme::named(tool_button(ui, index > 0, "↑"), &up).clicked() {
            actions.push(Action::Up(index));
        }
        let down = format!("{} ({})", tr("Move down"), keys("Ctrl+Shift+↓"));
        if theme::named(tool_button(ui, index + 1 < count, "↓"), &down).clicked() {
            actions.push(Action::Down(index));
        }
        tool_divider(ui);
        let more = tool_icon(ui, true, tr("More"), |painter, at, ink| theme::paint_icon(painter, theme::Icon::More, at, ink));
        egui::Popup::menu(&more).show(|ui| {
            if ui.add(egui::Button::new(tr("Duplicate")).shortcut_text(keys("Ctrl+D"))).clicked() {
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
        let cover = self.story.as_ref().and_then(|editor| editor.story.cover);
        let mut pick = false;
        if photo {
            let aspect = self.media_width(media[0], 1.0);
            let height = (width / aspect).min(PHOTO_MAX_HEIGHT);
            let (rect, response) = ui.allocate_exact_size(Vec2::new((height * aspect).min(width), height), Sense::click_and_drag());
            self.paint_media(ui, media[0], rect, false);
            if cover == Some(media[0]) {
                cover_chip(ui, rect);
            }
            photo_access(ui, &response, rect, &names[0]);
            drag_source(ui, &response, Drag::Placed(index, 0));
            drop_target(ui, &response, index, 0, actions);
            photo_tools(ui, &response, rect, media[0], cover, None, actions);
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
                            empty_slot(ui, rect, response.hovered() || response.has_focus(), "+", 20.0);
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Add photos…")));
                            theme::focus_ring(ui, &response, rect);
                            drop_target(ui, &response, index, media.len(), actions);
                            if response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Add photos from the trip, or drop one here")).clicked() {
                                pick = true;
                            }
                            continue;
                        };
                        self.paint_media(ui, media_id, rect, true);
                        if cover == Some(media_id) {
                            cover_chip(ui, rect);
                        }
                        photo_access(ui, &response, rect, &names[n]);
                        drag_source(ui, &response, Drag::Placed(index, n));
                        drop_target(ui, &response, index, n, actions);
                        photo_tools(ui, &response, rect, media_id, cover, Some((index, n, "Remove from grid")), actions);
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
                    if cover == Some(media_id) {
                        cover_chip(ui, rect);
                    }
                    photo_access(ui, &response, rect, &names[n]);
                    drag_source(ui, &response, Drag::Placed(index, n));
                    drop_target(ui, &response, index, n, actions);
                    photo_tools(ui, &response, rect, media_id, cover, Some((index, n, "Remove from gallery")), actions);
                }
            });
        }
        if gallery || grid.is_some() {
            self.photo_picker(ui, index, uid, &media, pick, actions);
        }
        let mut chosen: Option<(String, Vec<Vec<[i32; 2]>>)> = None;
        if let Some(Block::Route { route, source, .. }) = self.story.as_ref().map(|editor| &editor.blocks[index].block) {
            let (route, source) = (route.clone(), source.clone());
            let profile = self.route_profile(&source, ui.ctx());
            self.route_drawing(ui, &route, profile.as_deref(), width);
            let open = self.story.as_ref().is_some_and(|editor| editor.active.as_deref() == Some(uid));
            if open || route.is_empty() {
                chosen = self.route_chooser(ui, uid, &source, route.is_empty());
            }
        }
        // Only the Trip's links allowed to be published are offered.
        let link_trip = self.story.as_ref().filter(|editor| matches!(editor.blocks[index].block, Block::Link { .. })).map(|editor| editor.story.trip_id);
        let shareable: Vec<crate::trip::TripResource> = match link_trip {
            Some(trip_id) => self.resources_of(trip_id).into_iter().filter(|resource| resource.kind == crate::trip::ResourceKind::Link && resource.publish).collect(),
            None => Vec::new(),
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
        // Until a track is chosen, a low empty place rather than a blank map.
        let height = if route.iter().all(Vec::is_empty) { 96.0 } else { (width * 0.55).min(400.0) };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let Some((view, shape)) = crate::trip::route_shape(route, rect.width(), rect.height(), 28.0) else {
            empty_slot(ui, rect, false, tr("No route yet"), 14.0);
            return;
        };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6, theme::placeholder());
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
        let links: Vec<_> = self.tracks_of(trip_id).into_iter().filter(|link| link.track_id.is_some()).collect();
        if links.is_empty() {
            ui.label(RichText::new(tr("Link a GPS track to the trip first, in its Tracks row.")).size(12.0).color(theme::muted()));
            return None;
        }
        let days = self.days_of(trip_id);
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
        // A toolbar under the map, as the photo strip's filters are.
        ui.add_space(4.0);
        tool_bar(ui, Surface::Page, |ui| {
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
        let (mut starred, mut unused) = editor.picking.as_ref().filter(|_| open).map_or((false, false), |picker| (picker.starred, picker.unused));
        let mut toggle = start && !open;
        if open {
            // The strip's filters as a toolbar: which Day, then narrowing
            // chips, then Done.
            let days = self.days_of(trip_id);
            let shown = match day {
                DayFilter::Day(id) => days.iter().find(|d| d.id == id).map(super::trips::day_name).unwrap_or_default(),
                _ => tr("All days").to_string(),
            };
            tool_bar(ui, Surface::Page, |ui| {
                egui::ComboBox::from_id_salt(("pick-day", uid)).selected_text(shown).height(320.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut day, DayFilter::Any, tr("All days"));
                    for d in &days {
                        ui.selectable_value(&mut day, DayFilter::Day(d.id), super::trips::day_name(d));
                    }
                });
                tool_divider(ui);
                ui.toggle_value(&mut starred, format!("★  {}", tr("Starred only"))).on_hover_text(tr("Your stars are only read, never changed"));
                ui.toggle_value(&mut unused, tr("Not in the story yet")).on_hover_text(tr("Hide photos already used elsewhere in this story"));
                tool_divider(ui);
                if ui.add(theme::primary_button(tr("Done")).corner_radius(TOOL_RADIUS)).clicked() {
                    toggle = true;
                }
            });
            ui.label(RichText::new(tr("Click a photo to add it, again to take it out.")).size(12.0).color(theme::muted()));
        } else if page_button(ui, tr("Add photos…"), true).on_hover_text(tr("Pick photos of the trip for this block")).clicked() {
            toggle = true;
        }
        let Some(editor) = &mut self.story else { return };
        if toggle {
            // A gallery left with no photos shows nothing on the page.
            if open && media.is_empty() && matches!(editor.blocks[index].block, Block::Gallery(_)) {
                actions.push(Action::Remove(index));
            }
            editor.picking = if open { None } else { Some(Picker { uid: uid.into(), day: DayFilter::Any, starred: false, unused: false, choices: Vec::new() }) };
            if let Some(picker) = &mut editor.picking {
                picker.choices = trip_photos(&self.db, trip_id, picker.day, false);
            }
            return;
        }
        let Some(picker) = editor.picking.as_mut().filter(|picker| picker.uid == uid) else { return };
        if (picker.day, picker.starred) != (day, starred) {
            (picker.day, picker.starred) = (day, starred);
            picker.choices = trip_photos(&self.db, trip_id, day, starred);
        }
        picker.unused = unused;
        // Photos used in other blocks, to leave out or to mark.
        let elsewhere: std::collections::HashSet<i64> =
            editor.blocks.iter().filter(|block| block.uid != uid).flat_map(|block| block.block.media().iter().map(|item| item.media_id)).collect();
        let choices: Vec<i64> = picker.choices.iter().copied().filter(|id| !unused || !elsewhere.contains(id) || media.contains(id)).collect();
        if choices.is_empty() {
            ui.label(RichText::new(empty_strip(picker.day, picker.starred, !picker.choices.is_empty())).color(theme::muted()));
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
                    } else if elsewhere.contains(&media_id) {
                        // Already in the story: a quiet dot, so it isn't added twice by mistake.
                        ui.painter().circle_filled(rect.left_bottom() + Vec2::new(9.0, -9.0), 5.0, theme::muted());
                    }
                    let hint = if chosen {
                        tr("In this block. Click to take it out.")
                    } else if elsewhere.contains(&media_id) {
                        tr("Already elsewhere in this story. Click to add it here too.")
                    } else {
                        tr("Click to add")
                    };
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

    /// The cover photo's size as shown, turned as the catalog turns it, from
    /// its thumbnail; `None` until the thumbnail is ready.
    fn cover_size(&mut self, media_id: i64) -> Option<(u32, u32)> {
        let photo = self.story_photo(media_id)?;
        self.ensure_thumb(photo.id, &photo.path);
        let (_, w, h) = self.thumb_texture(photo.id)?;
        let (w, h) = (w as u32, h as u32);
        Some(if photo.rotation.rem_euclid(180) == 90 { (h, w) } else { (w, h) })
    }

    fn start_framing(&mut self, media_id: i64) {
        let size = self.cover_size(media_id).unwrap_or((3, 2));
        if let Some(editor) = &mut self.story {
            editor.framing = Some(Framing::of(editor.story.cover_crop, size.0, size.1));
        }
    }

    /// Draws the part `crop` of a Story photo, as shown, in `rect`.
    fn paint_cropped(&mut self, ui: &egui::Ui, media_id: i64, rect: Rect, crop: CoverCrop) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4, theme::placeholder());
        let Some(photo) = self.story_photo(media_id) else { return };
        self.ensure_thumb(photo.id, &photo.path);
        if let Some((texture, _, _)) = self.thumb_texture(photo.id) {
            theme::paint_image_rounded(&painter, &texture, rect, texture_uv(crop.fractions(), photo.rotation), photo.rotation, 4.0);
        }
    }

    /// The whole cover with the frame over it, to drag, and its shape, how
    /// tight it is, Reset, Cancel, and Done.
    fn frame_cover(&mut self, ui: &mut egui::Ui, media_id: i64, width: f32, actions: &mut Vec<Action>) {
        let Some((w, h)) = self.cover_size(media_id) else {
            ui.spinner();
            return;
        };
        let Some(mut framing) = self.story.as_ref().and_then(|editor| editor.framing) else { return };
        let height = (width * h as f32 / w.max(1) as f32).min(COVER_MAX_HEIGHT * 1.6);
        let shown = Vec2::new(height * w as f32 / h.max(1) as f32, height);
        let (row, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let rect = Rect::from_center_size(row.center(), shown);
        self.paint_cropped(ui, media_id, rect, CoverCrop::FULL);
        let [left, top, right, bottom] = framing.crop(w, h).fractions();
        let frame = Rect::from_min_max(rect.lerp_inside(Vec2::new(left, top)), rect.lerp_inside(Vec2::new(right, bottom)));
        let response = ui.interact(frame, ui.scope_id().with("cover-frame"), Sense::drag()).on_hover_cursor(egui::CursorIcon::Grab);
        if response.dragged() {
            let moved = response.drag_delta() / rect.size();
            if framing.shape == FREE_SHAPE {
                let [left, top, right, bottom] = framing.free;
                let dx = moved.x.clamp(-left, 1.0 - right);
                let dy = moved.y.clamp(-top, 1.0 - bottom);
                framing.free = [left + dx, top + dy, right + dx, bottom + dy];
            } else {
                framing.centre = [(framing.centre[0] + moved.x).clamp(0.0, 1.0), (framing.centre[1] + moved.y).clamp(0.0, 1.0)];
                // Kept to where the frame can go, so it moves at once on the way back.
                let [left, top, right, bottom] = framing.crop(w, h).fractions();
                framing.centre = [(left + right) / 2.0, (top + bottom) / 2.0];
            }
        }
        let mut grips = Vec::new();
        if framing.shape == FREE_SHAPE {
            // Handles on the corners and edges: which edges each one moves.
            const GRIP: f32 = 14.0;
            let handles: [(Pos2, [bool; 4], egui::CursorIcon); 8] = [
                (frame.left_top(), [true, true, false, false], egui::CursorIcon::ResizeNwSe),
                (frame.right_bottom(), [false, false, true, true], egui::CursorIcon::ResizeNwSe),
                (frame.right_top(), [false, true, true, false], egui::CursorIcon::ResizeNeSw),
                (frame.left_bottom(), [true, false, false, true], egui::CursorIcon::ResizeNeSw),
                (frame.center_top(), [false, true, false, false], egui::CursorIcon::ResizeVertical),
                (frame.center_bottom(), [false, false, false, true], egui::CursorIcon::ResizeVertical),
                (frame.left_center(), [true, false, false, false], egui::CursorIcon::ResizeHorizontal),
                (frame.right_center(), [false, false, true, false], egui::CursorIcon::ResizeHorizontal),
            ];
            for (n, (at, edges, cursor)) in handles.into_iter().enumerate() {
                let grip = Rect::from_center_size(at, Vec2::splat(GRIP));
                let handle = ui.interact(grip, ui.scope_id().with(("cover-grip", n)), Sense::drag()).on_hover_cursor(cursor);
                if handle.dragged() {
                    framing.free = pull_edges(framing.free, edges, handle.drag_delta() / rect.size());
                }
                grips.push(at);
            }
        }
        let painter = ui.painter_at(rect);
        let shade = egui::Color32::from_black_alpha(140);
        for outside in [
            Rect::from_min_max(rect.min, Pos2::new(rect.max.x, frame.min.y)),
            Rect::from_min_max(Pos2::new(rect.min.x, frame.max.y), rect.max),
            Rect::from_min_max(Pos2::new(rect.min.x, frame.min.y), Pos2::new(frame.min.x, frame.max.y)),
            Rect::from_min_max(Pos2::new(frame.max.x, frame.min.y), Pos2::new(rect.max.x, frame.max.y)),
        ] {
            painter.rect_filled(outside, 0, shade);
        }
        painter.rect_stroke(frame, 2, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Inside);
        for at in grips {
            ui.painter().rect_filled(Rect::from_center_size(at, Vec2::splat(8.0)), 1, egui::Color32::WHITE);
        }
        ui.add_space(6.0);
        let mut done = false;
        let mut cancel = false;
        tool_bar(ui, Surface::Page, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Shape")).color(theme::muted()));
            let name = |index: usize| match CoverCrop::SHAPES.get(index) {
                Some(None) => tr("Whole photo").to_string(),
                Some(Some(shape)) => CoverCrop::shape_name(*shape),
                None => tr("Free").to_string(),
            };
            let before = framing.shape;
            egui::ComboBox::from_id_salt("cover-shape").selected_text(name(framing.shape)).height(360.0).show_ui(ui, |ui| {
                for index in 0..=FREE_SHAPE {
                    ui.selectable_value(&mut framing.shape, index, name(index));
                }
            });
            if framing.shape == FREE_SHAPE && before != FREE_SHAPE {
                // Free starts from the frame as it was, to pull from there.
                let mut was = framing;
                was.shape = before;
                framing.free = was.crop(w, h).fractions();
            } else if before == FREE_SHAPE && framing.shape != FREE_SHAPE {
                framing.leave_free(w, h);
            }
            tool_divider(ui);
            if framing.shape == FREE_SHAPE {
                ui.add_space(6.0);
                ui.label(RichText::new(tr("Drag the corners and edges to any shape")).size(12.0).color(theme::muted()));
                ui.add_space(6.0);
            } else {
                ui.add_space(6.0);
                ui.label(RichText::new(tr("Size")).color(theme::muted()));
                ui.add_space(4.0);
                ui.add(egui::Slider::new(&mut framing.zoom, 0.2..=1.0).show_value(false));
                ui.add_space(6.0);
            }
            if tool_button(ui, true, tr("Reset")).on_hover_text(tr("Show the whole photo")).clicked() {
                framing = Framing { shape: 0, zoom: 1.0, centre: [0.5, 0.5], free: CoverCrop::FULL.fractions() };
            }
            tool_divider(ui);
            cancel = tool_button(ui, true, tr("Cancel")).clicked();
            ui.add_space(4.0);
            done = ui.add(theme::primary_button(tr("Done")).corner_radius(TOOL_RADIUS)).clicked();
        });
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Drag the frame to choose what the cover shows. Only this story's cover changes; the photo and its edits stay as they are.")).size(12.0).color(theme::muted()));
        ui.add_space(12.0);
        let Some(editor) = &mut self.story else { return };
        if done {
            let crop = framing.crop(w, h);
            // A saved frame reads back a hair off its shape, so it is saved
            // again only when it was moved.
            if framing != Framing::of(editor.story.cover_crop, w, h) && crop != editor.story.cover_crop {
                actions.push(Action::CoverCrop(crop));
            }
            editor.framing = None;
        } else if cancel {
            editor.framing = None;
        } else {
            editor.framing = Some(framing);
        }
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

    pub(super) fn unsaved_story_modal(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.story else {
            self.modal = Modal::None;
            return;
        };
        let (mut close, mut retry, mut quit) = (false, false, false);
        let reason = editor.error.clone().unwrap_or_default();
        let response = egui::Modal::new(egui::Id::unique("unsaved-story")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(420.0);
            about::title_bar(ui, &self.logo, tr("The story couldn't be saved"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.label(tr("If you quit now, the changes since the last save are lost."));
                ui.label(RichText::new(trf("Couldn't save: {}", &[&reason])).color(theme::muted()));
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(theme::primary_button(tr("Try again"))).clicked() {
                        retry = true;
                    }
                    if ui.button(tr("Keep editing")).clicked() {
                        close = true;
                    }
                    if ui.add(theme::danger_button(tr("Quit anyway"))).clicked() {
                        quit = true;
                    }
                });
            });
        });
        if quit {
            editor.leaving = true;
            self.modal = Modal::None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if retry {
            if self.save_story() {
                self.modal = Modal::None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if close || response.should_close() {
            self.modal = Modal::None;
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
    /// The field whose "/" menu was shown on the last frame.
    slash_open: Option<egui::Id>,
    /// The field whose "/" menu Escape closed, until it is typed in again.
    slash_shut: Option<egui::Id>,
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
                    format.color = theme::accent();
                    format.underline = egui::Stroke::new(1.0, theme::accent());
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
    let choices = slash_choices(query);
    if choices.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::unique(("slash-menu", index)))
        .order(egui::Order::Foreground)
        .fixed_pos(below.left_bottom() + Vec2::new(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(Margin::same(6)).show(ui, |ui| {
                ui.set_min_width(300.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(tr("Blocks")).size(11.0).color(theme::muted()));
                // Only a pointer that moves takes the choice from ↑ and ↓,
                // not one left resting where the menu opened.
                let moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
                for (n, (label, block)) in choices.into_iter().take(SLASH_SHOWN).enumerate() {
                    let row = block_row(ui, &block, &label, Some(tr(block_hint(&block))), Some(n == ctl.slash));
                    if row.hovered() && moved {
                        ctl.slash = n;
                    }
                    if n == ctl.slash {
                        let at = egui::pos2(row.rect.right() - 10.0, row.rect.center().y);
                        ui.painter().text(at, egui::Align2::RIGHT_CENTER, "↵", egui::FontId::proportional(13.0), theme::muted());
                    }
                    if row.clicked() {
                        actions.push(Action::Replace(index, block));
                    }
                }
                ui.add_space(4.0);
                ui.separator();
                ui.label(RichText::new(tr("↑ ↓ to choose, Enter to add")).size(11.0).color(theme::muted()));
            });
        });
}

/// `text`, a shortcut written with Ctrl+, Shift+ and Alt+, with the Mac's
/// key symbols on a Mac.
fn keys(text: &str) -> String {
    if cfg!(target_os = "macos") {
        text.replace("Ctrl+", "⌘").replace("Shift+", "⇧").replace("Alt+", "⌥")
    } else {
        text.to_string()
    }
}

/// The ⚙ of the Story's bar: how the preview and export look, and whether
/// the trip's dates show under the title.
fn story_settings(ui: &mut egui::Ui, story: &mut Story) {
    let gear = tool_icon(ui, true, tr("Story settings"), theme::paint_settings_icon);
    egui::Popup::from_toggle_button_response(&gear).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(260.0);
        ui.label(RichText::new(tr("Template")).size(12.0).color(theme::muted()));
        for code in crate::trip::TEMPLATES {
            ui.radio_value(&mut story.template, code.to_string(), tr(crate::trip::template_name(code)));
        }
        ui.label(RichText::new(tr("How the preview and export look. The blocks stay the same.")).size(11.5).color(theme::muted()));
        ui.separator();
        ui.checkbox(&mut story.show_date, tr("Show the trip's dates"));
    });
}

/// What a toolbar is laid on. Every toolbar of the Story has the same
/// shape, buttons, and dividers; only their colours differ, so the bar
/// reads on the page and on any photo in either theme.
#[derive(Clone, Copy, PartialEq)]
enum Surface {
    Page,
    Photo,
}

/// A toolbar: a rounded bar of borderless buttons that light up when
/// pointed at, in groups split by [`tool_divider`].
fn tool_bar<R>(ui: &mut egui::Ui, surface: Surface, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let frame = match surface {
        Surface::Page => {
            let shadow = egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: egui::Color32::from_black_alpha(if theme::is_dark() { 70 } else { 20 }) };
            egui::Frame::new().fill(ui.visuals().window_fill).stroke(egui::Stroke::new(1.0, theme::hairline())).shadow(shadow)
        }
        Surface::Photo => egui::Frame::new().fill(egui::Color32::from_black_alpha(170)),
    };
    frame
        .corner_radius(TOOL_RADIUS + 3)
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                tool_style(ui, surface);
                add(ui)
            })
            .inner
        })
        .inner
}

/// The look of a toolbar's controls, for `ui` and what is added to it:
/// buttons with no fill or outline until pointed at, a chosen one tinted,
/// all the same height.
fn tool_style(ui: &mut egui::Ui, surface: Surface) {
    let (ink, over, pressed, chosen, chosen_ink, line) = match surface {
        Surface::Page => (theme::text(), theme::hover(), theme::select(), theme::select(), theme::accent(), theme::hairline()),
        Surface::Photo => {
            let white = egui::Color32::from_white_alpha;
            (egui::Color32::WHITE, white(40), white(60), white(56), egui::Color32::WHITE, white(70))
        }
    };
    let spacing = ui.spacing_mut();
    spacing.item_spacing.x = 2.0;
    spacing.button_padding = Vec2::new(8.0, 4.0);
    spacing.interact_size.y = TOOL;
    let visuals = ui.visuals_mut();
    visuals.selection.bg_fill = chosen;
    visuals.selection.stroke.color = chosen_ink;
    visuals.widgets.noninteractive.bg_stroke.color = line;
    let widgets = &mut visuals.widgets;
    for (state, fill) in [(&mut widgets.inactive, egui::Color32::TRANSPARENT), (&mut widgets.hovered, over), (&mut widgets.active, pressed), (&mut widgets.open, pressed)] {
        state.weak_bg_fill = fill;
        state.fg_stroke.color = ink;
        state.bg_stroke = egui::Stroke::NONE;
        state.corner_radius = egui::CornerRadius::same(TOOL_RADIUS);
    }
}

/// A toolbar button with text or a symbol, at least as wide as it is tall.
fn tool_button(ui: &mut egui::Ui, enabled: bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(text).min_size(Vec2::splat(TOOL)))
}

/// A square toolbar button with an icon drawn by `paint`, named `name`
/// for screen readers and on hover.
fn tool_icon(ui: &mut egui::Ui, enabled: bool, name: &str, paint: impl FnOnce(&egui::Painter, Pos2, egui::Color32)) -> egui::Response {
    let response = tool_button(ui, enabled, "");
    let ink = ui.visuals().widgets.inactive.fg_stroke.color;
    paint(ui.painter(), response.rect.center(), if enabled { ink } else { ink.gamma_multiply(0.35) });
    theme::named(response, name)
}

/// A borderless button on the page, like a toolbar's; one that adds
/// something reads "+ …" in the accent colour.
fn page_button(ui: &mut egui::Ui, text: &str, adds: bool) -> egui::Response {
    ui.scope(|ui| {
        tool_style(ui, Surface::Page);
        let label = if adds { RichText::new(format!("+  {text}")).color(theme::accent()) } else { RichText::new(text) };
        ui.add(egui::Button::new(label).min_size(Vec2::new(TOOL, TOOL)))
    })
    .inner
}

/// Why the photo strip shows nothing and what brings photos back: the
/// "not in the story yet" filter hid them all, or the starred or day
/// filter left none, or the Trip has none.
fn empty_strip(day: DayFilter, starred: bool, all_used: bool) -> &'static str {
    if all_used {
        tr("Every photo here is already in the story. Turn off “Not in the story yet” to use one again.")
    } else if starred {
        tr("None of these photos has a star. Turn off “Starred only” to see them all.")
    } else if day != DayFilter::Any {
        tr("No photos on this day. Choose “All days” to pick from the whole trip.")
    } else {
        tr("No photos in this trip yet. Select photos in the library and use Add to Trip.")
    }
}

/// An empty place to fill, drawn alike for the cover, a grid's places,
/// and a route with no track: a hairline outline and muted `text`, lit
/// when pointed at or the keyboard is on it.
fn empty_slot(ui: &egui::Ui, rect: Rect, lit: bool, text: &str, size: f32) {
    let painter = ui.painter();
    let fill = if lit { theme::select() } else { egui::Color32::TRANSPARENT };
    painter.rect(rect, CORNER, fill, egui::Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
    let ink = if lit { theme::accent() } else { theme::muted() };
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(size), ink);
}

/// A kind of block to add, alike wherever blocks are offered: its mark in
/// a tile, its name, and `hint` under it when given. It is as wide as its
/// words need, a long hint cut short with "…". `lit` says whether it is
/// the one chosen; `None` leaves that to pointing at it or the keyboard.
fn block_row(ui: &mut egui::Ui, block: &Block, label: &str, hint: Option<&str>, lit: Option<bool>) -> egui::Response {
    const HINT_WIDTH: f32 = 340.0;
    let (height, tile) = if hint.is_some() { (44.0, 30.0) } else { (32.0, 22.0) };
    let visuals = ui.visuals();
    let ink = visuals.strong_text_color();
    let name = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(14.0), ink);
    let note = hint.map(|hint| {
        let mut job = egui::text::LayoutJob::simple_singleline(hint.to_string(), egui::FontId::proportional(11.5), theme::muted());
        job.wrap = egui::text::TextWrapping::truncate_at_width(HINT_WIDTH);
        ui.painter().layout_job(job)
    });
    // The tile and its margins before the words, and room after them for
    // a mark such as ↵ or a submenu's arrow.
    let left = 6.0 + tile + 10.0;
    let words = name.size().x.max(note.as_ref().map_or(0.0, |note| note.size().x));
    let width = ui.available_width().max(left + words + 24.0);
    let (rect, row) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    row.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let painter = ui.painter();
    let visuals = ui.visuals();
    if lit.unwrap_or_else(|| row.hovered() || row.has_focus()) {
        painter.rect_filled(rect, TOOL_RADIUS, theme::accent().gamma_multiply(0.14));
    }
    let icon = Rect::from_center_size(egui::pos2(rect.left() + 6.0 + tile / 2.0, rect.center().y), Vec2::splat(tile));
    painter.rect_filled(icon, TOOL_RADIUS, visuals.faint_bg_color);
    painter.rect_stroke(icon, TOOL_RADIUS, egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
    if matches!(block, Block::Link { .. }) {
        paint_link_icon(painter, icon.center(), ink);
    } else {
        painter.text(icon.center(), egui::Align2::CENTER_CENTER, block_icon(block), egui::FontId::proportional(tile * 0.46), ink);
    }
    let left = rect.left() + left;
    match note {
        Some(note) => {
            painter.galley(egui::pos2(left, rect.top() + 7.0), name, ink);
            painter.galley(egui::pos2(left, rect.top() + 25.0), note, theme::muted());
        }
        None => painter.galley(egui::pos2(left, rect.center().y - name.size().y / 2.0), name, ink),
    }
    row.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Whether the pointer was just pressed on the page outside `rect`: not on
/// a toolbar, menu, or list open above the page, which are layers of
/// their own.
fn pressed_outside(ui: &egui::Ui, rect: Rect) -> bool {
    let at = ui.input(|input| input.pointer.primary_pressed().then(|| input.pointer.interact_pos()).flatten());
    at.is_some_and(|at| !rect.contains(at) && ui.ctx().layer_id_at(at) == Some(ui.layer_id()))
}

/// Two links of a chain, slanted, about 14 points wide.
fn paint_link_icon(painter: &egui::Painter, center: Pos2, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.4, color);
    let slant = Vec2::angled(-std::f32::consts::FRAC_PI_4);
    for side in [-1.0, 1.0] {
        let middle = center + slant * (side * 2.6);
        let points: Vec<Pos2> = (0..=24)
            .map(|step| {
                let angle = std::f32::consts::TAU * step as f32 / 24.0;
                // An oval along the slant, 4.5 long either way and 2.4 across.
                middle + slant * (4.5 * angle.cos()) + slant.rot90() * (2.4 * angle.sin())
            })
            .collect();
        painter.add(egui::Shape::line(points, stroke));
    }
}

/// The grip a block is dragged by: two columns of dots, lit when pointed at.
fn drag_handle(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(18.0, TOOL), Sense::drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Drag to move this block")));
    let painter = ui.painter();
    if response.hovered() || response.dragged() {
        painter.rect_filled(rect, TOOL_RADIUS, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    for (x, y) in [(-3.0, -5.0), (3.0, -5.0), (-3.0, 0.0), (3.0, 0.0), (-3.0, 5.0), (3.0, 5.0)] {
        painter.circle_filled(rect.center() + Vec2::new(x, y), 1.4, theme::muted());
    }
    response
}

/// A thin upright line between groups of a toolbar's controls.
fn tool_divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, TOOL - 10.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color));
}

/// The tools of a photo in a block, on its top-right corner while it is
/// pointed at or the keyboard is on it or them: Use as cover, unless it is
/// the cover, and Remove for a photo of a gallery or grid, whose English
/// name `remove` carries. Hidden while anything is dragged, so they never
/// stand in the way of a drop.
fn photo_tools(ui: &mut egui::Ui, response: &egui::Response, rect: Rect, media_id: i64, cover: Option<i64>, remove: Option<(usize, usize, &'static str)>, actions: &mut Vec<Action>) {
    let key = response.id.with("photo-tools");
    let tools_focused = ui.ctx().data(|data| data.get_temp::<bool>(key)).unwrap_or(false);
    let is_cover = cover == Some(media_id);
    let wanted = ui.rect_contains_pointer(rect) || response.has_focus() || tools_focused;
    if !wanted || egui::DragAndDrop::has_any_payload(ui.ctx()) || (is_cover && remove.is_none()) || rect.width() < 60.0 {
        ui.ctx().data_mut(|data| data.remove::<bool>(key));
        return;
    }
    let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(6.0)).layout(Layout::right_to_left(Align::Min)));
    let mut focused = false;
    tool_bar(&mut bar, Surface::Photo, |ui| {
        // Laid out from the right, so ✕ comes first to sit in the corner.
        if let Some((index, n, name)) = remove {
            let button = theme::named(tool_button(ui, true, "✕"), tr(name));
            if button.clicked() {
                actions.push(Action::RemoveFromGallery(index, n));
            }
            focused |= button.has_focus();
        }
        if !is_cover {
            // A small tile has room for a word only.
            let label = if rect.width() >= 220.0 { tr("Use as cover") } else { tr("Cover") };
            let button = theme::named(tool_button(ui, true, label), tr("Use as cover"));
            if button.clicked() {
                actions.push(Action::Cover(Some(media_id)));
            }
            focused |= button.has_focus();
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(key, focused));
}

/// The name of a cover's frame: its shape, or Free.
fn frame_name(crop: CoverCrop, w: u32, h: u32) -> String {
    match CoverCrop::SHAPES.get(Framing::of(crop, w, h).shape) {
        Some(Some(shape)) => CoverCrop::shape_name(*shape),
        Some(None) => tr("Whole photo").to_string(),
        None => tr("Free").to_string(),
    }
}

/// A small "Cover" label on the corner of a block's photo that is the
/// Story's cover, so it is known where the cover came from.
fn cover_chip(ui: &egui::Ui, rect: Rect) {
    if rect.width() < 60.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    let text = painter.layout_no_wrap(tr("Cover").to_string(), egui::FontId::proportional(11.0), egui::Color32::WHITE);
    let chip = Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 6.0), text.size() + Vec2::new(12.0, 4.0));
    painter.rect_filled(chip, 4, egui::Color32::from_black_alpha(170));
    painter.galley(chip.min + Vec2::new(6.0, 2.0), text, egui::Color32::WHITE);
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

/// The mark of a kind of block where blocks are offered; [`block_row`]
/// draws a Link's as the toolbar's link icon instead.
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
    let slash = kind == TextKind::Paragraph && text.starts_with('/') && !text.contains('\n') && ctl.slash_shut != Some(id) && !slash_choices(&text[1..]).is_empty();
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
        if ctl.slash_shut == Some(id) {
            ctl.slash_shut = None;
        }
        if kind == TextKind::Paragraph {
            actions.extend(markdown_shortcut(text, index));
        }
    }
    // Typing in the field may have taken the "/" away this frame, and the
    // menu goes with the caret.
    if let Some(query) = text.strip_prefix('/').filter(|query| slash && response.has_focus() && !query.contains('\n')) {
        ctl.slash_open = Some(id);
        slash_menu(ui, query, response.rect, index, ctl, actions);
    }
}

/// What typing `text` into empty paragraph `index` turns it into, as
/// Markdown would have it. A separator goes on with a paragraph after it.
fn markdown_shortcut(text: &str, index: usize) -> Vec<Action> {
    let block = match text {
        "# " => Block::Heading(String::new()),
        "## " => Block::Subheading(String::new()),
        "- " | "* " => Block::List { ordered: false, items: vec![String::new()] },
        "1. " => Block::List { ordered: true, items: vec![String::new()] },
        "> " => Block::Quote { text: String::new(), citation: String::new() },
        "---" => return vec![Action::Replace(index, Block::Separator), Action::Insert(index + 1, Block::Paragraph(String::new()))],
        _ => return Vec::new(),
    };
    vec![Action::Replace(index, block)]
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
    let frame = egui::Frame::new().stroke(egui::Stroke::new(1.0, theme::hairline())).corner_radius(CORNER).inner_margin(Margin::symmetric(12, 10));
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
                let width = ui.available_width() - TOOL - ui.spacing().item_spacing.x;
                if ui.add(egui::TextEdit::singleline(value).hint_text(tr("Details")).desired_width(width)).changed() {
                    *typed = Some((uid.to_string(), field + 1));
                }
                if theme::named(page_button(ui, "✕", false), tr("Remove this line")).clicked() {
                    remove = Some(n);
                }
            });
        }
        if let Some(n) = remove {
            items.remove(n);
            *typed = Some((String::new(), 0));
        }
        if page_button(ui, tr("Add a line"), true).clicked() {
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
                let button = page_button(ui, tr("From the trip…"), false).on_hover_text(tr("Links kept with the trip that may be published"));
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
    ui.painter().vline(rect.left() + 2.0, rect.y_range(), egui::Stroke::new(3.0, theme::accent()));
}

/// The format buttons of a text block: styles for the selection, the
/// heading's level, and the list's kind.
fn text_tools(ui: &mut egui::Ui, index: usize, block: &Block, actions: &mut Vec<Action>) {
    let name = |name: &str, shortcut: &str| format!("{name} ({})", keys(shortcut));
    let styles = [
        (RichText::new("B").strong(), Mark::Bold, name(tr("Bold"), "Ctrl+B")),
        (RichText::new("I").italics(), Mark::Italic, name(tr("Italic"), "Ctrl+I")),
        (RichText::new("S").strikethrough(), Mark::Strike, name(tr("Strikethrough"), "Ctrl+Shift+X")),
        (RichText::new("</>").monospace().size(12.0), Mark::Code, name(tr("Code"), "Ctrl+E")),
    ];
    for (label, mark, name) in styles {
        if theme::named(tool_button(ui, true, label), &name).clicked() {
            actions.push(Action::Format(mark));
        }
    }
    if tool_icon(ui, true, &name(tr("Link"), "Ctrl+K"), paint_link_icon).clicked() {
        actions.push(Action::Format(Mark::Link));
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
        tool_divider(ui);
        if theme::named(tool_button(ui, true, label), tr(name)).clicked() {
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
        ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(3.0, theme::accent()));
    } else if response.hovered() || response.has_focus() {
        // The + at the right end, clear of the tools floating at the left.
        let plus = egui::pos2(rect.right() - GAP, rect.center().y);
        ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, theme::accent()));
        ui.painter().circle_filled(plus, GAP / 2.0 + 1.0, theme::ACCENT);
        ui.painter().text(plus, egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(14.0), egui::Color32::WHITE);
    }
    if let Some(moved) = response.dnd_release_payload::<MoveBlock>() {
        actions.push(Action::Move(moved.0, at));
    }
    let response = theme::named(response, tr("Add a block here"));
    egui::Popup::menu(&response).show(|ui| block_menu(ui, at, actions));
}

/// The blocks that can be added at `at`, in the rows of the "/" menu with
/// what each is for on hover, and the Photo grid's layouts beside.
fn block_menu(ui: &mut egui::Ui, at: usize, actions: &mut Vec<Action>) {
    ui.set_min_width(220.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    for (label, block) in new_blocks() {
        let row = block_row(ui, &block, tr(label), None, None).on_hover_text(tr(block_hint(&block)));
        if row.clicked() {
            actions.push(Action::Insert(at, block));
            ui.close();
        }
    }
    let grid = Block::Grid { columns: 2, rows: 2, caption: String::new(), media: Vec::new() };
    let row = block_row(ui, &grid, tr("Photo grid"), None, None);
    let arrow = Rect::from_center_size(egui::pos2(row.rect.right() - 12.0, row.rect.center().y), Vec2::splat(ui.spacing().icon_width));
    ui.painter().add(egui::containers::menu::SubMenuButton::arrow_shape(arrow, theme::muted()));
    egui::containers::menu::SubMenu::new().show(ui, &row, |ui| grid_menu(ui, at, actions));
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

/// The part of a texture for the part `[left, top, right, bottom]` of the
/// photo as shown, turned `rotation` degrees clockwise.
fn texture_uv([left, top, right, bottom]: [f32; 4], rotation: i32) -> Rect {
    let [u0, v0, u1, v1] = match rotation.rem_euclid(360) {
        90 => [top, 1.0 - right, bottom, 1.0 - left],
        180 => [1.0 - right, 1.0 - bottom, 1.0 - left, 1.0 - top],
        270 => [1.0 - bottom, left, 1.0 - top, right],
        _ => [left, top, right, bottom],
    };
    Rect::from_min_max(Pos2::new(u0, v0), Pos2::new(u1, v1))
}

/// Writes the editor's unsaved changes to `db`. Returns whether all is
/// saved; when not, the changes stay in the editor with the reason.
fn save(db: &crate::db::Database, editor: &mut StoryEditor) -> bool {
    if editor.unsaved.is_none() {
        return true;
    }
    match db.save_story(&editor.story, editor.story.cover != editor.saved_cover, &editor.blocks, editor.story.revision) {
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
        Action::Cover(media) => {
            // A new cover starts whole; the old one's frame wouldn't fit it.
            if editor.story.cover != media {
                editor.story.cover_crop = CoverCrop::FULL;
                editor.framing = None;
            }
            editor.choosing_cover = false;
            editor.story.cover = media;
        }
        Action::CoverCrop(crop) => editor.story.cover_crop = crop,
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
fn trip_photos(db: &crate::db::Database, trip_id: i64, day: DayFilter, starred: bool) -> Vec<i64> {
    let query = MemberQuery { day, starred, ..MemberQuery::default() };
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
    let combo = egui::ComboBox::from_id_salt(("block-layout", uid)).selected_text(shown).height(360.0).show_ui(ui, |ui| {
        let mut choice = |ui: &mut egui::Ui, shape: Shape, text: String, fits: bool, why: &str| {
            if ui.add_enabled(fits, egui::Button::selectable(current == shape, text)).on_disabled_hover_text(why).clicked() {
                actions.push(Action::Arrange(index, shape));
            }
        };
        choice(ui, Shape::Photo, tr("Single photo").into(), count == 1, tr("For a block with one photo"));
        choice(ui, Shape::Gallery, tr("Gallery").into(), count >= 2, tr("For a block with two photos or more"));
        ui.separator();
        for (columns, rows) in GRID_LAYOUTS {
            let text = format!("{}  ·  {}", trf("Grid {} × {}", &[&columns, &rows]), trf("{} photos", &[&(columns * rows)]));
            choice(ui, Shape::Grid(columns, rows), text, true, "");
        }
    });
    combo.response.on_hover_text(tr("How this block's photos are laid out. Drag a photo onto another to put them together."));
}

/// The layouts of a Photo grid to add at `at`, each its own choice; its
/// photos are picked from the Trip afterwards.
fn grid_menu(ui: &mut egui::Ui, at: usize, actions: &mut Vec<Action>) {
    for (columns, rows) in GRID_LAYOUTS {
        let text = format!("{columns} × {rows}  ·  {}", trf("{} photos", &[&(columns * rows)]));
        if ui.button(text).clicked() {
            actions.push(Action::Insert(at, Block::Grid { columns, rows, caption: String::new(), media: Vec::new() }));
            ui.close();
        }
    }
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
            cover_crop: crate::trip::CoverCrop::default(),
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
            reading: HashSet::new(),
            job: None,
            reveal: None,
            picking: None,
            active: None,
            hovered: None,
            text: TextCtl::default(),
            framing: None,
            choosing_cover: false,
            queued: Vec::new(),
            leaving: false,
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
    fn an_empty_photo_strip_names_the_filter_that_emptied_it() {
        let says = |day, starred, all_used, words: &str| assert!(empty_strip(day, starred, all_used).contains(words), "{words}");
        says(DayFilter::Day(1), true, true, "Not in the story yet");
        says(DayFilter::Day(1), true, false, "Starred only");
        says(DayFilter::Day(1), false, false, "All days");
        says(DayFilter::Unassigned, false, false, "All days");
        says(DayFilter::Any, false, false, "Add to Trip");
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
    fn a_cover_frame_follows_the_photo_as_turned() {
        let part = [0.1, 0.2, 0.6, 0.9];
        assert_eq!(texture_uv(part, 0), Rect::from_min_max(Pos2::new(0.1, 0.2), Pos2::new(0.6, 0.9)));
        let turned = texture_uv(part, 90);
        assert!((turned.min.x - 0.2).abs() < 1e-6 && (turned.min.y - 0.4).abs() < 1e-6 && (turned.max.x - 0.9).abs() < 1e-6 && (turned.max.y - 0.9).abs() < 1e-6);
        assert_eq!(texture_uv([0.0, 0.0, 1.0, 1.0], 270), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)));
        let framing = Framing::of(CoverCrop::framed(Some((16, 9)), 3000, 2000, 0.6, [0.4, 0.5]), 3000, 2000);
        assert_eq!(CoverCrop::SHAPES[framing.shape], Some((16, 9)), "read back as 16:9");
        let xpan = CoverCrop::framed(Some((65, 24)), 6000, 4000, 1.0, [0.5, 0.5]);
        assert_eq!(CoverCrop::SHAPES[Framing::of(xpan, 6000, 4000).shape], Some((65, 24)), "the XPan panorama reads back as itself");
        let [left, top, right, bottom] = xpan.fractions();
        assert!(((right - left) * 6000.0 / ((bottom - top) * 4000.0) - 65.0 / 24.0).abs() < 0.01);
        assert!((framing.zoom - 0.6).abs() < 0.01);
        let odd = CoverCrop::from_fractions([0.1, 0.1, 0.47, 0.9]);
        let free = Framing::of(odd, 3000, 2000);
        assert_eq!(free.shape, FREE_SHAPE, "a frame no shape fits is free");
        assert_eq!(free.crop(3000, 2000), odd);
    }

    #[test]
    fn a_smaller_frame_of_the_photos_own_shape_reads_back_as_the_whole_photo() {
        // 1.85:1 is not an offered shape.
        let crop = CoverCrop::framed(None, 3700, 2000, 0.6, [0.4, 0.5]);
        let framing = Framing::of(crop, 3700, 2000);
        assert_eq!(framing.shape, 0);
        assert!((framing.zoom - 0.6).abs() < 0.01);
        assert_eq!(framing.crop(3700, 2000), crop);
    }

    #[test]
    fn another_cover_or_an_undo_of_it_closes_the_frame() {
        let mut editor = editor(vec![Block::Separator]);
        let framing = Framing::of(CoverCrop::FULL, 3000, 2000);
        let before = editor.snapshot();
        editor.framing = Some(framing);
        apply(&mut editor, Action::Cover(Some(7)));
        assert!(editor.framing.is_none(), "another cover");
        editor.framing = Some(framing);
        let same = editor.snapshot();
        editor.restore(same);
        assert!(editor.framing.is_some(), "an undo that leaves the cover keeps the frame");
        editor.restore(before);
        assert!(editor.framing.is_none(), "undone to no cover");
    }

    #[test]
    fn free_handles_stop_at_the_smallest_frame() {
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3);
        let frame = [0.2, 0.2, 0.6, 0.6];
        let left = [true, false, false, false];
        assert!(near(pull_edges(frame, left, Vec2::new(0.1, 0.0)), [0.3, 0.2, 0.6, 0.6]));
        // Pulled past the opposite edge, the left edge stops short of it and the right stays.
        assert!(near(pull_edges(frame, left, Vec2::new(0.9, 0.0)), [0.55, 0.2, 0.6, 0.6]));
        let top_left = [true, true, false, false];
        assert!(near(pull_edges(frame, top_left, Vec2::new(-0.5, 0.9)), [0.0, 0.55, 0.6, 0.6]));
        let right = [false, false, true, false];
        assert!(near(pull_edges(frame, right, Vec2::new(-0.9, 0.0)), [0.2, 0.2, 0.25, 0.6]));
        assert!(near(pull_edges(frame, right, Vec2::new(0.9, 0.0)), [0.2, 0.2, 1.0, 0.6]));
    }

    #[test]
    fn a_fixed_shape_takes_over_where_the_free_frame_was() {
        let mut framing = Framing::of(CoverCrop::from_fractions([0.6, 0.5, 0.9, 0.9]), 3000, 2000);
        assert_eq!(framing.shape, FREE_SHAPE);
        framing.shape = CoverCrop::SHAPES.iter().position(|shape| *shape == Some((1, 1))).unwrap();
        framing.leave_free(3000, 2000);
        let [left, top, right, bottom] = framing.crop(3000, 2000).fractions();
        assert!(((left + right) / 2.0 - 0.75).abs() < 0.01 && ((top + bottom) / 2.0 - 0.7).abs() < 0.01, "centred where it was");
        let (free_area, square_area) = (0.3 * 0.4, (right - left) * (bottom - top));
        assert!((square_area / free_area - 1.0).abs() < 0.05, "about as large: {square_area} vs {free_area}");
    }

    #[test]
    fn escape_closes_the_innermost_thing_first() {
        let mut editor = editor(vec![Block::Gallery(Vec::new()), Block::Paragraph("/hea".into())]);
        let field = field_id(&editor.blocks[1].uid, 0);
        editor.text.slash_open = Some(field);
        editor.framing = Some(Framing::of(CoverCrop::FULL, 3000, 2000));
        editor.choosing_cover = true;
        editor.picking = Some(Picker { uid: editor.blocks[0].uid.clone(), day: DayFilter::Any, starred: false, unused: false, choices: Vec::new() });
        editor.active = Some(editor.blocks[1].uid.clone());
        assert_eq!(editor.escape(), Some(Closed::SlashMenu));
        assert_eq!(editor.text.slash_shut, Some(field), "the menu stays shut until typed in");
        assert_eq!(editor.escape(), Some(Closed::Framing));
        assert_eq!(editor.escape(), Some(Closed::CoverChoices));
        assert_eq!(editor.escape(), Some(Closed::PhotoStrip));
        assert!(matches!(editor.queued.as_slice(), [Action::Remove(0)]), "the empty gallery goes");
        assert_eq!(editor.escape(), Some(Closed::Block));
        assert_eq!(editor.escape(), None);
    }

    #[test]
    fn a_new_cover_starts_whole() {
        let mut editor = editor(vec![Block::Photo(MediaUse::new(1)), Block::Photo(MediaUse::new(2))]);
        apply(&mut editor, Action::Cover(Some(1)));
        apply(&mut editor, Action::CoverCrop(CoverCrop([0, 0, 5_000, 5_000])));
        assert_eq!(editor.story.cover_key(), Some(-1));
        apply(&mut editor, Action::Cover(Some(1)));
        assert!(!editor.story.cover_crop.is_full(), "the same cover keeps its frame");
        apply(&mut editor, Action::Cover(Some(2)));
        assert!(editor.story.cover_crop.is_full());
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
    fn markdown_at_the_start_of_a_paragraph_makes_its_block() {
        let mut editor = editor(vec![Block::Paragraph("# ".into()), Block::Paragraph("## ".into()), Block::Paragraph("---".into())]);
        for (index, text) in [(0, "# "), (1, "## "), (2, "---")] {
            for action in markdown_shortcut(text, index) {
                apply(&mut editor, action);
            }
        }
        assert_eq!(kinds(&editor), ["heading", "subheading", "separator", "paragraph"]);
        assert_eq!(editor.text.focus.as_ref().map(|focus| focus.0.as_str()), Some(editor.blocks[3].uid.as_str()), "writing goes on after the line");
        assert!(markdown_shortcut("#x", 0).is_empty() && markdown_shortcut("-- ", 0).is_empty());
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

    #[test]
    fn a_block_from_a_newer_picasso_outlives_this_editor_and_a_restart() {
        let dir = std::env::temp_dir().join(format!("picasso-story-newer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.db");
        let db = crate::db::Database::open(&path).unwrap();
        let trip_id = db.create_trip(&crate::trip::TripDraft::new("Kyoto", "Asia/Tokyo")).unwrap();
        let story_id = db.create_story(trip_id, "Kyoto").unwrap();
        let newer = Block::read("paragraph", crate::trip::BLOCK_VERSION + 1, r#"{"text":"hi","style":"lead"}"#, Vec::new());
        let written: Vec<StoryBlock> = [Block::Paragraph("Before".into()), newer, Block::Paragraph("After".into())].into_iter().map(StoryBlock::new).collect();
        db.save_story_blocks(story_id, 1, &written).unwrap();

        // Opened as `open_story` opens it, twice: here and in another window
        // still showing the Story as it was.
        let open = |db: &crate::db::Database| {
            let mut editor = editor(Vec::new());
            editor.story = db.story(story_id).unwrap();
            editor.saved_cover = editor.story.cover;
            editor.blocks = db.story_blocks(story_id);
            editor
        };
        let mut editor = open(&db);
        let mut elsewhere = open(&db);
        let kept = editor.blocks[1].clone();
        assert!(matches!(kept.block, Block::Unknown { .. }));

        // Typed around, joined from either side, moved, removed and undone.
        if let Block::Paragraph(text) = &mut editor.blocks[0].block {
            text.push_str(", edited");
        }
        apply(&mut editor, Action::Merge(1));
        apply(&mut editor, Action::Merge(2));
        assert_eq!(editor.blocks.len(), 3, "nothing is joined to or from it");
        apply(&mut editor, Action::Move(1, 0));
        let before = editor.snapshot();
        apply(&mut editor, Action::Remove(0));
        assert!(editor.blocks.iter().all(|block| block.uid != kept.uid));
        editor.restore(before);
        assert!(editor.unsaved.is_some());
        let revision = db.save_story(&editor.story, false, &editor.blocks, editor.story.revision).unwrap();
        editor.story.revision = revision;

        // The other window, saving what it still shows, is refused.
        elsewhere.blocks.retain(|block| block.uid != kept.uid);
        assert!(db.save_story(&elsewhere.story, false, &elsewhere.blocks, elsewhere.story.revision).is_err());

        drop(db);
        let db = crate::db::Database::open(&path).unwrap();
        let blocks = db.story_blocks(story_id);
        assert_eq!(blocks[0], kept, "the same block, id, version and stored words, where it was moved");
        assert_eq!((blocks[0].block.version(), blocks[0].block.payload()), (crate::trip::BLOCK_VERSION + 1, r#"{"text":"hi","style":"lead"}"#.to_string()));
        assert_eq!(blocks[1..].iter().map(|block| block.block.clone()).collect::<Vec<_>>(), [Block::Paragraph("Before, edited".into()), Block::Paragraph("After".into())]);
        assert_eq!(db.story(story_id).unwrap().revision, revision);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_story_is_saved_after_a_pause_or_within_seconds_and_a_failed_save_keeps_it_open() {
        let dir = std::env::temp_dir().join(format!("picasso-story-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.db");
        let db = crate::db::Database::open(&path).unwrap();
        let trip_id = db.create_trip(&crate::trip::TripDraft::new("Kyoto", "Asia/Tokyo")).unwrap();
        let story_id = db.create_story(trip_id, "Kyoto").unwrap();
        let mut editor = editor(Vec::new());
        editor.story = db.story(story_id).unwrap();

        editor.story.title = "Kyoto in July".into();
        editor.touch();
        assert!(!editor.due(), "not while typing");
        let now = Instant::now();
        editor.unsaved = Some((now - SAVE_PAUSE, now - SAVE_PAUSE));
        assert!(editor.due(), "after a pause");
        editor.unsaved = Some((now - SAVE_WITHIN, now));
        assert!(editor.due(), "within seconds of the first change, even while typing on");

        // A library that can't be written, as on a full or read-only disk.
        let reader = crate::db::Database::open_reader(&path).unwrap();
        assert!(!save(&reader, &mut editor), "so closing the Story or quitting is held back");
        assert!(editor.error.is_some() && editor.unsaved.is_some(), "the change stays, with the reason");
        assert_eq!(editor.story.revision, 1);
        assert!(!editor.due(), "not tried again on its own, only by Try again or another edit");

        // Picasso stopping now: the library has the Story as last saved.
        drop((reader, db));
        let db = crate::db::Database::open(&path).unwrap();
        assert_eq!(db.story(story_id).unwrap().title, "Kyoto");

        // Try again.
        editor.error = None;
        assert!(save(&db, &mut editor));
        assert!(editor.unsaved.is_none() && editor.error.is_none());
        assert_eq!(editor.story.revision, 2);
        assert!(save(&db, &mut editor), "nothing left to save");
        drop(db);
        let db = crate::db::Database::open(&path).unwrap();
        let saved = db.story(story_id).unwrap();
        assert_eq!((saved.title.as_str(), saved.revision), ("Kyoto in July", 2), "and after a restart");
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_routes_elevation_not_read_yet_is_read_by_the_job_not_left_out() {
        let dir = std::env::temp_dir().join(format!("picasso-story-elevation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let track = dir.join("climb.gpx");
        let points: String = (0..20).map(|n| format!(r#"<trkpt lat="35.0{n:02}" lon="135.7"><ele>{}</ele></trkpt>"#, 100 + n * 10)).collect();
        std::fs::write(&track, format!(r#"<?xml version="1.0"?><gpx version="1.1" creator="test"><trk><trkseg>{points}</trkseg></trk></gpx>"#)).unwrap();
        let flat = dir.join("missing.gpx");

        let mut profiles = HashMap::new();
        read_profiles(&mut profiles, vec![("climb".into(), vec![track]), ("gone".into(), vec![flat])]);
        let climb = profiles.get("climb").expect("read on the job's thread");
        assert!(climb.len() >= 2 && climb.first().unwrap()[1] < climb.last().unwrap()[1], "{climb:?}");
        assert!(!profiles.contains_key("gone"), "a track that can't be read draws its route alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_last_export_is_compared_after_each_change_and_only_the_newest_answer_counts() {
        let mut last = LastExport::new("/out/Kyoto".into(), "made".into(), None);
        let saved = (4, (10, 1));
        let first = last.ask(saved).expect("compared once after the export");
        assert_eq!(last.ask(saved), None, "not again until something changes");
        last.answer(first, "made");
        assert!(!last.stale, "nothing that shows has changed");

        let older = last.ask((5, (11, 1))).unwrap();
        let newer = last.ask((5, (12, 1))).unwrap();
        last.answer(newer, "changed");
        last.answer(older, "made");
        assert!(last.stale, "an answer to an older comparison is dropped");

        // Exported again: what an earlier comparison finds no longer counts.
        let mut again = LastExport::new("/out/Kyoto 2".into(), "changed".into(), Some(&last));
        again.answer(newer, "made");
        assert!(!again.stale);
        let fresh = again.ask((6, (13, 1))).unwrap();
        again.answer(fresh, "changed");
        assert!(!again.stale, "the new export shows the Story as it is");
    }
}
