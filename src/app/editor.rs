//! Editor session of the viewer. Every change becomes a step in the photo's
//! history, saved in the library; the original file is never written.
//! Steps can be undone and redone one at a time or all at once, copied to
//! other photos, and the result exported to a new file.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use image::RgbaImage;

use crate::edit::{self, presets, Control, EditJob, ExportOptions, Geometry, History, Kind, Options, Purpose, Settings};
use crate::i18n::{tr, trf};
use crate::photo_info::Histogram;
use crate::types::Photo;
use crate::worker::UiEvent;

use super::editor_stage::StageDrag;
use super::PicassoApp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Adjust,
    Crop,
    /// Four corner handles that turn, tilt, and skew the photo.
    Transform,
    Mask,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Compare {
    Off,
    /// The unedited photo in place of the edit.
    Before,
    SideBySide,
    /// Before left of the divider, at this fraction of the width.
    Split(f32),
}

/// Crop shapes: name and width over height. Zero is free, and a negative
/// ratio keeps the photo's own shape.
pub(super) const ASPECTS: [(&str, f32); 9] = [
    ("Free", 0.0),
    ("Original", -1.0),
    ("1:1", 1.0),
    ("4:5", 4.0 / 5.0),
    ("5:7", 5.0 / 7.0),
    ("2:3", 2.0 / 3.0),
    ("3:4", 3.0 / 4.0),
    ("9:16", 9.0 / 16.0),
    ("10:16", 10.0 / 16.0),
];

pub(super) const GUIDES: [&str; 5] = ["Rule of thirds", "Golden ratio", "Grid", "Diagonals", "None"];

pub(super) const EXPORT_FORMATS: [(&str, &str); 3] = [("JPEG", "jpg"), ("PNG", "png"), ("TIFF", "tif")];
pub(super) const EXPORT_EDGES: [Option<u32>; 5] = [None, Some(4096), Some(2048), Some(1600), Some(1080)];

/// A change still under the pointer. It becomes a step once the pointer
/// is released.
pub(super) struct Live {
    pub kind: Kind,
    pub settings: Settings,
    /// Replace the newest step of the same kind instead of adding one.
    pub amend: bool,
}

pub(super) struct EditSession {
    pub id: i64,
    pub history: History,
    pub live: Option<Live>,
    pub base: Option<Arc<RgbaImage>>,
    pub error: Option<String>,
    preview: Option<egui::TextureHandle>,
    before: Option<egui::TextureHandle>,
    pub histogram: Option<Box<Histogram>>,
    /// The previous photo's histogram, shown until this one's is rendered
    /// or the instant passes, so the panel doesn't blank on a change.
    pub held_histogram: Option<(Box<Histogram>, Instant)>,
    requested: Option<(Settings, Options, bool)>,
    before_requested: Option<(Settings, Options)>,
    seq: u64,
    shown: [u64; 2],
    pub tool: Tool,
    pub compare: Compare,
    pub clipping: bool,
    /// The white balance eyedropper waits for a click on the photo.
    pub picker: bool,
    pub aspect: usize,
    pub portrait: bool,
    pub guide: usize,
    /// A square grid over the photo in the Adjust tool.
    pub grid: bool,
    /// The straighten slider is being dragged, so the stage shows the grid.
    pub straightening: bool,
    /// Turning, tilting, or moving corners shrinks the crop to keep the
    /// empty edges out.
    pub constrain: bool,
    pub mask: Option<usize>,
    pub mask_overlay: bool,
    pub point_curve: bool,
    pub curve_channel: usize,
    pub curve_drag: Option<usize>,
    pub mix_part: usize,
    pub preset_amount: f32,
    pub preset_name: String,
    pub export: ExportOptions,
    pub export_format: usize,
    pub drag: Option<StageDrag>,
    /// Steps changed during this session, so the full-size view is stale.
    changed: bool,
}

impl EditSession {
    fn new(id: i64, history: History) -> EditSession {
        EditSession {
            id,
            history,
            live: None,
            base: None,
            error: None,
            preview: None,
            before: None,
            histogram: None,
            held_histogram: None,
            requested: None,
            before_requested: None,
            seq: 0,
            shown: [0; 2],
            tool: Tool::Adjust,
            compare: Compare::Off,
            clipping: false,
            picker: false,
            aspect: 0,
            portrait: false,
            guide: 0,
            grid: false,
            straightening: false,
            constrain: true,
            mask: None,
            mask_overlay: true,
            point_curve: false,
            curve_channel: 0,
            curve_drag: None,
            mix_part: 0,
            preset_amount: 1.0,
            preset_name: String::new(),
            export: ExportOptions::default(),
            export_format: 0,
            drag: None,
            changed: false,
        }
    }

