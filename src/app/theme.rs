//! Modern light library chrome with a dark photo viewer.

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Shadow, Stroke, TextStyle, Vec2};

pub const BG: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const TOOLBAR: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const TRAY: Color32 = Color32::from_rgb(0xFA, 0xFA, 0xFB);
pub const SIDEBAR: Color32 = Color32::from_rgb(0xF6, 0xF7, 0xF9);
pub const ACCENT: Color32 = Color32::from_rgb(0x2F, 0x6F, 0xEB);
pub const SELECT: Color32 = Color32::from_rgb(0xE4, 0xED, 0xFD);
pub const HOVER: Color32 = Color32::from_rgb(0xEC, 0xEE, 0xF2);
pub const PLACEHOLDER: Color32 = Color32::from_rgb(0xEE, 0xF0, 0xF3);
pub const TEXT: Color32 = Color32::from_rgb(0x1F, 0x23, 0x28);
pub const MUTED: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);
pub const HAIRLINE: Color32 = Color32::from_rgb(0xE4, 0xE6, 0xEB);
pub const STAR: Color32 = Color32::from_rgb(0xF5, 0xB4, 0x00);
pub const FOLDER: Color32 = Color32::from_rgb(0xF2, 0xB5, 0x3B);
pub const ALBUM: Color32 = Color32::from_rgb(0x8B, 0x5C, 0xF6);
pub const DANGER: Color32 = Color32::from_rgb(0xD9, 0x3A, 0x3A);
pub const VIEWER_BG: Color32 = Color32::from_rgb(0x0E, 0x0F, 0x11);
pub const VIEWER_BAR: Color32 = Color32::from_rgb(0x17, 0x18, 0x1B);
pub const VIEWER_PANEL: Color32 = Color32::from_rgb(0x1B, 0x1C, 0x20);
pub const VIEWER_MUTED: Color32 = Color32::from_rgb(0x9A, 0xA0, 0xA8);
pub const WHITE: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

pub const RADIUS: u8 = 6;
pub const THUMB_RADIUS: u8 = 8;

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(TextStyle::Body, FontId::proportional(13.5));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(13.5));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(11.0));
        style.text_styles.insert(TextStyle::Heading, FontId::proportional(20.0));

        let visuals = &mut style.visuals;
        visuals.panel_fill = BG;
        visuals.window_fill = WHITE;
        visuals.window_stroke = Stroke::new(1.0, HAIRLINE);
        visuals.window_corner_radius = CornerRadius::same(12);
        visuals.window_shadow = Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(36),
        };
        visuals.popup_shadow = Shadow {
            offset: [0, 6],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(28),
        };
        visuals.menu_corner_radius = CornerRadius::same(8);
        visuals.extreme_bg_color = WHITE;
        visuals.faint_bg_color = SIDEBAR;
        visuals.selection.bg_fill = SELECT;
        visuals.selection.stroke = Stroke::new(1.0, ACCENT);
        visuals.hyperlink_color = ACCENT;
        visuals.slider_trailing_fill = true;

        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        widgets.inactive.weak_bg_fill = Color32::from_rgb(0xF3, 0xF4, 0xF6);
        widgets.inactive.bg_fill = Color32::from_rgb(0xE4, 0xE6, 0xEB);
        widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xDA, 0xDD, 0xE2));
        widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        widgets.hovered.weak_bg_fill = HOVER;
        widgets.hovered.bg_fill = HOVER;
        widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xC9, 0xCE, 0xD6));
        widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
        widgets.active.weak_bg_fill = Color32::from_rgb(0xDF, 0xE3, 0xE9);
        widgets.active.bg_fill = Color32::from_rgb(0xDF, 0xE3, 0xE9);
        widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xB9, 0xC0, 0xCA));
        widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
        widgets.open.weak_bg_fill = HOVER;
        round_widgets(visuals);

        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(18);
    });
}

/// Dark style for the photo viewer, so the chrome recedes behind the picture.
pub fn viewer_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = VIEWER_BG;
    visuals.extreme_bg_color = Color32::from_rgb(0x22, 0x24, 0x28);
    visuals.faint_bg_color = Color32::from_rgb(0x22, 0x24, 0x28);
    visuals.selection.bg_fill = Color32::from_rgb(0x2B, 0x4C, 0x8F);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.slider_trailing_fill = true;
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x2C, 0x2E, 0x33));
    widgets.inactive.weak_bg_fill = Color32::from_rgb(0x26, 0x28, 0x2C);
    widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x30, 0x32, 0x37));
    widgets.hovered.weak_bg_fill = Color32::from_rgb(0x32, 0x35, 0x3A);
    widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x41, 0x45, 0x4C));
    widgets.active.weak_bg_fill = Color32::from_rgb(0x3B, 0x3F, 0x45);
    round_widgets(&mut visuals);
    visuals
}

fn round_widgets(visuals: &mut egui::Visuals) {
    let widgets = &mut visuals.widgets;
    for state in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.corner_radius = CornerRadius::same(RADIUS);
        state.expansion = 0.0;
    }
}

pub fn primary_button(text: &str) -> egui::Button<'static> {
    filled_button(text, ACCENT)
}

