//! Edit panel beside the stage: tools, histogram, history, and the develop
//! sections. Widgets change a draft of the settings; the draft becomes the
//! live change shown on the stage and a step once the pointer is released.

use std::ops::RangeInclusive;

use eframe::egui::{self, Color32, Pos2, RichText, Sense, Stroke, Vec2};
use serde_json::{Map, Value};

use crate::edit::{
    self, presets, Control, Geometry, Kind, Mask, Settings, Shape, Unit, BANDS, CURVE_CHANNELS, HDR_METHODS, LOCALS, MIX_PARTS, PROFILES, ZONES,
};
use crate::i18n::{tr, trf};
use crate::photo_info::Histogram;
use crate::types::Photo;

use super::editor::{Compare, EditSession, Live, Tool, ASPECTS, EXPORT_EDGES, EXPORT_FORMATS, GUIDES};
use super::theme::{self, Icon, WHITE};
use super::PicassoApp;

const PANEL_W: f32 = 276.0;
/// Height kept for the export controls under the sections until they have
/// been drawn once and measured.
const EXPORT_H: f32 = 136.0;

/// HSL hue of each color band's swatch.
const BAND_SWATCH: [f32; 8] = [0.0, 30.0, 55.0, 120.0, 180.0, 220.0, 275.0, 315.0];

enum Action {
    Undo,
    Redo,
    UndoAll,
    RedoAll,
    Jump(usize),
    Export,
    Done,
    AutoTone,
    WhiteBalance(&'static str),
    Copy,
    Paste,
    Preset(String),
    SavePreset(String),
    DeletePreset(usize),
    Rotate(i32),
    Tool(Tool),
    Compare(Compare),
    Clipping,
    Commit(Kind, Settings),
}

/// Settings as the widgets of one frame left them, and the kind of step
/// the change makes.
struct Draft {
    settings: Settings,
    kind: Option<Kind>,
    amend: bool,
    /// Values Auto tone or Auto white balance set, badged while they stay.
    auto: Vec<(Control, f32)>,
}

impl Draft {
    fn changed(&mut self, kind: Kind) {
        self.kind = Some(kind);
        self.amend = false;
    }
}

/// What the panel reads from the app besides the session.
struct Context<'a> {
    user_presets: &'a [String],
    /// Name and patch of the preset the newest step applied.
    last_preset: Option<(String, Map<String, Value>)>,
    has_clipboard: bool,
    photo: Option<&'a Photo>,
}

impl PicassoApp {
    pub(super) fn edit_panel(&mut self, ui: &mut egui::Ui, height: f32, photo: Option<&Photo>) {
        let mut actions = Vec::new();
        let names: Vec<String> = self.user_presets.iter().map(|preset| preset.name.clone()).collect();
        let last_preset = self
            .edit
            .as_ref()
            .and_then(|session| session.history.last_preset().map(str::to_string))
            .and_then(|name| self.preset_patch(&name).map(|patch| (name, patch)));
        let context = Context {
            user_presets: &names,
            last_preset,
            has_clipboard: self.edit_clipboard.is_some(),
            photo,
        };
        let frame = egui::Frame::new().fill(theme::sidebar()).inner_margin(egui::Margin::same(12));
        frame.show(ui, |ui| {
            ui.set_min_size(Vec2::new(PANEL_W, height - 24.0));
            ui.set_max_width(PANEL_W);
            // The viewer row lays out left to right; the panel stacks downward.
            let layout = egui::Layout::top_down(egui::Align::Min);
            ui.with_layout(layout, |ui| panel_contents(ui, self.edit.as_mut(), height - 24.0, &context, &mut actions));
        });
        let ctx = ui.ctx().clone();
        for action in actions {
            match action {
                Action::Undo => self.edit_history(edit::History::undo),
                Action::Redo => self.edit_history(edit::History::redo),
                Action::UndoAll => self.edit_history(edit::History::undo_all),
                Action::RedoAll => self.edit_history(edit::History::redo_all),
                Action::Jump(count) => self.edit_history(|history| history.jump(count)),
                Action::Export => self.export_edit(&ctx),
                Action::Done => self.toggle_editing(),
                Action::AutoTone => self.auto_tone(),
                Action::WhiteBalance(name) => self.white_balance_preset(name),
                Action::Copy => self.copy_settings(None),
                Action::Paste => self.paste_settings(&[]),
                Action::Preset(name) => self.apply_preset(&name),
                Action::SavePreset(name) => self.save_user_preset(name),
                Action::DeletePreset(index) => self.delete_user_preset(index),
                Action::Rotate(degrees) => self.rotate_current(degrees),
                Action::Tool(tool) => self.set_tool(tool),
                Action::Compare(mode) => self.toggle_compare(mode),
                Action::Clipping => self.toggle_clipping(),
                Action::Commit(kind, settings) => self.edit_commit(kind, &settings, false),
            }
        }
        self.finish_live(&ctx);
    }
}