    /// Settings on screen: the applied steps plus any change in progress.
    pub fn current(&self) -> Settings {
        match &self.live {
            Some(live) => live.settings.clone(),
            None => self.history.settings(),
        }
    }

    fn options(&self) -> Options {
        Options {
            crop: !self.uncropped(),
            adjust: true,
            clipping: self.clipping,
            overlay: self.mask.filter(|_| self.tool == Tool::Mask && self.mask_overlay),
        }
    }

    /// Whether the tool shows the whole canvas, crop and all.
    pub fn uncropped(&self) -> bool {
        matches!(self.tool, Tool::Crop | Tool::Transform)
    }

    /// How the stage compares, after tools that need the edit alone.
    pub fn compare(&self) -> Compare {
        if self.tool == Tool::Adjust {
            self.compare
        } else {
            Compare::Off
        }
    }

    pub fn preview(&self) -> Option<&egui::TextureHandle> {
        self.preview.as_ref()
    }

    pub fn before(&self) -> Option<&egui::TextureHandle> {
        self.before.as_ref().or(self.preview.as_ref())
    }

    /// Size of the canvas crops are measured on, in pixels, at the current
    /// straighten angle.
    pub fn canvas(&self, photo: &Photo) -> [f32; 2] {
        let [w, h] = match &self.base {
            Some(base) => [base.width() as f32, base.height() as f32],
            None => [photo.width.max(1) as f32, photo.height.max(1) as f32],
        };
        edit::canvas_size(self.current().get(Control::STRAIGHTEN), w, h)
    }

    /// Width over height of the chosen crop shape in canvas pixels, or
    /// `None` when free. `turns` is the catalog rotation in quarter turns.
    pub fn aspect_ratio(&self, canvas: [f32; 2], turns: usize) -> Option<f32> {
        let (_, ratio) = ASPECTS[self.aspect.min(ASPECTS.len() - 1)];
        if ratio == 0.0 {
            return None;
        }
        let odd = turns % 2 == 1;
        let mut screen = if ratio < 0.0 {
            let own = canvas[0] / canvas[1];
            if odd {
                1.0 / own
            } else {
                own
            }
        } else {
            ratio
        };
        if self.portrait {
            screen = 1.0 / screen;
        }
        Some(if odd { 1.0 / screen } else { screen })
    }

    /// Fits the crop of `settings` again after its turn, tilt, or corners
    /// changed: the crop's shape, or the chosen aspect, as large as the
    /// canvas allows, then shrunk to keep the empty edges out while
    /// constraining. `turns` is the catalog rotation in quarter turns.
    pub fn refit_crop(&self, settings: &mut Settings, turns: usize) {
        let Some(base) = &self.base else { return };
        let canvas = edit::canvas_size(settings.get(Control::STRAIGHTEN), base.width() as f32, base.height() as f32);
        let [left, top, right, bottom] = settings.crop();
        let ratio = self
            .aspect_ratio(canvas, turns)
            .unwrap_or(((right - left) * canvas[0]) / ((bottom - top) * canvas[1]).max(1e-3));
        let widest = edit::with_aspect(edit::FULL, ratio, canvas[0], canvas[1]);
        settings.set_crop(if self.constrain {
            edit::constrain(widest, &Geometry::turn_of(settings), canvas[0], canvas[1])
        } else {
            widest
        });
    }
}

impl PicassoApp {
    /// Settings thumbnails and full-size views of `id` are rendered with.
    pub(super) fn edit_settings(&self, id: i64) -> Settings {
        self.edits.get(&id).cloned().unwrap_or_default()
    }

    /// Size of `photo` on screen, after its crop and the catalog rotation.
    /// The crop tool shows the whole canvas.
    pub(super) fn shown_size(&self, photo: &Photo) -> (f32, f32) {
        let session = self.edit.as_ref().filter(|session| session.id == photo.id);
        let mut settings = match session {
            Some(session) => session.current(),
            None => self.edit_settings(photo.id),
        };
        if session.is_some_and(EditSession::uncropped) || photo.is_video() {
            settings.set_crop(edit::FULL);
        }
        let (w, h) = edit::edited_size(&settings, photo.width.max(1), photo.height.max(1));
        if photo.rotation.rem_euclid(180) == 90 {
            (h as f32, w as f32)
        } else {
            (w as f32, h as f32)
        }
    }