pub fn danger_button(text: &str) -> egui::Button<'static> {
    filled_button(text, DANGER)
}

fn filled_button(text: &str, fill: Color32) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).color(WHITE).strong())
        .fill(fill)
        .stroke(Stroke::NONE)
}

pub fn card_shadow() -> Shadow {
    Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(40),
    }
}

/// Application icon as a mipmapped texture, so it stays smooth when drawn
/// much smaller than its 256 px source.
pub fn load_logo(ctx: &egui::Context) -> egui::TextureHandle {
    let size = crate::ICON_SIZE;
    let image = egui::ColorImage::from_rgba_unmultiplied([size, size], crate::ICON_RGBA);
    let options = egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
    ctx.load_texture("picasso-logo", image, options)
}

pub fn paint_logo(painter: &egui::Painter, logo: &egui::TextureHandle, rect: Rect) {
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    painter.image(logo.id(), rect, uv, WHITE);
}

/// Map-pin mark: a round head of `radius` over a point, with a `hole` dot.
pub fn paint_pin(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32, hole: Color32) {
    let head = center + Vec2::new(0.0, -0.45 * radius);
    painter.add(egui::Shape::convex_polygon(
        vec![
            head + Vec2::new(-0.84 * radius, 0.45 * radius),
            head + Vec2::new(0.84 * radius, 0.45 * radius),
            center + Vec2::new(0.0, 1.45 * radius),
        ],
        color,
        Stroke::NONE,
    ));
    painter.circle_filled(head, radius, color);
    painter.circle_filled(head, 0.36 * radius, hole);
}

/// Draws `uv` of the texture into `rect`, turned `rotation` degrees clockwise.
///
/// Rotation is applied to the texture coordinates while drawing, so decoded
/// pixels, cached thumbnails, and the photo file all stay as they are. `rect`
/// is the on-screen box, already in the rotated aspect ratio.
pub fn paint_image(painter: &egui::Painter, texture: &egui::TextureHandle, rect: Rect, uv: Rect, rotation: i32) {
    paint_image_rounded(painter, texture, rect, uv, rotation, 0.0);
}

/// Same as [`paint_image`], clipped to a rectangle with rounded corners.
pub fn paint_image_rounded(
    painter: &egui::Painter,
    texture: &egui::TextureHandle,
    rect: Rect,
    uv: Rect,
    rotation: i32,
    radius: f32,
) {
    let turns = rotation.div_euclid(90).rem_euclid(4) as usize;
    if turns == 0 && radius <= 0.5 {
        painter.image(texture.id(), rect, uv, WHITE);
        return;
    }
    // Texture corners for the screen corners, clockwise from top-left.
    let tex = [uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()];
    let [a, b, c, d] = [0, 1, 2, 3].map(|corner| tex[(corner + 4 - turns) % 4]);
    let uv_at = |pos: Pos2| {
        let s = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        let t = ((pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        let mix = |tl: f32, tr: f32, br: f32, bl: f32| {
            tl * (1.0 - s) * (1.0 - t) + tr * s * (1.0 - t) + br * s * t + bl * (1.0 - s) * t
        };
        Pos2::new(mix(a.x, b.x, c.x, d.x), mix(a.y, b.y, c.y, d.y))
    };
    let mut mesh = egui::Mesh::with_texture(texture.id());
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.center(),
        uv: uv_at(rect.center()),
        color: WHITE,
    });
    let outline = rounded_outline(rect, radius);
    for pos in &outline {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *pos,
            uv: uv_at(*pos),
            color: WHITE,
        });
    }
    let count = outline.len() as u32;
    for index in 0..count {
        mesh.add_triangle(0, 1 + index, 1 + (index + 1) % count);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let radius = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    if radius <= 0.5 {
        return vec![rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    }
    const STEPS: usize = 6;
    let quarter = std::f32::consts::FRAC_PI_2;
    let corners = [
        (Pos2::new(rect.left() + radius, rect.top() + radius), 2.0),
        (Pos2::new(rect.right() - radius, rect.top() + radius), 3.0),
        (Pos2::new(rect.right() - radius, rect.bottom() - radius), 0.0),
        (Pos2::new(rect.left() + radius, rect.bottom() - radius), 1.0),
    ];
    let mut points = Vec::with_capacity(4 * (STEPS + 1));
    for (center, start) in corners {
        for step in 0..=STEPS {
            let angle = (start + step as f32 / STEPS as f32) * quarter;
            points.push(center + Vec2::new(angle.cos(), angle.sin()) * radius);
        }
    }
    points
}

pub fn cover_uv(width: usize, height: usize) -> Rect {
    let width = width as f32;
    let height = height as f32;
    if width <= 1.0 || height <= 1.0 || (width - height).abs() < 1.0 {
        return Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    if width > height {
        let margin = (1.0 - height / width) * 0.5;
        Rect::from_min_max(egui::pos2(margin, 0.0), egui::pos2(1.0 - margin, 1.0))
    } else {
        let margin = (1.0 - width / height) * 0.5;
        Rect::from_min_max(egui::pos2(0.0, margin), egui::pos2(1.0, 1.0 - margin))
    }
}

pub fn frame(fill: Color32) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(18, 12))
}

pub fn toolbar_frame(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(14, 9))
}