fn panel_contents(ui: &mut egui::Ui, session: Option<&mut EditSession>, height: f32, context: &Context, actions: &mut Vec<Action>) {
    let top = ui.cursor().top();
    let history = session.as_ref().filter(|session| session.error.is_none()).map(|session| &session.history);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Edit")).size(16.0).strong().color(theme::text()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tr("Done")).clicked() {
                actions.push(Action::Done);
            }
            let Some(history) = history else { return };
            history_menu(ui, history, context, actions);
            let (can_undo, can_redo) = (history.can_undo(), history.can_redo());
            if theme::icon_button(ui, Icon::Redo, can_redo, false).on_hover_text(tr("Redo (Ctrl+Y)")).clicked() {
                actions.push(Action::Redo);
            }
            if theme::icon_button(ui, Icon::Undo, can_undo, false).on_hover_text(tr("Undo (Ctrl+Z)")).clicked() {
                actions.push(Action::Undo);
            }
        });
    });
    ui.add_space(4.0);
    if context.photo.is_some_and(Photo::is_video) {
        ui.label(RichText::new(tr("Videos can't be edited")).color(theme::muted()));
        return;
    }
    let Some(session) = session else {
        ui.label(RichText::new(tr("Loading…")).color(theme::muted()));
        return;
    };
    if let Some(error) = &session.error {
        ui.label(RichText::new(error).color(theme::DANGER));
    }
    // Until the photo is decoded the controls already show its saved edit,
    // so the panel looks the same while the next photo loads; only Auto,
    // the eyedropper, and the histogram wait for the pixels.
    if session.error.is_some() && session.base.is_none() {
        return;
    }

    let tools = [
        (Tool::Adjust, "Adjust", "Light, color, effects, and detail"),
        (Tool::Crop, "Crop", "Crop & straighten (C)"),
        (Tool::Transform, "Transform", "Drag four corners to turn, tilt, and skew (T)"),
        (Tool::Mask, "Masks", "Linear and radial local adjustments"),
    ];
    let picked = button_row(
        ui,
        &tools.map(|(tool, label, hint)| Item {
            label,
            hint,
            enabled: true,
            selected: session.tool == tool,
        }),
    );
    if let Some(index) = picked {
        actions.push(Action::Tool(tools[index].0));
    }
    ui.add_space(4.0);
    let now = std::time::Instant::now();
    let held = session.held_histogram.as_ref().filter(|(_, until)| now < *until);
    if let (None, Some((_, until))) = (&session.histogram, held) {
        ui.ctx().request_repaint_after(*until - now);
    }
    match session.histogram.as_ref().or(held.map(|(histogram, _)| histogram)) {
        Some(histogram) => super::viewer::paint_histogram(ui, histogram),
        None => super::viewer::paint_histogram_placeholder(ui, false),
    }

    ui.add_space(2.0);
    let compare = session.compare;
    let row = [
        ("Before", "Show the original (\\)", compare == Compare::Before),
        ("Side by side", "Before and after side by side (Y)", compare == Compare::SideBySide),
        ("Split", "Before and after split view (Shift+Y)", matches!(compare, Compare::Split(_))),
    ];
    let row_w = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(row_w, 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let clipping = theme::icon_button(ui, Icon::Clipping, true, session.clipping);
        if clipping.on_hover_text(tr("Show clipped highlights and shadows (J)")).clicked() {
            actions.push(Action::Clipping);
        }
        let items = row.map(|(label, hint, selected)| Item {
            label,
            hint,
            enabled: true,
            selected,
        });
        match segmented(ui, &items, ui.available_width()) {
            Some(0) => actions.push(Action::Compare(Compare::Before)),
            Some(1) => actions.push(Action::Compare(Compare::SideBySide)),
            Some(2) => actions.push(Action::Compare(Compare::Split(0.5))),
            _ => {}
        }
    });
    ui.add_space(4.0);

    let mut draft = Draft {
        settings: session.current(),
        kind: None,
        amend: false,
        auto: session.history.auto_values(),
    };
    session.straightening = false;
    let used = ui.cursor().top() - top;
    // The export controls' height as last drawn, so the sections take what
    // is left and the panel keeps to the stage's height. Taller, it would
    // push the filmstrip down, and back up while the next photo loads.
    let tail_id = egui::Id::unique("edit-panel-tail");
    let tail_h = ui.ctx().data(|data| data.get_temp::<f32>(tail_id)).unwrap_or(EXPORT_H);
    egui::ScrollArea::vertical()
        .id_salt("edit-panel")
        .auto_shrink([false, false])
        .max_height((height - used - tail_h).max(100.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            match session.tool {
                Tool::Adjust => adjust_sections(ui, session, &mut draft, context, actions),
                Tool::Crop => crop_section(ui, session, &mut draft, context, actions),
                Tool::Transform => corners_section(ui, session, &mut draft, context, actions),
                Tool::Mask => mask_section(ui, session, &mut draft),
            }
        });
    let tail_top = ui.cursor().top();
    ui.add_space(6.0);
    export_section(ui, session, actions);
    let measured = ui.cursor().top() - tail_top;
    if (measured - tail_h).abs() > 0.5 {
        ui.ctx().data_mut(|data| data.insert_temp(tail_id, measured));
        ui.ctx().request_repaint();
    }

    if let Some(kind) = draft.kind.take() {
        if draft.settings != session.current() {
            session.live = Some(Live {
                kind,
                settings: draft.settings,
                amend: draft.amend,
            });
        }
    }
}

struct Item {
    label: &'static str,
    hint: &'static str,
    enabled: bool,
    selected: bool,
}

/// The ··· button beside undo and redo: the steps furthest back and
/// forward, and the settings as a whole.
fn history_menu(ui: &mut egui::Ui, history: &edit::History, context: &Context, actions: &mut Vec<Action>) {
    let button = theme::icon_button(ui, Icon::More, true, false);
    let edited = !history.settings().is_identity();
    let popup = egui::Popup::menu(&button).show(|ui| {
        let mut item = |enabled: bool, label: &'static str, hint: &'static str| ui.add_enabled(enabled, egui::Button::new(tr(label))).on_hover_text(tr(hint)).clicked();
        if item(history.can_undo(), "Undo all", "Back to the original") {
            actions.push(Action::UndoAll);
        }
        if item(history.can_redo(), "Redo all", "Apply every step again") {
            actions.push(Action::RedoAll);
        }
        if item(edited, "Reset all", "Back to the untouched photo, as a step") {
            actions.push(Action::Commit(Kind::Reset, Settings::default()));
        }
        if item(true, "Copy", "Copy these settings (Ctrl+Shift+C)") {
            actions.push(Action::Copy);
        }
        if item(context.has_clipboard, "Paste", "Paste copied settings (Ctrl+Shift+V)") {
            actions.push(Action::Paste);
        }
    });
    if popup.is_none() {
        button.on_hover_text(tr("More"));
    }
}

/// Options side by side in one rounded track, the picked one raised on it.
/// Returns the clicked one; clicking the picked one again is how a compare
/// mode is turned off.
fn segmented(ui: &mut egui::Ui, items: &[Item], width: f32) -> Option<usize> {
    let height = 26.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, theme::RADIUS + 1, theme::well());
    let part_w = (width - 4.0) / items.len() as f32;
    let raised = if theme::is_dark() { Color32::from_rgb(0x35, 0x3F, 0x42) } else { theme::bg() };
    let mut clicked = None;
    for (index, item) in items.iter().enumerate() {
        let part = egui::Rect::from_min_size(rect.min + Vec2::new(2.0 + part_w * index as f32, 2.0), Vec2::new(part_w, height - 4.0));
        let response = ui.interact(part, egui::Id::unique(("segmented", item.label)), Sense::click());
        if item.selected {
            painter.rect_filled(part.translate(Vec2::new(0.0, 0.5)), theme::RADIUS - 1, Color32::from_black_alpha(24));
            painter.rect_filled(part, theme::RADIUS - 1, raised);
        } else if response.hovered() {
            painter.rect_filled(part, theme::RADIUS - 1, theme::overlay(14));
        }
        let color = if item.selected { theme::text() } else { theme::muted() };
        let mut job = egui::text::LayoutJob::simple_singleline(tr(item.label).to_string(), egui::FontId::proportional(12.0), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(part_w - 6.0);
        let galley = painter.layout_job(job);
        painter.galley(part.center() - galley.size() / 2.0, galley, color);
        if response.on_hover_text(tr(item.hint)).clicked() {
            clicked = Some(index);
        }
    }
    clicked
}

/// Equal-width buttons across the panel. Returns the clicked one.
fn button_row(ui: &mut egui::Ui, items: &[Item]) -> Option<usize> {
    let gap = 4.0;
    let count = items.len() as f32;
    let width = ((ui.available_width() - gap * (count - 1.0)) / count).max(30.0);
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.spacing_mut().button_padding.x = 2.0;
        for (index, item) in items.iter().enumerate() {
            let text = RichText::new(tr(item.label)).size(12.0);
            let button = egui::Button::new(text).selected(item.selected).min_size(Vec2::new(width, 24.0));
            if ui.add_enabled(item.enabled, button).on_hover_text(tr(item.hint)).clicked() {
                clicked = Some(index);
            }
        }
    });
    clicked
}