    pub(super) fn toggle_editing(&mut self) {
        self.editing = !self.editing;
        if !self.editing {
            self.end_edit_session();
        }
    }

    /// Opens a session for `photo` and keeps its preview rendered with the
    /// current settings. `None` closes the session.
    pub(super) fn sync_edit(&mut self, ctx: &egui::Context, photo: Option<&Photo>) {
        let Some(photo) = photo.filter(|_| self.editing) else {
            self.end_edit_session();
            return;
        };
        if self.edit.as_ref().map(|session| session.id) != Some(photo.id) {
            let held = self.edit.as_mut().and_then(|session| {
                let until = Instant::now() + super::PANEL_HOLD;
                session.histogram.take().map(|histogram| (histogram, until)).or(session.held_histogram.take())
            });
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
            let mut session = EditSession::new(photo.id, history);
            session.held_histogram = held;
            self.edit = Some(session);
        }
        let pointer_down = ctx.input(|input| input.pointer.any_down());
        let Some(session) = self.edit.as_mut() else { return };
        let Some(base) = session.base.clone() else { return };
        let wanted = (session.current(), session.options(), session.live.is_some() && pointer_down);
        if session.requested.as_ref() != Some(&wanted) {
            session.seq += 1;
            let _ = self.edit_tx.send(EditJob::Render {
                id: session.id,
                seq: session.seq,
                base: base.clone(),
                settings: wanted.0.clone(),
                options: wanted.1,
                purpose: Purpose::Preview,
                draft: wanted.2,
            });
            session.requested = Some(wanted);
        }
        if session.compare() != Compare::Off {
            let geometry = Settings::default().with_geometry_of(&session.current());
            let options = Options {
                adjust: false,
                ..Options::default()
            };
            let wanted = (geometry, options);
            if session.before_requested.as_ref() != Some(&wanted) {
                session.seq += 1;
                let _ = self.edit_tx.send(EditJob::Render {
                    id: session.id,
                    seq: session.seq,
                    base,
                    settings: wanted.0.clone(),
                    options: wanted.1,
                    purpose: Purpose::Before,
                    draft: false,
                });
                session.before_requested = Some(wanted);
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
                session.base = Some(image);
                session.requested = None;
                session.before_requested = None;
            }
            UiEvent::EditFailed { id, error } => {
                if let Some(session) = self.edit.as_mut().filter(|session| session.id == id) {
                    session.error = Some(error);
                }
            }
            UiEvent::EditPreview {
                id,
                seq,
                purpose,
                w,
                h,
                rgba,
                histogram,
            } => {
                let Some(session) = self.edit.as_mut().filter(|session| session.id == id) else { return };
                let slot = purpose as usize;
                if seq <= session.shown[slot] || rgba.len() < w * h * 4 {
                    return;
                }
                session.shown[slot] = seq;
                session.error = None;
                if histogram.is_some() {
                    session.histogram = histogram;
                }
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba[..w * h * 4]);
                let texture = match purpose {
                    Purpose::Preview => &mut session.preview,
                    Purpose::Before => &mut session.before,
                };
                match texture.as_mut() {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => *texture = Some(ctx.load_texture(format!("edit-{id}-{slot}"), image, egui::TextureOptions::LINEAR)),
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

    pub(super) fn edit_history(&mut self, change: impl FnOnce(&mut History)) {
        let Some(session) = self.edit.as_mut() else { return };
        let before = session.history.clone();
        session.live = None;
        change(&mut session.history);
        if session.history != before {
            self.edit_changed();
        }
    }

    /// Records `settings` as a step of `kind`.
    pub(super) fn edit_commit(&mut self, kind: Kind, settings: &Settings, amend: bool) {
        let Some(session) = self.edit.as_mut() else { return };
        session.live = None;
        let changed = if amend {
            session.history.amend(kind, settings)
        } else {
            session.history.commit(kind, settings)
        };
        if changed {
            self.edit_changed();
        }
    }

    /// Turns the change in progress into a step once the pointer is up.
    pub(super) fn finish_live(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.pointer.any_down()) {
            return;
        }
        let Some(live) = self.edit.as_mut().and_then(|session| session.live.take()) else { return };
        self.edit_commit(live.kind, &live.settings, live.amend);
        ctx.request_repaint();
    }

    /// Saves the steps and refreshes everything drawn from them.
    fn edit_changed(&mut self) {
        let Some(session) = self.edit.as_mut() else { return };
        session.changed = true;
        let id = session.id;
        let history = session.history.clone();
        self.store_history(id, &history);
    }

    fn store_history(&mut self, id: i64, history: &History) {
        self.db
            .save_edit_history(id, &history.steps_json(), history.steps().len(), history.applied());
        let settings = history.settings();
        if settings.is_identity() {
            self.edits.remove(&id);
        } else {
            self.edits.insert(id, settings);
        }
        if self.edited_only {
            self.library_dirty = true;
        }
        self.forget_thumb(id);
    }

    pub(super) fn toggle_compare(&mut self, mode: Compare) {
        if let Some(session) = self.edit.as_mut() {
            session.tool = Tool::Adjust;
            let same = std::mem::discriminant(&session.compare) == std::mem::discriminant(&mode);
            session.compare = if same { Compare::Off } else { mode };
        }
    }

    pub(super) fn toggle_clipping(&mut self) {
        if let Some(session) = self.edit.as_mut() {
            session.clipping = !session.clipping;
        }
    }

    pub(super) fn set_tool(&mut self, tool: Tool) {
        if let Some(session) = self.edit.as_mut() {
            session.tool = if session.tool == tool { Tool::Adjust } else { tool };
            session.picker = false;
            session.drag = None;
            if session.tool == Tool::Mask && session.mask.is_none() && !session.current().masks.is_empty() {
                session.mask = Some(0);
            }
        }
    }

    /// Esc inside the editor: leaves a tool or the eyedropper first.
    /// Returns whether the key was used.
    pub(super) fn edit_escape(&mut self) -> bool {
        let Some(session) = self.edit.as_mut() else { return false };
        if session.picker {
            session.picker = false;
            return true;
        }
        if session.tool != Tool::Adjust {
            session.tool = Tool::Adjust;
            session.drag = None;
            return true;
        }
        if session.compare != Compare::Off {
            session.compare = Compare::Off;
            return true;
        }
        false
    }

    /// Copies the edit of the photo being edited, or of the selected photo
    /// in the library, without its crop.
    pub(super) fn copy_settings(&mut self, id: Option<i64>) {
        let settings = match (&self.edit, id) {
            (Some(session), _) => session.current(),
            (None, Some(id)) => self.edit_settings(id),
            (None, None) => return,
        };
        self.edit_clipboard = Some(settings.without_geometry());
        self.activity = tr("Edit settings copied").into();
    }

    /// Pastes copied settings into the photo being edited, or into every
    /// photo in `ids`.
    pub(super) fn paste_settings(&mut self, ids: &[i64]) {
        let Some(pasted) = self.edit_clipboard.clone() else {
            self.activity = tr("Copy an edit first (Ctrl+Shift+C)").into();
            return;
        };
        if let Some(session) = &self.edit {
            let settings = pasted.with_geometry_of(&session.current());
            self.edit_commit(Kind::Paste, &settings, false);
            return;
        }
        for &id in ids {
            let mut history = self
                .db
                .edit_history(id)
                .map(|(steps, applied)| History::from_json(&steps, applied))
                .unwrap_or_default();
            let settings = pasted.with_geometry_of(&history.settings());
            if history.commit(Kind::Paste, &settings) {
                self.store_history(id, &history);
                self.forget_display(id);
            }
        }
        self.activity = trf("Pasted the edit into {} photos", &[&ids.len()]);
    }

    pub(super) fn auto_tone(&mut self) {
        let Some(session) = self.edit.as_ref() else { return };
        let Some(base) = session.base.clone() else { return };
        let mut settings = session.current();
        for (control, value) in edit::auto_tone(&base) {
            settings.set(control, value);
        }
        self.edit_commit(Kind::AutoTone, &settings, false);
    }

    pub(super) fn white_balance_preset(&mut self, name: &str) {
        let Some(session) = self.edit.as_ref() else { return };
        let mut settings = session.current();
        let (kind, temperature, tint) = if name == "Auto" {
            let Some(base) = session.base.clone() else { return };
            let (temperature, tint) = edit::auto_white_balance(&base);
            (Kind::AutoWhiteBalance, temperature, tint)
        } else {
            let Some((_, temperature, tint)) = presets::WHITE_BALANCE.iter().find(|(preset, _, _)| *preset == name) else {
                return;
            };
            (Kind::WhiteBalance(name.to_string()), *temperature, *tint)
        };
        settings.set(Control::TEMPERATURE, temperature);
        settings.set(Control::TINT, tint);
        self.edit_commit(kind, &settings, false);
    }

    /// Neutralizes the color under a click on the canvas, given in fractions.
    pub(super) fn pick_white_balance(&mut self, at: [f32; 2]) {
        let Some(session) = self.edit.as_mut() else { return };
        session.picker = false;
        let Some(base) = session.base.clone() else { return };
        let mut settings = session.current();
        let (w, h) = (base.width() as f32, base.height() as f32);
        let [cw, ch] = edit::canvas_size(settings.get(Control::STRAIGHTEN), w, h);
        let (x, y) = Geometry::from_settings(&settings).canvas_to_source(at[0] * cw, at[1] * ch, cw, ch);
        let (temperature, tint) = edit::white_balance_for(edit::sample(&base, x / w, y / h, 3));
        settings.set(Control::TEMPERATURE, temperature);
        settings.set(Control::TINT, tint);
        self.edit_commit(Kind::Picker, &settings, false);
    }

    /// The patch of a built-in or saved preset.
    pub(super) fn preset_patch(&self, name: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
        if let Some(preset) = self.user_presets.iter().find(|preset| preset.name == name) {
            return Some(preset.patch.clone());
        }
        presets::builtin(name).map(presets::Preset::patch)
    }

    pub(super) fn apply_preset(&mut self, name: &str) {
        let Some(patch) = self.preset_patch(name) else { return };
        let Some(session) = self.edit.as_mut() else { return };
        session.preset_amount = 1.0;
        let settings = presets::apply(&session.current(), &patch, 1.0);
        self.edit_commit(Kind::Preset(name.to_string()), &settings, false);
    }

    pub(super) fn save_user_preset(&mut self, name: String) {
        let Some(session) = self.edit.as_ref() else { return };
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let preset = presets::UserPreset::new(name.clone(), &session.current());
        self.user_presets.retain(|existing| existing.name != name);
        self.user_presets.push(preset);
        self.db.set_setting("user_presets", &presets::user_presets_to_json(&self.user_presets));
        self.activity = trf("Saved preset {}", &[&name]);
    }

    pub(super) fn delete_user_preset(&mut self, index: usize) {
        if index < self.user_presets.len() {
            self.user_presets.remove(index);
            self.db.set_setting("user_presets", &presets::user_presets_to_json(&self.user_presets));
        }
    }

    pub(super) fn export_edit(&mut self, ctx: &egui::Context) {
        let Some(session) = self.edit.as_ref() else { return };
        let Some(photo) = self.db.photo(session.id) else { return };
        let settings = session.current();
        let options = session.export;
        let (format, extension) = EXPORT_FORMATS[session.export_format.min(EXPORT_FORMATS.len() - 1)];
        let source = PathBuf::from(&photo.path);
        let folder = PathBuf::from(photo.path.trim_start_matches(r"\\?\"));
        let name = PathBuf::from(edit::export_name(&source)).with_extension(extension);
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr("Export edited photo"))
            .set_file_name(name.to_string_lossy())
            .add_filter(format, &[extension]);
        if let Some(dir) = folder.parent() {
            dialog = dialog.set_directory(dir);
        }
        let Some(target) = dialog.save_file() else { return };
        self.activity = trf("Exporting {}…", &[&crate::types::folder_label(&target.to_string_lossy())]);
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let rotation = photo.rotation;
        let _ = std::thread::Builder::new().name("picasso-export".into()).spawn(move || {
            let result = edit::export(&source, &target, &settings, rotation, options).map(|()| target);
            let _ = events.send(UiEvent::Exported(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn forget_thumb(&mut self, id: i64) {
        self.thumbs.map.remove(&id);
        self.thumb_failed.remove(&id);
    }

    pub(super) fn forget_display(&mut self, id: i64) {
        self.displays.retain(|item| item.id != id);
        self.display_inflight.remove(&id);
        self.display_failed.remove(&id);
        if self.sharp == Some(id) {
            self.sharp = None;
        }
    }
}
