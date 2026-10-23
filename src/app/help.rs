//! Help & Shortcuts dialog, opened with F1, `?`, or the help button in the
//! header and the viewer toolbar.

use std::path::Path;

use eframe::egui::{self, Color32, FontId, Key, Margin, Modifiers, RichText, Sense, Stroke, Vec2};

use crate::i18n::{tr, trf};

use super::{about, theme};

/// Inputs that do the same thing, and what they do.
type Shortcut = (&'static [&'static str], &'static str);
/// Heading and rows of one part of the app.
type Section = (&'static str, &'static [Shortcut]);

const KEYBOARD: &[Section] = &[
    (
        "General",
        &[
            (&["F1", "?"], "Open or close help"),
            (&["Ctrl+,"], "Open settings"),
            (&["Esc"], "Close a dialog or go back"),
            (&["F5"], "Rescan folders"),
            (&["F11"], "Fullscreen"),
            (&["Ctrl+Shift+C"], "Copy edit settings"),
            (&["Ctrl+Shift+V"], "Paste settings"),
        ],
    ),
    (
        "Library",
        &[
            (&["← → ↑ ↓"], "Move the selection"),
            (&["Ctrl+A"], "Select all photos"),
            (&["Enter"], "Open the selected photo"),
            (&["E"], "Edit the selected photo"),
            (&["S"], "Add/Remove Star"),
            (&["R", "Ctrl+]"], "Rotate Clockwise"),
            (&["Shift+R", "Ctrl+["], "Rotate Counterclockwise"),
            (&["I"], "Info and map panel"),
            (&["Delete"], "Move to trash"),
        ],
    ),
    (
        "Viewer",
        &[
            (&["←", "→"], "Previous / next photo"),
            (&["Home", "End"], "First / last photo"),
            (&["+", "−"], "Zoom in / out"),
            (&["0"], "Fit"),
            (&["1"], "Actual size"),
            (&["Space"], "Start or pause the slideshow"),
            (&["I"], "Details, histogram, and EXIF"),
            (&["Esc"], "Back to the library"),
        ],
    ),
    ("Video", &[(&["Space"], "Play or pause"), (&["M"], "Mute or unmute")]),
    (
        "Editor",
        &[
            (&["E"], "Open or close the editor"),
            (&["Ctrl+Z"], "Undo"),
            (&["Ctrl+Y", "Ctrl+Shift+Z"], "Redo"),
            (&["\\"], "Show the original"),
            (&["Y"], "Before and after side by side"),
            (&["Shift+Y"], "Before and after split view"),
            (&["J"], "Show clipped highlights and shadows"),
            (&["C"], "Crop & straighten"),
            (&["Enter"], "Finish cropping"),
            (&["Esc"], "Leave the tool or close the editor"),
        ],
    ),
];

/// Mouse inputs are translated; a `Ctrl+` or `Shift+` prefix is kept as is.
const MOUSE: &[Section] = &[
    (
        "Library",
        &[
            (&["Click"], "Select a photo"),
            (&["Ctrl+Click"], "Add or remove from the selection"),
            (&["Shift+Click"], "Select a range"),
            (&["Double-click"], "Open the photo"),
            (&["Right-click"], "Photo, folder, and album menu"),
            (&["Drag & drop"], "Add a folder to the library"),
        ],
    ),
    (
        "Viewer",
        &[
            (&["Wheel", "Pinch"], "Zoom at the pointer"),
            (&["Drag"], "Pan a zoomed photo"),
            (&["Double-click"], "Switch between fit and actual size"),
            (&["Click"], "Play or pause a video"),
        ],
    ),
    (
        "Map",
        &[
            (&["Drag"], "Move the map"),
            (&["Wheel"], "Zoom the map"),
            (&["Click"], "Select the photo on a pin"),
            (&["Double-click"], "Open the photo on a pin"),
        ],
    ),
    (
        "Editor",
        &[
            (&["Double-click"], "Reset a slider"),
            (&["Drag"], "Move or resize the crop"),
            (&["Double-click"], "Finish cropping"),
            (&["Click", "Drag"], "Add or move a curve point"),
            (&["Double-click"], "Remove a curve point"),
            (&["Drag"], "Move or reshape a mask"),
        ],
    ),
];

const KEYS: f32 = 170.0;
const ACTION: f32 = 220.0;
const GAP: f32 = 12.0;
const PANEL_MARGIN: i8 = 14;
/// Outer width of one panel, including its margin and outline.
const PANEL: f32 = KEYS + GAP + ACTION + 2.0 * PANEL_MARGIN as f32 + 2.0;