fn section(ui: &mut egui::Ui, title: &'static str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(tr(title)).strong().size(12.5).color(theme::text()))
        .id_salt(title)
        .default_open(open)
        .show(ui, add);
}

fn subheading(ui: &mut egui::Ui, title: &'static str) {
    ui.add_space(4.0);
    ui.label(RichText::new(tr(title).to_uppercase()).size(10.5).strong().color(theme::muted()));
}

/// Slider with its name and value above it. Double-clicking the name or
/// the slider puts it back to neutral.
fn slider(ui: &mut egui::Ui, draft: &mut Draft, control: Control, kind: Kind) -> egui::Response {
    slider_labeled(ui, draft, control, kind, control.def().label, None)
}

fn slider_labeled(
    ui: &mut egui::Ui,
    draft: &mut Draft,
    control: Control,
    kind: Kind,
    label: &'static str,
    swatch: Option<Color32>,
) -> egui::Response {
    let def = control.def();
    let mut value = draft.settings.get(control);
    let neutral = control.neutral();
    let auto = draft.auto.iter().any(|(other, set)| *other == control && *set == value);
    let reset = slider_header(ui, tr(label), swatch, auto, &control.format(value), value != neutral);
    let step = match def.unit {
        Unit::Ev => 0.01,
        Unit::Radius | Unit::Degrees => 0.1,
        _ => 1.0,
    };
    let response = develop_slider(ui, &mut value, def.min..=def.max, step, neutral);
    if reset || response.double_clicked() {
        value = control.neutral();
    }
    if reset || response.changed() || response.double_clicked() {
        draft.settings.set(control, value);
        draft.changed(kind);
    }
    response
}

/// Bold name, an "auto" badge when Auto tone or Auto white balance set the
/// value, and the value at the right. True when the name is double-clicked.
fn slider_header(ui: &mut egui::Ui, label: &str, swatch: Option<Color32>, auto: bool, value: &str, changed: bool) -> bool {
    let mut reset = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(color) = swatch {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.5, color);
        }
        let name = egui::Label::new(RichText::new(label).size(12.5).strong().color(theme::text())).sense(Sense::click());
        if ui.add(name).on_hover_text(tr("Double-click to reset")).double_clicked() {
            reset = true;
        }
        if auto {
            egui::Frame::new()
                .fill(badge_fill())
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(5, 1))
                .show(ui, |ui| ui.label(RichText::new(tr("Auto").to_lowercase()).size(10.0).color(theme::muted())));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let color = if changed { theme::text() } else { theme::muted() };
            ui.label(RichText::new(value).monospace().size(11.5).color(color));
        });
    });
    reset
}

const RAIL_HEIGHT: f32 = 5.0;
const SLIDER_HEIGHT: f32 = 20.0;

