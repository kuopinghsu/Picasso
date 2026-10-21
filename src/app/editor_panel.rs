//! Edit panel beside the stage: tools, histogram, history, and the develop
//! sections. Widgets change a draft of the settings; the draft becomes the
//! live change shown on the stage and a step once the pointer is released.

use eframe::egui::{self, Color32, Pos2, RichText, Sense, Stroke, Vec2};
use serde_json::{Map, Value};

use crate::edit::{
    self, presets, Control, Kind, Mask, Settings, Shape, Unit, BANDS, CURVE_CHANNELS, LOCALS, MIX_PARTS, PROFILES, ZONES,
};
use crate::i18n::{tr, trf};
use crate::photo_info::Histogram;
use crate::types::Photo;

use super::editor::{Compare, EditSession, Live, Tool, ASPECTS, EXPORT_EDGES, EXPORT_FORMATS, GUIDES};
use super::theme::{self, ACCENT, VIEWER_MUTED, VIEWER_PANEL, WHITE};
use super::PicassoApp;

const PANEL_W: f32 = 276.0;
/// Height kept for the export controls under the sections.
const EXPORT_H: f32 = 96.0;

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
        let frame = egui::Frame::new().fill(VIEWER_PANEL).inner_margin(egui::Margin::same(12));
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
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Edit")).size(16.0).strong().color(WHITE));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tr("Done")).clicked() {
                actions.push(Action::Done);
            }
        });
    });
    ui.add_space(4.0);
    if context.photo.is_some_and(Photo::is_video) {
        ui.label(RichText::new(tr("Videos can't be edited")).color(VIEWER_MUTED));
        return;
    }
    let Some(session) = session else {
        ui.label(RichText::new(tr("Loading…")).color(VIEWER_MUTED));
        return;
    };
    if let Some(error) = &session.error {
        ui.label(RichText::new(error).color(theme::DANGER));
    }
    if session.base.is_none() {
        if session.error.is_none() {
            ui.label(RichText::new(tr("Loading…")).color(VIEWER_MUTED));
        }
        return;
    }

    let tools = [
        (Tool::Adjust, "Adjust", "Light, color, effects, and detail"),
        (Tool::Crop, "Crop", "Crop & straighten (C)"),
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
    match &session.histogram {
        Some(histogram) => super::viewer::paint_histogram(ui, histogram),
        None => {
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 112.0), Sense::hover());
        }
    }

    let history = &session.history;
    let row = [
        (history.can_undo(), "Undo all", "Back to the original"),
        (history.can_undo(), "Undo", "Undo (Ctrl+Z)"),
        (history.can_redo(), "Redo", "Redo (Ctrl+Y)"),
        (history.can_redo(), "Redo all", "Apply every step again"),
    ];
    let picked = button_row(
        ui,
        &row.map(|(enabled, label, hint)| Item {
            label,
            hint,
            enabled,
            selected: false,
        }),
    );
    match picked {
        Some(0) => actions.push(Action::UndoAll),
        Some(1) => actions.push(Action::Undo),
        Some(2) => actions.push(Action::Redo),
        Some(3) => actions.push(Action::RedoAll),
        _ => {}
    }

    let compare = session.compare;
    let row = [
        ("Before", "Show the original (\\)", compare == Compare::Before),
        ("Side by side", "Before and after side by side (Y)", compare == Compare::SideBySide),
        ("Split", "Before and after split view (Shift+Y)", matches!(compare, Compare::Split(_))),
        ("Clipping", "Show clipped highlights and shadows (J)", session.clipping),
    ];
    let picked = button_row(
        ui,
        &row.map(|(label, hint, selected)| Item {
            label,
            hint,
            enabled: true,
            selected,
        }),
    );
    match picked {
        Some(0) => actions.push(Action::Compare(Compare::Before)),
        Some(1) => actions.push(Action::Compare(Compare::SideBySide)),
        Some(2) => actions.push(Action::Compare(Compare::Split(0.5))),
        Some(3) => actions.push(Action::Clipping),
        _ => {}
    }

    let edited = !history.settings().is_identity();
    let row = [
        ("Auto", "Auto tone: balance exposure and contrast", true),
        ("Reset all", "Back to the untouched photo, as a step", edited),
        ("Copy", "Copy these settings (Ctrl+Shift+C)", true),
        ("Paste", "Paste copied settings (Ctrl+Shift+V)", context.has_clipboard),
    ];
    let picked = button_row(
        ui,
        &row.map(|(label, hint, enabled)| Item {
            label,
            hint,
            enabled,
            selected: false,
        }),
    );
    match picked {
        Some(0) => actions.push(Action::AutoTone),
        Some(1) => actions.push(Action::Commit(Kind::Reset, Settings::default())),
        Some(2) => actions.push(Action::Copy),
        Some(3) => actions.push(Action::Paste),
        _ => {}
    }
    ui.add_space(4.0);

    let mut draft = Draft {
        settings: session.current(),
        kind: None,
        amend: false,
    };
    let used = ui.cursor().top() - top;
    egui::ScrollArea::vertical()
        .id_salt("edit-panel")
        .auto_shrink([false, false])
        .max_height((height - used - EXPORT_H).max(100.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            match session.tool {
                Tool::Adjust => adjust_sections(ui, session, &mut draft, context, actions),
                Tool::Crop => crop_section(ui, session, &mut draft, context, actions),
                Tool::Mask => mask_section(ui, session, &mut draft),
            }
        });
    ui.add_space(6.0);
    export_section(ui, session, actions);

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
    egui::CollapsingHeader::new(RichText::new(tr(title)).strong().size(12.5).color(WHITE))
        .id_salt(title)
        .default_open(open)
        .show(ui, add);
}