/// Draws the dialog with `library`, the folder holding the database, at the
/// bottom. Returns true once it should close.
pub(super) fn show(ctx: &egui::Context, library: &Path) -> bool {
    let mut close = ctx.input_mut(|input| {
        input.consume_key(Modifiers::NONE, Key::F1) || input.consume_key(Modifiers::NONE, Key::Questionmark)
    });
    let screen = ctx.content_rect();
    let side_by_side = screen.width() >= 2.0 * PANEL + GAP + 120.0;
    let content = if side_by_side { 2.0 * PANEL + GAP } else { PANEL };
    let max_height = (screen.height() - 140.0).max(200.0);
    let response = egui::Modal::new(egui::Id::unique("help")).frame(about::dialog_frame()).show(ctx, |ui| {
        ui.set_width(content + 42.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        title_bar(ui, tr("Help & Shortcuts"), &mut close);
        egui::Frame::new().inner_margin(Margin { left: 16, right: 10, top: 14, bottom: 16 }).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(GAP);
                    if side_by_side {
                        panel(ui, tr("Keyboard"), KEYBOARD, false);
                        panel(ui, tr("Mouse / trackpad"), MOUSE, true);
                    } else {
                        ui.vertical(|ui| {
                            panel(ui, tr("Keyboard"), KEYBOARD, false);
                            panel(ui, tr("Mouse / trackpad"), MOUSE, true);
                        });
                    }
                    // Room for the scroll bar.
                    ui.add_space(4.0);
                });
            });
            ui.add_space(12.0);
            let path = library.display().to_string();
            let path = path.trim_start_matches(r"\\?\");
            ui.label(RichText::new(trf("Library: {}", &[&path])).size(11.5).color(theme::muted()));
        });
    });
    close || response.should_close()
}

/// Bold `title` and a close button that sets `close`, over a separator.
/// Shared with the Settings dialog.
pub(super) fn title_bar(ui: &mut egui::Ui, title: &str, close: &mut bool) {
    egui::Frame::new().inner_margin(Margin { left: 18, right: 12, top: 10, bottom: 10 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(15.0).strong().color(theme::text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if close_button(ui).on_hover_text(tr("Close")).clicked() {
                    *close = true;
                }
            });
        });
    });
    ui.add(egui::Separator::default().spacing(0.0));
}

fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    let painter = ui.painter();
    let fill = if hovered { theme::hover() } else { Color32::TRANSPARENT };
    painter.rect(rect, 6, fill, Stroke::new(1.0, theme::hairline()), egui::StrokeKind::Inside);
    let arm = 4.5;
    let stroke = Stroke::new(1.4, if hovered { theme::text() } else { theme::muted() });
    let c = rect.center();
    painter.line_segment([c + Vec2::new(-arm, -arm), c + Vec2::new(arm, arm)], stroke);
    painter.line_segment([c + Vec2::new(arm, -arm), c + Vec2::new(-arm, arm)], stroke);
    response
}

/// Outlined box with an upper-case `title` and one block per section.
fn panel(ui: &mut egui::Ui, title: &str, sections: &[Section], mouse: bool) {
    egui::Frame::new()
        .fill(theme::tray())
        .stroke(Stroke::new(1.0, theme::hairline()))
        .corner_radius(8)
        .inner_margin(Margin::same(PANEL_MARGIN))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(KEYS + GAP + ACTION);
                ui.spacing_mut().item_spacing = Vec2::new(GAP, 6.0);
                ui.spacing_mut().interact_size.y = 18.0;
                ui.label(RichText::new(title.to_uppercase()).size(12.0).strong().color(theme::muted()));
                for (heading, rows) in sections {
                    ui.add_space(8.0);
                    ui.label(RichText::new(tr(heading)).size(12.5).strong().color(theme::text()));
                    for (inputs, action) in *rows {
                        row(ui, &input_text(inputs, mouse), tr(action));
                    }
                }
            });
        });
}

fn row(ui: &mut egui::Ui, inputs: &str, action: &str) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(KEYS, 0.0), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.set_width(KEYS);
            keycap(ui, inputs);
        });
        ui.allocate_ui_with_layout(Vec2::new(ACTION, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(ACTION);
            ui.add_space(1.0);
            ui.add(egui::Label::new(RichText::new(action).size(13.0).color(theme::text())).wrap());
        });
    });
}

/// Inputs joined with " / ". Ctrl reads as ⌘ on macOS, where the same
/// shortcuts use the Command key.
fn input_text(inputs: &[&'static str], mouse: bool) -> String {
    let text = inputs
        .iter()
        .map(|&input| match input.rfind('+') {
            Some(at) if mouse => {
                let (modifiers, gesture) = input.split_at(at + 1);
                format!("{modifiers}{}", tr(gesture))
            }
            _ if mouse => tr(input).to_string(),
            _ => input.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" / ");
    if cfg!(target_os = "macos") {
        text.replace("Ctrl+", "⌘")
    } else {
        text
    }
}

/// Key cap in the link color. Latin text uses the monospace font, which has
/// no CJK glyphs, so translated gestures in those scripts stay proportional.
fn keycap(ui: &mut egui::Ui, text: &str) {
    let latin = text.chars().all(|c| (c as u32) < 0x2E80);
    let font = if latin { FontId::monospace(11.5) } else { FontId::proportional(12.0) };
    let color = ui.visuals().hyperlink_color;
    egui::Frame::new()
        .fill(theme::well())
        .corner_radius(4)
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| ui.label(RichText::new(text).font(font).color(color)));
}