/// Thick rail filled in the accent color from `anchor` to the value, with a
/// round knob. egui's slider underneath, left invisible, handles the pointer
/// and the arrow keys.
fn develop_slider(ui: &mut egui::Ui, value: &mut f32, range: RangeInclusive<f32>, step: f64, anchor: f32) -> egui::Response {
    let (min, max) = (*range.start(), *range.end());
    let width = ui.available_width();
    let response = ui
        .scope(|ui| {
            let spacing = ui.spacing_mut();
            spacing.slider_width = width;
            spacing.interact_size.y = SLIDER_HEIGHT;
            let widgets = &mut ui.visuals_mut().widgets;
            for state in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
                state.bg_fill = Color32::TRANSPARENT;
                state.fg_stroke = Stroke::NONE;
            }
            ui.add(egui::Slider::new(value, range).show_value(false).trailing_fill(false).step_by(step))
        })
        .inner;

    let rect = response.rect;
    // egui keeps the knob's center this far from each end.
    let radius = rect.height() / 2.5;
    let x_at = |at: f32| {
        let t = if max > min { ((at - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
        egui::lerp(rect.left() + radius..=rect.right() - radius, t)
    };
    let y = rect.center().y;
    let rail = egui::Rect::from_x_y_ranges(rect.x_range(), y - RAIL_HEIGHT / 2.0..=y + RAIL_HEIGHT / 2.0);
    let painter = ui.painter();
    painter.rect_filled(rail, RAIL_HEIGHT / 2.0, rail_fill());

    let knob = x_at(*value);
    let start = if anchor <= min {
        rail.left()
    } else if anchor >= max {
        rail.right()
    } else {
        x_at(anchor)
    };
    if (knob - start).abs() > 0.5 {
        let fill = egui::Rect::from_x_y_ranges(start.min(knob)..=start.max(knob), rail.y_range());
        painter.rect_filled(fill, RAIL_HEIGHT / 2.0, theme::accent());
    }
    let grow = if response.dragged() || response.hovered() { 1.0 } else { 0.0 };
    let center = egui::pos2(knob, y);
    painter.circle_filled(center + Vec2::new(0.0, 1.0), radius + grow, Color32::from_black_alpha(50));
    painter.circle(center, radius + grow, knob_fill(), Stroke::new(1.0, theme::hairline()));
    response
}

fn badge_fill() -> Color32 {
    if theme::is_dark() {
        Color32::from_rgb(0x3C, 0x45, 0x48)
    } else {
        Color32::from_rgb(0xE3, 0xE1, 0xDB)
    }
}

fn rail_fill() -> Color32 {
    if theme::is_dark() {
        Color32::from_rgb(0x44, 0x4E, 0x52)
    } else {
        Color32::from_rgb(0xD5, 0xD3, 0xCC)
    }
}

fn knob_fill() -> Color32 {
    if theme::is_dark() {
        Color32::from_gray(200)
    } else {
        WHITE
    }
}

fn adjust_sections(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, context: &Context, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Profile")).size(12.0).color(theme::text()));
        let mut profile = draft.settings.get(Control::PROFILE).round() as usize;
        egui::ComboBox::from_id_salt("edit-profile")
            .width(150.0)
            .selected_text(tr(PROFILES[profile.min(PROFILES.len() - 1)]))
            .show_ui(ui, |ui| {
                for (index, name) in PROFILES.iter().enumerate() {
                    if ui.selectable_value(&mut profile, index, tr(name)).clicked() {
                        draft.settings.set(Control::PROFILE, index as f32);
                        draft.changed(Kind::Profile);
                    }
                }
            });
    });

    section(ui, "Presets", false, |ui| presets_section(ui, session, draft, context, actions));

    section(ui, "Transform", false, |ui| transform_section(ui, session, draft, context, actions));

    section(ui, "Light", true, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(tr("Auto tone")).on_hover_text(tr("Auto tone: balance exposure and contrast")).clicked() {
                    actions.push(Action::AutoTone);
                }
            });
        });
        for control in Control::LIGHT {
            slider(ui, draft, control, Kind::Adjust);
        }
    });

    section(ui, "Tone curve", false, |ui| tone_curve_section(ui, session, draft));

    section(ui, "HDR", false, |ui| hdr_section(ui, draft));

    section(ui, "Color", true, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("White balance")).size(12.0).color(theme::text()));
            egui::ComboBox::from_id_salt("edit-wb")
                .width(110.0)
                .selected_text(tr("Choose…"))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(false, tr("Auto")).clicked() {
                        actions.push(Action::WhiteBalance("Auto"));
                    }
                    for (name, _, _) in presets::WHITE_BALANCE {
                        if ui.selectable_label(false, tr(name)).clicked() {
                            actions.push(Action::WhiteBalance(name));
                        }
                    }
                });
            let picker = ui
                .selectable_label(session.picker, "⊙")
                .on_hover_text(tr("White balance picker: click something that should be neutral gray"));
            if picker.clicked() {
                session.picker = !session.picker;
            }
        });
        slider(ui, draft, Control::TEMPERATURE, Kind::Adjust);
        slider(ui, draft, Control::TINT, Kind::Adjust);
        subheading(ui, "Presence");
        slider(ui, draft, Control::VIBRANCE, Kind::Adjust);
        slider(ui, draft, Control::SATURATION, Kind::Adjust);
    });

    section(ui, "Color mixer", false, |ui| {
        ui.horizontal(|ui| {
            for (index, part) in MIX_PARTS.iter().enumerate() {
                ui.selectable_value(&mut session.mix_part, index, tr(part));
            }
        });
        for band in 0..BANDS.len() {
            let swatch = swatch(BAND_SWATCH[band]);
            slider_labeled(ui, draft, Control::mix(session.mix_part, band), Kind::Adjust, BANDS[band], Some(swatch));
        }
    });

    section(ui, "Color grading", false, |ui| grading_section(ui, draft));

    section(ui, "Effects", false, |ui| {
        slider(ui, draft, Control::TEXTURE, Kind::Adjust);
        slider(ui, draft, Control::CLARITY, Kind::Adjust);
        slider(ui, draft, Control::DEHAZE, Kind::Adjust);
        subheading(ui, "Vignette");
        for control in [
            Control::VIGNETTE_AMOUNT,
            Control::VIGNETTE_MIDPOINT,
            Control::VIGNETTE_ROUNDNESS,
            Control::VIGNETTE_FEATHER,
            Control::VIGNETTE_HIGHLIGHTS,
        ] {
            slider(ui, draft, control, Kind::Adjust);
        }
        subheading(ui, "Grain");
        for control in [Control::GRAIN_AMOUNT, Control::GRAIN_SIZE, Control::GRAIN_ROUGHNESS] {
            slider(ui, draft, control, Kind::Adjust);
        }
    });

    section(ui, "Detail", false, |ui| {
        subheading(ui, "Sharpening");
        for control in [
            Control::SHARPEN_AMOUNT,
            Control::SHARPEN_RADIUS,
            Control::SHARPEN_DETAIL,
            Control::SHARPEN_MASKING,
        ] {
            slider(ui, draft, control, Kind::Adjust);
        }
        subheading(ui, "Noise reduction");
        slider(ui, draft, Control::NOISE_LUMINANCE, Kind::Adjust);
        slider(ui, draft, Control::NOISE_COLOR, Kind::Adjust);
    });

    section(ui, "Black & White", false, |ui| {
        let mut mono = draft.settings.get(Control::MONOCHROME) >= 0.5;
        if ui.checkbox(&mut mono, tr("Convert to black & white")).changed() {
            draft.settings.set(Control::MONOCHROME, if mono { 1.0 } else { 0.0 });
            draft.changed(Kind::Adjust);
        }
        ui.add_enabled_ui(mono, |ui| {
            for band in 0..BANDS.len() {
                slider_labeled(ui, draft, Control::bw(band), Kind::Adjust, BANDS[band], Some(swatch(BAND_SWATCH[band])));
            }
        });
    });

    section(ui, "History", true, |ui| {
        let history = &session.history;
        let applied = history.applied();
        let rows = std::iter::once(tr("Original").to_string()).chain(history.steps().iter().map(edit::Step::label));
        for (index, text) in rows.enumerate() {
            let color = if index > applied { theme::muted().gamma_multiply(0.7) } else { theme::text() };
            if ui.selectable_label(index == applied, RichText::new(text).size(12.0).color(color)).clicked() {
                actions.push(Action::Jump(index));
            }
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr("Edits are saved as steps in the library. The original file is never changed."))
                .size(11.0)
                .color(theme::muted()),
        );
    });
}

/// What each HDR method does to a photo, under its picker.
const HDR_NOTES: [&str; 5] = [
    "Evens out light and shadow by area and brings out detail, the classic HDR look.",
    "Blends darker and brighter copies of the photo, keeping each area from the best exposed one. Natural, with open shadows.",
    "A gentle curve that rolls off bright highlights and lifts shadows.",
    "A logarithmic curve that opens up deep shadows the most.",
    "The film-like curve from cinema, with rich contrast and soft highlights.",
];

fn hdr_section(ui: &mut egui::Ui, draft: &mut Draft) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Method")).size(12.0).color(theme::text()));
        let mut method = (draft.settings.get(Control::HDR_METHOD).round().max(0.0) as usize).min(HDR_METHODS.len() - 1);
        egui::ComboBox::from_id_salt("edit-hdr-method")
            .width(170.0)
            .selected_text(tr(HDR_METHODS[method]))
            .show_ui(ui, |ui| {
                for (index, name) in HDR_METHODS.iter().enumerate() {
                    if ui.selectable_value(&mut method, index, tr(name)).clicked() {
                        draft.settings.set(Control::HDR_METHOD, index as f32);
                        draft.changed(Kind::Adjust);
                    }
                }
            });
    });
    let method = (draft.settings.get(Control::HDR_METHOD).round().max(0.0) as usize).min(HDR_NOTES.len() - 1);
    ui.label(RichText::new(tr(HDR_NOTES[method])).size(11.0).color(theme::muted()));
    slider(ui, draft, Control::HDR_STRENGTH, Kind::Adjust);
    slider(ui, draft, Control::HDR_DETAIL, Kind::Adjust);
}

fn swatch(hue: f32) -> Color32 {
    let [r, g, b] = edit::hue_rgb(hue).map(|c| (c * 0.85 * 255.0) as u8);
    Color32::from_rgb(r, g, b)
}