fn subheading(ui: &mut egui::Ui, title: &'static str) {
    ui.add_space(4.0);
    ui.label(RichText::new(tr(title).to_uppercase()).size(10.5).strong().color(VIEWER_MUTED));
}

/// Slider with its name and value above it. Double-clicking the name or
/// the slider puts it back to neutral.
fn slider(ui: &mut egui::Ui, draft: &mut Draft, control: Control, kind: Kind) {
    slider_labeled(ui, draft, control, kind, control.def().label, None);
}

fn slider_labeled(ui: &mut egui::Ui, draft: &mut Draft, control: Control, kind: Kind, label: &'static str, swatch: Option<Color32>) {
    let def = control.def();
    let mut value = draft.settings.get(control);
    let mut reset = false;
    ui.horizontal(|ui| {
        if let Some(color) = swatch {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.5, color);
        }
        let name = egui::Label::new(RichText::new(tr(label)).size(12.0).color(WHITE)).sense(Sense::click());
        if ui.add(name).on_hover_text(tr("Double-click to reset")).double_clicked() {
            reset = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let color = if value != control.neutral() { ACCENT } else { VIEWER_MUTED };
            ui.label(RichText::new(control.format(value)).size(11.5).color(color));
        });
    });
    ui.spacing_mut().slider_width = ui.available_width();
    let step = match def.unit {
        Unit::Ev => 0.01,
        Unit::Radius | Unit::Degrees => 0.1,
        _ => 1.0,
    };
    let response = ui.add(egui::Slider::new(&mut value, def.min..=def.max).show_value(false).step_by(step));
    if reset || response.double_clicked() {
        value = control.neutral();
    }
    if reset || response.changed() || response.double_clicked() {
        draft.settings.set(control, value);
        draft.changed(kind);
    }
}

fn adjust_sections(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, context: &Context, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Profile")).size(12.0).color(WHITE));
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

    section(ui, "Light", true, |ui| {
        for control in Control::LIGHT {
            slider(ui, draft, control, Kind::Adjust);
        }
    });

    section(ui, "Tone curve", false, |ui| tone_curve_section(ui, session, draft));

    section(ui, "Color", true, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("White balance")).size(12.0).color(WHITE));
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
            let color = if index > applied { VIEWER_MUTED.gamma_multiply(0.7) } else { WHITE };
            if ui.selectable_label(index == applied, RichText::new(text).size(12.0).color(color)).clicked() {
                actions.push(Action::Jump(index));
            }
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr("Edits are saved as steps in the library. The original file is never changed."))
                .size(11.0)
                .color(VIEWER_MUTED),
        );
    });
}

fn swatch(hue: f32) -> Color32 {
    let [r, g, b] = edit::hue_rgb(hue).map(|c| (c * 0.85 * 255.0) as u8);
    Color32::from_rgb(r, g, b)
}

