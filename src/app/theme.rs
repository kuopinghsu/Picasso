//! Picasa 3 library chrome: light toolbar, white photo well, blue selection.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Stroke};

pub const BG: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const TOOLBAR: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF2);
pub const TRAY: Color32 = Color32::from_rgb(0xEC, 0xEC, 0xEC);
pub const SIDEBAR: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const ACCENT: Color32 = Color32::from_rgb(0x3A, 0x7F, 0xD5);
pub const SELECT: Color32 = Color32::from_rgb(0xD4, 0xE6, 0xF8);
pub const ACCENT_SOFT: Color32 = SELECT;
pub const TEXT: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
pub const MUTED: Color32 = Color32::from_rgb(0x66, 0x66, 0x66);
pub const HAIRLINE: Color32 = Color32::from_rgb(0xC6, 0xC6, 0xC6);
pub const STAR: Color32 = Color32::from_rgb(0xF5, 0xC4, 0x18);
pub const FOLDER: Color32 = Color32::from_rgb(0xF0, 0xC1, 0x3A);
pub const FACE: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const VIEWER_BG: Color32 = Color32::from_rgb(0x3C, 0x3C, 0x3C);
pub const VIEWER_BAR: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2A);
pub const WHITE: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.all_styles_mut(|style| {
        style.visuals.panel_fill = SIDEBAR;
        style.visuals.window_fill = WHITE;
        style.visuals.extreme_bg_color = Color32::from_rgb(0xF7, 0xF7, 0xF7);
        style.visuals.faint_bg_color = Color32::from_rgb(0xF4, 0xF4, 0xF4);
        style.visuals.selection.bg_fill = ACCENT_SOFT;
        style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xF7, 0xF7, 0xF7);
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xC0, 0xC0, 0xC0));
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xE5, 0xF1, 0xFB);
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x7E, 0xB4, 0xEA));
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(0xE5, 0xF1, 0xFB);
        style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0xCC, 0xE4, 0xF7);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(0xCC, 0xE4, 0xF7);
        style.visuals.widgets.inactive.corner_radius = CornerRadius::same(2);
        style.visuals.widgets.hovered.corner_radius = CornerRadius::same(2);
        style.visuals.widgets.active.corner_radius = CornerRadius::same(2);
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
    });
}

/// Blue lens mark in the same toolbar slot as Picasa 3's application icon.
pub fn paint_logo(painter: &egui::Painter, center: Pos2, radius: f32) {
    painter.circle_filled(center, radius, Color32::from_rgb(0x2F, 0x6F, 0xC4));
    painter.circle_stroke(center, radius * 0.62, Stroke::new(radius * 0.16, WHITE));
    painter.circle_filled(center, radius * 0.22, WHITE);
}

pub fn paint_image(painter: &egui::Painter, texture: &egui::TextureHandle, rect: egui::Rect, uv: egui::Rect) {
    painter.image(texture.id(), rect, uv, WHITE);
}

pub fn cover_uv(width: usize, height: usize) -> egui::Rect {
    let width = width as f32;
    let height = height as f32;
    if width <= 1.0 || height <= 1.0 || (width - height).abs() < 1.0 {
        return egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    if width > height {
        let margin = (1.0 - height / width) * 0.5;
        egui::Rect::from_min_max(egui::pos2(margin, 0.0), egui::pos2(1.0 - margin, 1.0))
    } else {
        let margin = (1.0 - width / height) * 0.5;
        egui::Rect::from_min_max(egui::pos2(0.0, margin), egui::pos2(1.0, 1.0 - margin))
    }
}

pub fn frame(fill: Color32) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(egui::Margin::same(8))
}

pub fn toolbar_frame(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(8, 5))
}