fn presets_section(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, context: &Context, actions: &mut Vec<Action>) {
    let last = context.last_preset.as_ref().map(|(name, _)| name.as_str());
    if let Some((name, patch)) = &context.last_preset {
        ui.horizontal(|ui| {
            ui.label(RichText::new(presets::display_name(name)).size(12.0).color(theme::accent()));
        });
        let mut percent = session.preset_amount * 100.0;
        slider_header(ui, tr("Amount"), None, false, &format!("{percent:.0}%"), true);
        let response = develop_slider(ui, &mut percent, 0.0..=200.0, 1.0, 0.0);
        if response.changed() {
            session.preset_amount = percent / 100.0;
            let before = session.history.settings_at(session.history.applied().saturating_sub(1));
            draft.settings = presets::apply(&before, patch, session.preset_amount);
            draft.kind = Some(Kind::Preset(name.clone()));
            draft.amend = true;
        }
        ui.add_space(4.0);
    }
    for group in presets::GROUPS {
        subheading(ui, group);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for preset in presets::BUILTIN.iter().filter(|preset| preset.group == group) {
                let text = RichText::new(tr(preset.name)).size(11.5);
                if ui.selectable_label(last == Some(preset.name), text).clicked() {
                    actions.push(Action::Preset(preset.name.to_string()));
                }
            }
        });
    }
    subheading(ui, "My presets");
    if context.user_presets.is_empty() {
        ui.label(RichText::new(tr("Save the current settings as a preset to reuse them.")).size(11.0).color(theme::muted()));
    }
    for (index, name) in context.user_presets.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.selectable_label(last == Some(name.as_str()), RichText::new(name).size(12.0)).clicked() {
                actions.push(Action::Preset(name.clone()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").on_hover_text(tr("Delete preset")).clicked() {
                    actions.push(Action::DeletePreset(index));
                }
            });
        });
    }
    ui.horizontal(|ui| {
        let field = egui::TextEdit::singleline(&mut session.preset_name)
            .hint_text(tr("Preset name"))
            .desired_width(ui.available_width() - 60.0);
        let response = ui.add(field);
        let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let save = ui.add_enabled(!session.preset_name.trim().is_empty(), egui::Button::new(tr("Save")));
        if save.clicked() || (enter && !session.preset_name.trim().is_empty()) {
            actions.push(Action::SavePreset(std::mem::take(&mut session.preset_name)));
        }
    });
}

fn tone_curve_section(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut session.point_curve, false, tr("Parametric"));
        ui.selectable_value(&mut session.point_curve, true, tr("Point"));
    });
    if session.point_curve {
        ui.horizontal(|ui| {
            for (index, channel) in CURVE_CHANNELS.iter().enumerate() {
                ui.selectable_value(&mut session.curve_channel, index, tr(channel));
            }
        });
    }
    curve_graph(ui, session, draft);
    if session.point_curve {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Click to add a point, drag to move, double-click to remove.")).size(10.5).color(theme::muted()));
        });
        let channel = session.curve_channel;
        if ui
            .add_enabled(!draft.settings.curves[channel].is_identity(), egui::Button::new(tr("Reset curve")))
            .clicked()
        {
            draft.settings.curves[channel] = edit::Curve::default();
            draft.changed(Kind::Curve);
        }
    } else {
        for control in [
            Control::CURVE_HIGHLIGHTS,
            Control::CURVE_LIGHTS,
            Control::CURVE_DARKS,
            Control::CURVE_SHADOWS,
        ] {
            slider(ui, draft, control, Kind::Curve);
        }
        subheading(ui, "Splits");
        for control in [Control::SPLIT_SHADOWS, Control::SPLIT_MIDTONES, Control::SPLIT_HIGHLIGHTS] {
            slider(ui, draft, control, Kind::Curve);
        }
    }
}

fn curve_graph(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft) {
    let width = ui.available_width();
    let sense = if session.point_curve { Sense::click_and_drag() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, width * 0.8), sense);
    let painter = ui.painter_at(rect.expand(5.0));
    painter.rect_filled(rect, 2, theme::well());
    let to_screen = |p: [f32; 2]| Pos2::new(rect.left() + p[0] * rect.width(), rect.bottom() - p[1] * rect.height());
    let from_screen = |pos: Pos2| {
        [
            ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
            ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0),
        ]
    };
    if let Some(histogram) = &session.histogram {
        let bins = &histogram.bins[Histogram::LUMA];
        let peak = histogram.peak() as f32;
        let mut mesh = egui::Mesh::default();
        let fill = theme::overlay(22);
        for (bin, &count) in bins.iter().enumerate() {
            let x = bin as f32 / 255.0;
            let index = mesh.vertices.len() as u32;
            mesh.colored_vertex(to_screen([x, 0.0]), fill);
            mesh.colored_vertex(to_screen([x, (count as f32 / peak).min(1.0)]), fill);
            if bin > 0 {
                mesh.add_triangle(index - 2, index - 1, index);
                mesh.add_triangle(index - 1, index, index + 1);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    let grid = Stroke::new(1.0, theme::overlay(20));
    for quarter in 1..4 {
        let t = quarter as f32 / 4.0;
        painter.line_segment([to_screen([t, 0.0]), to_screen([t, 1.0])], grid);
        painter.line_segment([to_screen([0.0, t]), to_screen([1.0, t])], grid);
    }
    painter.line_segment([to_screen([0.0, 0.0]), to_screen([1.0, 1.0])], Stroke::new(1.0, theme::overlay(30)));

    let curve_line = |points: &[[f32; 2]], color: Color32| {
        let table = edit::sample_curve(points, 129);
        let line: Vec<Pos2> = table
            .iter()
            .enumerate()
            .map(|(index, y)| to_screen([index as f32 / 128.0, *y]))
            .collect();
        painter.add(egui::Shape::line(line, Stroke::new(1.6, color)));
        table
    };
    let n = |control: Control| draft.settings.get(control) / 100.0;
    let splits = [n(Control::SPLIT_SHADOWS), n(Control::SPLIT_MIDTONES), n(Control::SPLIT_HIGHLIGHTS)];
    if !session.point_curve {
        for split in splits {
            painter.line_segment(
                [to_screen([split, 0.0]), to_screen([split, 1.0])],
                Stroke::new(1.0, theme::overlay(45)),
            );
        }
        let amounts = [
            n(Control::CURVE_SHADOWS),
            n(Control::CURVE_DARKS),
            n(Control::CURVE_LIGHTS),
            n(Control::CURVE_HIGHLIGHTS),
        ];
        curve_line(&edit::parametric_points(amounts, splits), theme::text());
        return;
    }

    let channel = session.curve_channel;
    let colors = [
        theme::text(),
        Color32::from_rgb(0xFF, 0x60, 0x58),
        Color32::from_rgb(0x5C, 0xD6, 0x5C),
        Color32::from_rgb(0x5A, 0x9C, 0xFF),
    ];
    for other in (0..4).filter(|other| *other != channel) {
        if !draft.settings.curves[other].is_identity() {
            curve_line(&draft.settings.curves[other].0, colors[other].gamma_multiply(0.35));
        }
    }
    let table = curve_line(&draft.settings.curves[channel].0, colors[channel]);
    let points = &mut draft.settings.curves[channel].0;
    let nearest = |points: &[[f32; 2]], pos: Pos2| {
        points
            .iter()
            .enumerate()
            .map(|(index, point)| (index, to_screen(*point).distance(pos)))
            .filter(|(_, distance)| *distance < 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    };
    let insert = |points: &mut Vec<[f32; 2]>, point: [f32; 2]| {
        let index = points.partition_point(|existing| existing[0] < point[0]);
        let crowded = points.iter().any(|existing| (existing[0] - point[0]).abs() < 0.02);
        if crowded || index == 0 || index >= points.len() {
            return None;
        }
        points.insert(index, point);
        Some(index)
    };
    let mut changed = false;
    if response.double_clicked() {
        if let Some(index) = response.interact_pointer_pos().and_then(|pos| nearest(points, pos)) {
            if index > 0 && index + 1 < points.len() {
                points.remove(index);
                changed = true;
            }
        }
    } else if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            if nearest(points, pos).is_none() {
                let x = from_screen(pos)[0];
                changed |= insert(points, [x, lookup(&table, x)]).is_some();
            }
        }
    }
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            session.curve_drag = nearest(points, pos).or_else(|| insert(points, from_screen(pos)));
        }
    }
    if response.dragged() {
        if let (Some(index), Some(pos)) = (session.curve_drag, response.interact_pointer_pos()) {
            if index < points.len() {
                let [x, y] = from_screen(pos);
                let last = points.len() - 1;
                let x = if index == 0 {
                    0.0
                } else if index == last {
                    1.0
                } else {
                    x.clamp(points[index - 1][0] + 0.01, points[index + 1][0] - 0.01)
                };
                points[index] = [x, y];
                changed = true;
            }
        }
    }
    if response.drag_stopped() {
        session.curve_drag = None;
    }
    for (index, point) in points.iter().enumerate() {
        let active = session.curve_drag == Some(index);
        painter.circle_filled(to_screen(*point), if active { 5.0 } else { 4.0 }, colors[channel]);
        painter.circle_stroke(to_screen(*point), 4.5, Stroke::new(1.0, Color32::BLACK));
    }
    if changed {
        draft.changed(Kind::Curve);
    }
}