fn presets_section(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft, context: &Context, actions: &mut Vec<Action>) {
    let last = context.last_preset.as_ref().map(|(name, _)| name.as_str());
    if let Some((name, patch)) = &context.last_preset {
        ui.horizontal(|ui| {
            ui.label(RichText::new(presets::display_name(name)).size(12.0).color(ACCENT));
        });
        let mut percent = session.preset_amount * 100.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Amount")).size(12.0).color(WHITE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{percent:.0}%")).size(11.5).color(VIEWER_MUTED));
            });
        });
        ui.spacing_mut().slider_width = ui.available_width();
        let response = ui.add(egui::Slider::new(&mut percent, 0.0..=200.0).show_value(false).step_by(1.0));
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
        ui.label(RichText::new(tr("Save the current settings as a preset to reuse them.")).size(11.0).color(VIEWER_MUTED));
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
            ui.label(RichText::new(tr("Click to add a point, drag to move, double-click to remove.")).size(10.5).color(VIEWER_MUTED));
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
    painter.rect_filled(rect, 2, Color32::from_gray(0x24));
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
        let fill = Color32::from_white_alpha(22);
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
    let grid = Stroke::new(1.0, Color32::from_white_alpha(20));
    for quarter in 1..4 {
        let t = quarter as f32 / 4.0;
        painter.line_segment([to_screen([t, 0.0]), to_screen([t, 1.0])], grid);
        painter.line_segment([to_screen([0.0, t]), to_screen([1.0, t])], grid);
    }
    painter.line_segment([to_screen([0.0, 0.0]), to_screen([1.0, 1.0])], Stroke::new(1.0, Color32::from_white_alpha(30)));

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
                Stroke::new(1.0, Color32::from_white_alpha(45)),
            );
        }
        let amounts = [
            n(Control::CURVE_SHADOWS),
            n(Control::CURVE_DARKS),
            n(Control::CURVE_LIGHTS),
            n(Control::CURVE_HIGHLIGHTS),
        ];
        curve_line(&edit::parametric_points(amounts, splits), WHITE);
        return;
    }

    let channel = session.curve_channel;
    let colors = [
        WHITE,
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
                ui.label(RichText::new(tr(ZONES[zone])).size(11.0).color(VIEWER_MUTED));
                wheel(ui, draft, zone, size);
            });
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - size) * 0.5).max(0.0));
        ui.vertical(|ui| {
            ui.set_width(size);
            ui.label(RichText::new(tr(ZONES[3])).size(11.0).color(VIEWER_MUTED));
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
    let mut reshape = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Aspect")).size(12.0).color(WHITE));
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
    let angle = draft.settings.get(Control::STRAIGHTEN);
    if reshape {
        if let Some(ratio) = session.aspect_ratio(canvas, turns) {
            let crop = edit::with_aspect(draft.settings.crop(), ratio, canvas[0], canvas[1]);
            draft.settings.set_crop(edit::constrain(crop, angle, canvas[0], canvas[1]));
            draft.changed(Kind::Crop);
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Overlay")).size(12.0).color(WHITE));
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
    slider(ui, draft, Control::STRAIGHTEN, Kind::Crop);
    let new_angle = draft.settings.get(Control::STRAIGHTEN);
    if new_angle != angle {
        let [left, top, right, bottom] = draft.settings.crop();
        let ratio = ((right - left) * canvas[0]) / ((bottom - top) * canvas[1]).max(1e-3);
        let widest = edit::with_aspect(edit::FULL, ratio, canvas[0], canvas[1]);
        draft.settings.set_crop(edit::constrain(widest, new_angle, canvas[0], canvas[1]));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (control, label) in [(Control::FLIP_H, "Flip horizontal"), (Control::FLIP_V, "Flip vertical")] {
            let on = draft.settings.get(control) >= 0.5;
            if ui.selectable_label(on, tr(label)).clicked() {
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
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let geometry = Settings::default().with_geometry_of(&draft.settings);
        if ui.add_enabled(!geometry.is_identity(), egui::Button::new(tr("Reset crop"))).clicked() {
            draft.settings = draft.settings.with_geometry_of(&Settings::default());
            draft.changed(Kind::Crop);
        }
        if ui.add(theme::primary_button(tr("Done"))).clicked() {
            actions.push(Action::Tool(Tool::Adjust));
        }
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new(tr("Drag the corners or edges to crop, and inside the frame to move it. Double-click or press Enter to finish."))
            .size(11.0)
            .color(VIEWER_MUTED),
    );
}

fn mask_section(ui: &mut egui::Ui, session: &mut EditSession, draft: &mut Draft) {
    let [left, top, right, bottom] = draft.settings.crop();
    let (cx, cy) = ((left + right) * 0.5, (top + bottom) * 0.5);
    let canvas = session.base.as_ref().map_or([1.0, 1.0], |base| [base.width() as f32, base.height() as f32]);
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
                .color(VIEWER_MUTED),
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
        ui.label(RichText::new(tr("Feather")).size(12.0).color(WHITE));
        ui.spacing_mut().slider_width = ui.available_width();
        if ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).show_value(false)).changed() {
            *feather = percent / 100.0;
            changed = true;
        }
    }
    for (local, name) in LOCALS.iter().enumerate() {
        let (min, max) = Mask::local_range(local);
        let mut value = mask.amounts[local];
        let mut reset = false;
        ui.horizontal(|ui| {
            let label = egui::Label::new(RichText::new(tr(name)).size(12.0).color(WHITE)).sense(Sense::click());
            if ui.add(label).on_hover_text(tr("Double-click to reset")).double_clicked() {
                reset = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = if local == 0 { format!("{value:+.2} EV") } else { format!("{value:+.0}") };
                let color = if value != 0.0 { ACCENT } else { VIEWER_MUTED };
                ui.label(RichText::new(text).size(11.5).color(color));
            });
        });
        ui.spacing_mut().slider_width = ui.available_width();
        let step = if local == 0 { 0.01 } else { 1.0 };
        let response = ui.add(egui::Slider::new(&mut value, min..=max).show_value(false).step_by(step));
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
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Quality")).size(12.0).color(WHITE));
            ui.spacing_mut().slider_width = (ui.available_width() - 40.0).max(60.0);
            ui.add(egui::Slider::new(&mut session.export.quality, 50..=100));
        });
    }
    let export = ui
        .add_sized([ui.available_width(), 30.0], theme::primary_button(tr("Export…")))
        .on_hover_text(tr("Save the edited photo as a new file"));
    if export.clicked() {
        actions.push(Action::Export);
    }
}
