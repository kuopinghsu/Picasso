//! About dialog, opened from the logo in the header.

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Shadow, Stroke, Vec2};

use crate::i18n::{tr, trf};

use super::theme::{self, WHITE};

const FILL: Color32 = Color32::from_rgb(0x20, 0x21, 0x24);
const TITLE_BAR: Color32 = Color32::from_rgb(0x19, 0x1A, 0x1D);
const BORDER: Color32 = Color32::from_rgb(0x3A, 0x3C, 0x41);
const BODY: Color32 = Color32::from_rgb(0xD5, 0xD8, 0xDD);
const FAINT: Color32 = Color32::from_rgb(0x9A, 0xA0, 0xA8);
const LINK: Color32 = Color32::from_rgb(0x6C, 0xA6, 0xFF);
const RADIUS: u8 = 10;

const FEATURES: [&str; 4] = [
    "Fast thumbnails and background scanning, written in Rust",
    "Timeline, folder tree, albums, and places on a map",
    "Non-destructive editing with undo and redo",
    "HEIC, camera RAW, and video playback",
];

/// Draws the dialog. Returns true once it should close.
pub(super) fn show(ctx: &egui::Context, logo: &egui::TextureHandle) -> bool {
    let frame = egui::Frame::new()
        .fill(FILL)
        .corner_radius(CornerRadius::same(RADIUS))
        .stroke(Stroke::new(1.0, BORDER))
        .shadow(Shadow {
            offset: [0, 12],
            blur: 32,
            spread: 0,
            color: Color32::from_black_alpha(90),
        });
    let mut close = false;
    let response = egui::Modal::new(egui::Id::unique("about")).frame(frame).show(ctx, |ui| {
        ui.style_mut().visuals = theme::viewer_visuals();
        ui.set_width(620.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        title_bar(ui, logo, &mut close);
        egui::Frame::new()
            .inner_margin(Margin {
                left: 24,
                right: 28,
                top: 22,
                bottom: 26,
            })
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(136.0), Sense::hover());
                    theme::paint_logo(ui.painter(), logo, rect);
                    ui.add_space(26.0);
                    ui.vertical(|ui| body(ui));
                });
            });
    });
    close || response.should_close()
}

fn title_bar(ui: &mut egui::Ui, logo: &egui::TextureHandle, close: &mut bool) {
    let corners = CornerRadius {
        nw: RADIUS,
        ne: RADIUS,
        sw: 0,
        se: 0,
    };
    egui::Frame::new()
        .fill(TITLE_BAR)
        .corner_radius(corners)
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                theme::paint_logo(ui.painter(), logo, rect);
                ui.label(RichText::new(tr("About Picasso")).size(13.0).color(BODY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if close_button(ui).on_hover_text(tr("Close")).clicked() {
                        *close = true;
                    }
                });
            });
        });
}

fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), Color32::from_rgb(0x32, 0x35, 0x3A));
    }
    let arm = 5.0;
    let stroke = Stroke::new(1.4, if response.hovered() { WHITE } else { BODY });
    let c = rect.center();
    painter.line_segment([c + Vec2::new(-arm, -arm), c + Vec2::new(arm, arm)], stroke);
    painter.line_segment([c + Vec2::new(arm, -arm), c + Vec2::new(-arm, arm)], stroke);
    response
}

fn body(ui: &mut egui::Ui) {
    let build = env!("PICASSO_BUILD_DATE");
    let mut title = egui::text::LayoutJob::default();
    let parts = [
        RichText::new(format!("Picasso {}  ", env!("CARGO_PKG_VERSION"))).size(26.0).strong().color(WHITE),
        RichText::new(trf("(Build {})", &[&build])).size(16.0).color(FAINT),
    ];
    for part in parts {
        part.append_to(&mut title, ui.style(), egui::FontSelection::Default, egui::Align::BOTTOM);
    }
    ui.label(title);

    heading(ui, tr("Designed for speed and clarity"));
    text(ui, tr("A fast local photo library in the spirit of Picasa"));

    heading(ui, tr("Key features"));
    for feature in FEATURES {
        text(ui, &format!("-  {}", tr(feature)));
    }

    heading(ui, tr("Credits"));
    text(ui, tr("Built with Rust, egui, image, rawler, heic-rs, rusqlite, rodio, and FFmpeg"));
    text(ui, tr("Map data © OpenStreetMap contributors"));
    text(ui, tr("And everyone who contributes to these open source projects"));

    heading(ui, tr("License"));
    let repository = env!("CARGO_PKG_REPOSITORY");
    let label = RichText::new(repository.trim_start_matches("https://")).size(13.5).color(LINK);
    ui.hyperlink_to(label, repository);
    let year = build.get(..4).unwrap_or(build);
    text(
        ui,
        &trf("© {} Picasso contributors. Licensed under the {} License.", &[&year, &env!("CARGO_PKG_LICENSE")]),
    );
}

fn heading(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title).size(17.0).strong().color(WHITE));
    ui.add_space(1.0);
}

fn text(ui: &mut egui::Ui, line: &str) {
    ui.label(RichText::new(line).size(13.5).color(BODY));
}