fn lookup(table: &[f32], x: f32) -> f32 {
    let last = table.len() - 1;
    let position = x.clamp(0.0, 1.0) * last as f32;
    let index = (position as usize).min(last - 1);
    table[index] + (table[index + 1] - table[index]) * (position - index as f32)
}

fn grading_section(ui: &mut egui::Ui, draft: &mut Draft) {
    let gap = 6.0;
    let size = ((ui.available_width() - gap * 2.0) / 3.0).clamp(50.0, 86.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for zone in 0..3 {
            ui.vertical(|ui| {
                ui.set_width(size);
                ui.label(RichText::new(tr(ZONES[zone])).size(11.0).color(theme::muted()));
                wheel(ui, draft, zone, size);
            });
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - size) * 0.5).max(0.0));
        ui.vertical(|ui| {
            ui.set_width(size);
            ui.label(RichText::new(tr(ZONES[3])).size(11.0).color(theme::muted()));
            wheel(ui, draft, 3, size);
        });
    });
    subheading(ui, "Luminance");
    for zone in 0..4 {
        slider_labeled(ui, draft, Control::grade(zone, 2), Kind::Adjust, ZONES[zone], None);
    }
    slider(ui, draft, Control::GRADE_BLENDING, Kind::Adjust);
    slider(ui, draft, Control::GRADE_BALANCE, Kind::Adjust);
}

/// Hue and saturation of a grading zone as a point on a color wheel.
fn wheel(ui: &mut egui::Ui, draft: &mut Draft, zone: usize, size: f32) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    let center = rect.center();
    let radius = size * 0.5 - 3.0;
    let painter = ui.painter();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, Color32::from_gray(120));
    let segments = 60;
    for step in 0..=segments {
        let hue = step as f32 / segments as f32 * 360.0;
        let angle = hue.to_radians();
        let [r, g, b] = edit::hue_rgb(hue).map(|c| ((0.47 + (c - 0.47) * 0.8) * 255.0) as u8);
        mesh.colored_vertex(center + Vec2::new(angle.cos(), -angle.sin()) * radius, Color32::from_rgb(r, g, b));
        if step > 0 {
            mesh.add_triangle(0, step, step + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_stroke(center, radius, Stroke::new(1.0, Color32::from_black_alpha(160)));
    let hue_control = Control::grade(zone, 0);
    let saturation_control = Control::grade(zone, 1);
    let hue = draft.settings.get(hue_control);
    let saturation = draft.settings.get(saturation_control) / 100.0;
    let knob = center + Vec2::new(hue.to_radians().cos(), -hue.to_radians().sin()) * saturation * radius;
    painter.circle_filled(knob, 5.0, WHITE);
    painter.circle_stroke(knob, 5.0, Stroke::new(1.5, Color32::BLACK));
    let response = response.on_hover_text(trf(
        "Hue {}°  Saturation {}\nDouble-click to reset",
        &[&format!("{hue:.0}"), &format!("{:.0}", saturation * 100.0)],
    ));
    if response.double_clicked() {
        draft.settings.set(hue_control, 0.0);
        draft.settings.set(saturation_control, 0.0);
        draft.changed(Kind::Adjust);
    } else if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let offset = pos - center;
            let amount = (offset.length() / radius).min(1.0) * 100.0;
            let angle = (-offset.y).atan2(offset.x).to_degrees().rem_euclid(360.0);
            draft.settings.set(hue_control, angle.round());
            draft.settings.set(saturation_control, amount.round());
            draft.changed(Kind::Adjust);
        }
    }
}

fn crop_section(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, context: &Context, actions: &mut Vec<Action>) {
    let Some(photo) = context.photo else { return };
    let canvas = session.canvas(photo);
    let turns = photo.rotation.div_euclid(90).rem_euclid(4) as usize;
    let before = Geometry::turn_of(&draft.settings);
    aspect_row(ui, session, draft, canvas, turns);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Overlay")).size(12.0).color(theme::text()));
        egui::ComboBox::from_id_salt("edit-guide")
            .width(130.0)
            .selected_text(tr(GUIDES[session.guide]))
            .show_ui(ui, |ui| {
                for (index, name) in GUIDES.iter().enumerate() {
                    ui.selectable_value(&mut session.guide, index, tr(name));
                }
            });
    });
    ui.add_space(4.0);
    straighten_slider(ui, session, draft);
    ui.add_space(4.0);
    flip_and_rotate(ui, draft, actions);
    let mut refit = constrain_checkbox(ui, session);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let mut reset = draft.settings.clone();
        for control in [Control::STRAIGHTEN, Control::FLIP_H, Control::FLIP_V] {
            reset.set(control, control.neutral());
        }
        reset.set_crop(edit::FULL);
        if ui.add_enabled(reset != draft.settings, egui::Button::new(tr("Reset crop"))).clicked() {
            draft.settings = reset;
            draft.changed(Kind::Crop);
            refit = true;
        }
        if ui.add(theme::primary_button(tr("Done"))).clicked() {
            actions.push(Action::Tool(Tool::Adjust));
        }
    });
    if refit || Geometry::turn_of(&draft.settings) != before {
        session.refit_crop(&mut draft.settings, turns);
        draft.changed(Kind::Crop);
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(tr("Drag the corners or edges to crop, and inside the frame to move it. Double-click or press Enter to finish."))
            .size(11.0)
            .color(theme::muted()),
    );
}

