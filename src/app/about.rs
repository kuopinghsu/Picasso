//! About dialog, opened from the logo in the header.

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Shadow, Stroke, Vec2};

use crate::i18n::{tr, trf};

use super::theme;

const RADIUS: u8 = 10;

const FEATURES: [&str; 4] = [
    "Fast thumbnails and background scanning, written in Rust",
    "Timeline, folder tree, albums, and places on a map",
    "Non-destructive editing with undo and redo",
    "HEIC, camera RAW, and video playback",
];

/// Draws the dialog. Returns true once it should close.
pub(super) fn show(ctx: &egui::Context, logo: &egui::TextureHandle) -> bool {
    let mut close = false;
    let response = egui::Modal::new(egui::Id::unique("about")).frame(dialog_frame()).show(ctx, |ui| {
        ui.set_width(620.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        title_bar(ui, logo, tr("About Picasso"), &mut close);
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

/// Card behind the About, Help & Shortcuts, and Settings dialogs.
pub(super) fn dialog_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::bg())
        .corner_radius(CornerRadius::same(RADIUS))
        .stroke(Stroke::new(1.0, theme::hairline()))
        .shadow(Shadow {
            offset: [0, 12],
            blur: 32,
            spread: 0,
            color: Color32::from_black_alpha(if theme::is_dark() { 90 } else { 45 }),
        })
}

/// Window-style strip across the top of a dialog with the logo, `title`,
/// and a close button that sets `close`.
pub(super) fn title_bar(ui: &mut egui::Ui, logo: &egui::TextureHandle, title: &str, close: &mut bool) {
    let corners = CornerRadius {
        nw: RADIUS,
        ne: RADIUS,
        sw: 0,
        se: 0,
    };
    egui::Frame::new()
        .fill(theme::sidebar())
        .corner_radius(corners)
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                theme::paint_logo(ui.painter(), logo, rect);
                ui.label(RichText::new(title).size(13.0).color(theme::text()));
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
        painter.rect_filled(rect, CornerRadius::same(6), theme::hover());
    }
    let arm = 5.0;
    let stroke = Stroke::new(1.4, if response.hovered() { theme::text() } else { theme::muted() });
    let c = rect.center();
    painter.line_segment([c + Vec2::new(-arm, -arm), c + Vec2::new(arm, arm)], stroke);
    painter.line_segment([c + Vec2::new(arm, -arm), c + Vec2::new(-arm, arm)], stroke);
    response
}

fn body(ui: &mut egui::Ui) {
    let build = match env!("PICASSO_COMMIT") {
        "" => env!("PICASSO_BUILD_DATE").to_string(),
        commit => format!("{} · {commit}", env!("PICASSO_BUILD_DATE")),
    };
    let mut title = egui::text::LayoutJob::default();
    let parts = [
        RichText::new(format!("Picasso {}  ", env!("CARGO_PKG_VERSION"))).size(26.0).strong().color(theme::text()),
        RichText::new(trf("(Build {})", &[&build])).size(16.0).color(theme::muted()),
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
    text(ui, tr("Maps from OpenFreeMap and OpenMapTiles, OpenStreetMap, CARTO, and Esri"));
    text(ui, tr("And everyone who contributes to these open source projects"));

    heading(ui, tr("License"));
    let repository = env!("CARGO_PKG_REPOSITORY");
    let label = RichText::new(repository.trim_start_matches("https://")).size(13.5).color(ui.visuals().hyperlink_color);
    ui.hyperlink_to(label, repository);
    let year = build.get(..4).unwrap_or(&build);
    text(
        ui,
        &trf("© {} Picasso contributors. Licensed under the {} License.", &[&year, &env!("CARGO_PKG_LICENSE")]),
    );
}

fn heading(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title).size(17.0).strong().color(theme::text()));
    ui.add_space(1.0);
}

fn text(ui: &mut egui::Ui, line: &str) {
    ui.label(RichText::new(line).size(13.5).color(theme::text()));
}