/// The crop shape, with a button that swaps portrait and landscape. A new
/// shape takes the largest crop of it inside the current one.
fn aspect_row(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, canvas: [f32; 2], turns: usize) {
    let mut reshape = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Aspect")).size(12.0).color(theme::text()));
        egui::ComboBox::from_id_salt("edit-aspect")
            .width(100.0)
            .selected_text(tr(ASPECTS[session.aspect].0))
            .show_ui(ui, |ui| {
                for (index, (name, _)) in ASPECTS.iter().enumerate() {
                    if ui.selectable_value(&mut session.aspect, index, tr(name)).clicked() {
                        reshape = true;
                    }
                }
            });
        let swap = ui
            .add_enabled(session.aspect != 0, egui::Button::new("⇄"))
            .on_hover_text(tr("Swap portrait and landscape"));
        if swap.clicked() {
            session.portrait = !session.portrait;
            reshape = true;
        }
    });
    if reshape {
        if let Some(ratio) = session.aspect_ratio(canvas, turns) {
            let mut crop = edit::with_aspect(draft.settings.crop(), ratio, canvas[0], canvas[1]);
            if session.constrain {
                crop = edit::constrain(crop, &Geometry::turn_of(&draft.settings), canvas[0], canvas[1]);
            }
            draft.settings.set_crop(crop);
            draft.changed(Kind::Crop);
        }
    }
}

/// Returns true when constraining was just turned on, so the crop needs
/// fitting again.
fn constrain_checkbox(ui: &mut egui::Ui, session: &mut EditSession) -> bool {
    ui.checkbox(&mut session.constrain, tr("Constrain crop"))
        .on_hover_text(tr("Crop away the empty edges that turning, perspective, or moved corners leave"))
        .changed()
        && session.constrain
}

/// Straighten, perspective, flips, 90° turns, and the grid, without leaving
/// Adjust, and the way to the corner handles and the crop frame.
fn transform_section(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, context: &Context, actions: &mut Vec<Action>) {
    let Some(photo) = context.photo else { return };
    let canvas = session.canvas(photo);
    let turns = photo.rotation.div_euclid(90).rem_euclid(4) as usize;
    transform_controls(ui, session, draft, turns, actions);
    ui.checkbox(&mut session.grid, tr("Show grid"))
        .on_hover_text(tr("Lines over the photo to line up horizons and verticals"));
    aspect_row(ui, session, draft, canvas, turns);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if ui.button(tr("Corners…")).on_hover_text(tr("Drag four corners to turn, tilt, and skew (T)")).clicked() {
            actions.push(Action::Tool(Tool::Transform));
        }
        if ui.button(tr("Crop…")).on_hover_text(tr("Crop & straighten (C)")).clicked() {
            actions.push(Action::Tool(Tool::Crop));
        }
        reset_transform_button(ui, session, draft, turns);
    });
}

/// The Transform tool: the stage shows four corner handles.
fn corners_section(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, context: &Context, actions: &mut Vec<Action>) {
    let Some(photo) = context.photo else { return };
    let turns = photo.rotation.div_euclid(90).rem_euclid(4) as usize;
    ui.label(
        RichText::new(tr(
            "Drag a corner to move it, the middle of a side to slide or skew that side, inside to shift the photo, and outside to turn it.",
        ))
        .size(11.0)
        .color(theme::muted()),
    );
    ui.add_space(4.0);
    transform_controls(ui, session, draft, turns, actions);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let moved = (0..4).any(|corner| (0..2).any(|axis| draft.settings.get(Control::corner(corner, axis)) != 0.0));
        if ui.add_enabled(moved, egui::Button::new(tr("Reset corners"))).clicked() {
            for corner in 0..4 {
                for axis in 0..2 {
                    draft.settings.set(Control::corner(corner, axis), 0.0);
                }
            }
            session.refit_crop(&mut draft.settings, turns);
            draft.changed(Kind::Crop);
        }
        reset_transform_button(ui, session, draft, turns);
    });
    if ui.add(theme::primary_button(tr("Done"))).clicked() {
        actions.push(Action::Tool(Tool::Adjust));
    }
}

/// Straighten, perspective, flips, 90° turns, and constraining the crop.
/// A new turn or tilt fits the crop again.
fn transform_controls(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, turns: usize, actions: &mut Vec<Action>) {
    let before = Geometry::turn_of(&draft.settings);
    straighten_slider(ui, session, draft);
    subheading(ui, "Perspective");
    for (axis, hint) in [
        (0, "Positive widens the top, for buildings that lean back"),
        (1, "Positive widens the right side"),
    ] {
        if perspective_slider(ui, draft, axis, turns).on_hover_text(tr(hint)).dragged() {
            session.straightening = true;
        }
    }
    ui.add_space(4.0);
    flip_and_rotate(ui, draft, actions);
    let refit = constrain_checkbox(ui, session);
    if refit || Geometry::turn_of(&draft.settings) != before {
        session.refit_crop(&mut draft.settings, turns);
        draft.changed(Kind::Crop);
    }
}

/// Puts straighten, perspective, corners, flips, and crop back to none.
fn reset_transform_button(ui: &mut egui::Ui, session: &EditSession, draft: &mut Draft, turns: usize) {
    let reset = draft.settings.with_geometry_of(&Settings::default());
    if ui.add_enabled(reset != draft.settings, egui::Button::new(tr("Reset transform"))).clicked() {
        draft.settings = reset;
        session.refit_crop(&mut draft.settings, turns);
        draft.changed(Kind::Crop);
    }
}

/// Which tilt a perspective slider moves, and which way, so that the
/// vertical one works up and down on screen after the catalog rotation.
/// `axis` is 0 for vertical and 1 for horizontal.
fn perspective_control(axis: usize, turns: usize) -> (Control, f32) {
    let (vertical, horizontal) = (Control::PERSPECTIVE_V, Control::PERSPECTIVE_H);
    match (turns % 4, axis) {
        (0, 0) => (vertical, 1.0),
        (0, _) => (horizontal, 1.0),
        (1, 0) => (horizontal, -1.0),
        (1, _) => (vertical, 1.0),
        (2, 0) => (vertical, -1.0),
        (2, _) => (horizontal, -1.0),
        (3, 0) => (horizontal, 1.0),
        _ => (vertical, -1.0),
    }
}

fn perspective_slider(ui: &mut egui::Ui, draft: &mut Draft, axis: usize, turns: usize) -> egui::Response {
    let (control, sign) = perspective_control(axis, turns);
    let mut value = draft.settings.get(control) * sign;
    let label = tr(["Vertical", "Horizontal"][axis]);
    let reset = slider_header(ui, label, None, false, &control.format(value), value != 0.0);
    let response = develop_slider(ui, &mut value, -45.0..=45.0, 0.1, 0.0);
    if reset || response.double_clicked() {
        value = 0.0;
    }
    if reset || response.changed() || response.double_clicked() {
        draft.settings.set(control, value * sign);
        draft.changed(Kind::Crop);
    }
    response
}

/// The straighten slider, from −180° to 180°, and a box for an exact angle.
fn straighten_slider(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft) {
    let response = slider(ui, draft, Control::STRAIGHTEN, Kind::Crop);
    let mut dragging = response.dragged();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Angle")).size(12.0).color(theme::text()));
        let mut value = draft.settings.get(Control::STRAIGHTEN);
        let exact = ui
            .add(egui::DragValue::new(&mut value).range(-180.0..=180.0).speed(0.1).fixed_decimals(1).suffix("°"))
            .on_hover_text(tr("Type an angle, or drag for fine steps"));
        if exact.changed() {
            draft.settings.set(Control::STRAIGHTEN, value);
            draft.changed(Kind::Crop);
        }
        dragging |= exact.dragged();
    });
    session.straightening |= dragging;
}

fn flip_and_rotate(ui: &mut egui::Ui, draft: &mut Draft, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        for (control, label) in [(Control::FLIP_H, "Flip horizontal"), (Control::FLIP_V, "Flip vertical")] {
            let on = draft.settings.get(control) >= 0.5;
            if ui.add(egui::Button::new(tr(label)).selected(on)).clicked() {
                draft.settings.set(control, if on { 0.0 } else { 1.0 });
                draft.changed(Kind::Crop);
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button(tr("Rotate Left")).clicked() {
            actions.push(Action::Rotate(-90));
        }
        if ui.button(tr("Rotate Right")).clicked() {
            actions.push(Action::Rotate(90));
        }
    });
}

fn mask_section(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft) {
    let [left, top, right, bottom] = draft.settings.crop();
    let (cx, cy) = ((left + right) * 0.5, (top + bottom) * 0.5);
    let canvas = session.base.as_ref().map_or([1.0, 1.0], |base| {
        edit::canvas_size(draft.settings.get(Control::STRAIGHTEN), base.width() as f32, base.height() as f32)
    });
    ui.horizontal(|ui| {
        let count = draft.settings.masks.len() + 1;
        if ui.button(tr("+ Linear gradient")).clicked() {
            draft.settings.masks.push(Mask {
                name: trf("Linear {}", &[&count]),
                shape: Shape::Linear {
                    from: [cx, top + (bottom - top) * 0.15],
                    to: [cx, top + (bottom - top) * 0.55],
                },
                invert: false,
                amounts: [0.0; 9],
            });
            session.mask = Some(draft.settings.masks.len() - 1);
            draft.changed(Kind::Mask);
        }
        if ui.button(tr("+ Radial gradient")).clicked() {
            let size = ((right - left) * canvas[0]).min((bottom - top) * canvas[1]) * 0.3;
            draft.settings.masks.push(Mask {
                name: trf("Radial {}", &[&count]),
                shape: Shape::Radial {
                    center: [cx, cy],
                    radius: [size / canvas[0], size / canvas[1]],
                    feather: 0.5,
                },
                invert: false,
                amounts: [0.0; 9],
            });
            session.mask = Some(draft.settings.masks.len() - 1);
            draft.changed(Kind::Mask);
        }
    });
    if draft.settings.masks.is_empty() {
        ui.label(
            RichText::new(tr("Add a gradient, then drag its handles on the photo and set what it changes."))
                .size(11.0)
                .color(theme::muted()),
        );
        return;
    }
    ui.checkbox(&mut session.mask_overlay, tr("Show the selected mask in red"));
    let mut remove = None;
    for (index, mask) in draft.settings.masks.iter().enumerate() {
        ui.horizontal(|ui| {
            let selected = session.mask == Some(index);
            if ui.selectable_label(selected, RichText::new(&mask.name).size(12.0)).clicked() {
                session.mask = Some(index);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").on_hover_text(tr("Delete mask")).clicked() {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        draft.settings.masks.remove(index);
        session.mask = match session.mask {
            Some(selected) if selected > index => Some(selected - 1),
            Some(selected) if selected == index => None,
            other => other,
        }
        .filter(|selected| *selected < draft.settings.masks.len());
        draft.changed(Kind::Mask);
    }
    let Some(index) = session.mask.filter(|index| *index < draft.settings.masks.len()) else { return };
    ui.add_space(4.0);
    let mask = &mut draft.settings.masks[index];
    let mut changed = false;
    changed |= ui.checkbox(&mut mask.invert, tr("Invert")).changed();
    if let Shape::Radial { feather, .. } = &mut mask.shape {
        let mut percent = *feather * 100.0;
        slider_header(ui, tr("Feather"), None, false, &format!("{percent:.0}"), true);
        if develop_slider(ui, &mut percent, 0.0..=100.0, 1.0, 0.0).changed() {
            *feather = percent / 100.0;
            changed = true;
        }
    }
    for (local, name) in LOCALS.iter().enumerate() {
        let (min, max) = Mask::local_range(local);
        let mut value = mask.amounts[local];
        let text = if local == 0 { format!("{value:+.2} EV") } else { format!("{value:+.0}") };
        let reset = slider_header(ui, tr(name), None, false, &text, value != 0.0);
        let step = if local == 0 { 0.01 } else { 1.0 };
        let response = develop_slider(ui, &mut value, min..=max, step, 0.0);
        if reset || response.double_clicked() {
            value = 0.0;
        }
        if reset || response.changed() || response.double_clicked() {
            mask.amounts[local] = value;
            changed = true;
        }
    }
    if changed {
        draft.changed(Kind::Mask);
    }
}

fn export_section(ui: &mut egui::Ui, session: &mut EditSession, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("export-format")
            .width(70.0)
            .selected_text(EXPORT_FORMATS[session.export_format].0)
            .show_ui(ui, |ui| {
                for (index, (name, _)) in EXPORT_FORMATS.iter().enumerate() {
                    ui.selectable_value(&mut session.export_format, index, *name);
                }
            });
        let edge_label = |edge: Option<u32>| match edge {
            None => tr("Full size").to_string(),
            Some(edge) => trf("{} px", &[&edge]),
        };
        egui::ComboBox::from_id_salt("export-edge")
            .width(90.0)
            .selected_text(edge_label(session.export.long_edge))
            .show_ui(ui, |ui| {
                for edge in EXPORT_EDGES {
                    ui.selectable_value(&mut session.export.long_edge, edge, edge_label(edge));
                }
            })
            .response
            .on_hover_text(tr("Long edge of the exported file"));
    });
    if session.export_format == 0 {
        let mut quality = f32::from(session.export.quality);
        slider_header(ui, tr("Quality"), None, false, &format!("{quality:.0}"), true);
        develop_slider(ui, &mut quality, 50.0..=100.0, 1.0, 50.0);
        session.export.quality = quality.round().clamp(50.0, 100.0) as u8;
    }
    let export = ui
        .add_sized([ui.available_width(), 30.0], theme::primary_button(tr("Export…")))
        .on_hover_text(tr("Save the edited photo as a new file"));
    if export.clicked() {
        actions.push(Action::Export);
    }
}
