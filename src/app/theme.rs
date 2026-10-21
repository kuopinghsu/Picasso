//! Light and dark color themes shared by every page.
//!
//! Colors that depend on the theme are read through functions such as
//! [`text`] and [`panel`]; the constants below look the same in both.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Shadow, Stroke, TextStyle, Vec2};

pub const ACCENT: Color32 = Color32::from_rgb(0x2F, 0x6F, 0xEB);
pub const STAR: Color32 = Color32::from_rgb(0xF5, 0xB4, 0x00);
pub const FOLDER: Color32 = Color32::from_rgb(0xF2, 0xB5, 0x3B);
pub const ALBUM: Color32 = Color32::from_rgb(0x8B, 0x5C, 0xF6);
pub const DANGER: Color32 = Color32::from_rgb(0xD9, 0x3A, 0x3A);
/// Marks drawn over a photo or on a dark badge, in either theme.
pub const WHITE: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

pub const RADIUS: u8 = 6;
pub const THUMB_RADIUS: u8 = 8;

/// The theme the user picked. `System` follows the operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

    pub fn code(self) -> &'static str {
        match self {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn from_code(code: &str) -> ThemeMode {
        match code {
            "light" => ThemeMode::Light,
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::System,
        }
    }

    /// English label, translated with `tr` where shown.
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "System theme",
            ThemeMode::Light => "Light theme",
            ThemeMode::Dark => "Dark theme",
        }
    }

    /// True when this mode shows the dark palette on a system that is dark
    /// when `system_dark` is.
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            ThemeMode::System => system_dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        }
    }
}

struct Palette {
    /// Library page and dialogs.
    bg: Color32,
    /// Header and viewer toolbar.
    toolbar: Color32,
    /// Status bar at the bottom of the library.
    tray: Color32,
    /// Folder list, side panels, and the editor.
    sidebar: Color32,
    /// Behind the photo in the viewer.
    stage: Color32,
    /// Inset areas such as the histogram and the curve graph.
    well: Color32,
    select: Color32,
    hover: Color32,
    placeholder: Color32,
    text: Color32,
    muted: Color32,
    hairline: Color32,
}

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    toolbar: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    tray: Color32::from_rgb(0xFA, 0xFA, 0xFB),
    sidebar: Color32::from_rgb(0xF6, 0xF7, 0xF9),
    stage: Color32::from_rgb(0xE9, 0xEB, 0xEE),
    well: Color32::from_rgb(0xEC, 0xEE, 0xF2),
    select: Color32::from_rgb(0xE4, 0xED, 0xFD),
    hover: Color32::from_rgb(0xEC, 0xEE, 0xF2),
    placeholder: Color32::from_rgb(0xEE, 0xF0, 0xF3),
    text: Color32::from_rgb(0x1F, 0x23, 0x28),
    muted: Color32::from_rgb(0x6B, 0x72, 0x80),
    hairline: Color32::from_rgb(0xE4, 0xE6, 0xEB),
};

const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x13, 0x14, 0x17),
    toolbar: Color32::from_rgb(0x17, 0x18, 0x1B),
    tray: Color32::from_rgb(0x17, 0x18, 0x1B),
    sidebar: Color32::from_rgb(0x1B, 0x1C, 0x20),
    stage: Color32::from_rgb(0x0E, 0x0F, 0x11),
    well: Color32::from_rgb(0x24, 0x26, 0x2A),
    select: Color32::from_rgb(0x22, 0x36, 0x5E),
    hover: Color32::from_rgb(0x2A, 0x2C, 0x31),
    placeholder: Color32::from_rgb(0x26, 0x28, 0x2C),
    text: Color32::from_rgb(0xE8, 0xEA, 0xED),
    muted: Color32::from_rgb(0x9A, 0xA0, 0xA8),
    hairline: Color32::from_rgb(0x2C, 0x2E, 0x33),
};

static DARK_ACTIVE: AtomicBool = AtomicBool::new(false);

fn palette() -> &'static Palette {
    if DARK_ACTIVE.load(Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

pub fn is_dark() -> bool {
    DARK_ACTIVE.load(Ordering::Relaxed)
}

pub fn bg() -> Color32 {
    palette().bg
}
pub fn toolbar() -> Color32 {
    palette().toolbar
}
pub fn tray() -> Color32 {
    palette().tray
}
pub fn sidebar() -> Color32 {
    palette().sidebar
}
pub fn stage() -> Color32 {
    palette().stage
}
pub fn well() -> Color32 {
    palette().well
}
pub fn select() -> Color32 {
    palette().select
}
pub fn hover() -> Color32 {
    palette().hover
}
pub fn placeholder() -> Color32 {
    palette().placeholder
}
pub fn text() -> Color32 {
    palette().text
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn hairline() -> Color32 {
    palette().hairline
}

/// Translucent tint that stands out from the current background: white on
/// the dark theme, black on the light one.
pub fn overlay(alpha: u8) -> Color32 {
    if is_dark() {
        Color32::from_white_alpha(alpha)
    } else {
        Color32::from_black_alpha(alpha)
    }
}

/// Fonts and spacing, set once at startup.
pub fn apply(ctx: &egui::Context, dark: bool) {
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(TextStyle::Body, FontId::proportional(13.5));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(13.5));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(11.0));
        style.text_styles.insert(TextStyle::Heading, FontId::proportional(20.0));
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(18);
    });
    set_dark(ctx, dark);
}

/// Switches every page to the light or dark palette.
pub fn set_dark(ctx: &egui::Context, dark: bool) {
    DARK_ACTIVE.store(dark, Ordering::Relaxed);
    ctx.set_theme(if dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
    ctx.all_styles_mut(|style| style.visuals = visuals(dark));
}

fn visuals(dark: bool) -> egui::Visuals {
    let p = if dark { &DARK } else { &LIGHT };
    let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = p.bg;
    visuals.window_fill = if dark { p.sidebar } else { WHITE };
    visuals.window_stroke = Stroke::new(1.0, p.hairline);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 110 } else { 36 }),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 28 }),
    };
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.extreme_bg_color = if dark { Color32::from_rgb(0x22, 0x24, 0x28) } else { WHITE };
    visuals.faint_bg_color = if dark { Color32::from_rgb(0x22, 0x24, 0x28) } else { p.sidebar };
    visuals.selection.bg_fill = if dark { Color32::from_rgb(0x2B, 0x4C, 0x8F) } else { p.select };
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = if dark { Color32::from_rgb(0x6C, 0xA6, 0xFF) } else { ACCENT };
    visuals.slider_trailing_fill = true;

    // Fill and outline of buttons and fields: resting, hovered, pressed.
    let [rest, rest_fill, rest_line, over, over_line, press, press_line] = if dark {
        [(0x26, 0x28, 0x2C), (0x30, 0x32, 0x37), (0x30, 0x32, 0x37), (0x32, 0x35, 0x3A), (0x41, 0x45, 0x4C), (0x3B, 0x3F, 0x45), (0x4A, 0x4F, 0x57)]
    } else {
        [(0xF3, 0xF4, 0xF6), (0xE4, 0xE6, 0xEB), (0xDA, 0xDD, 0xE2), (0xEC, 0xEE, 0xF2), (0xC9, 0xCE, 0xD6), (0xDF, 0xE3, 0xE9), (0xB9, 0xC0, 0xCA)]
    }
    .map(|(r, g, b)| Color32::from_rgb(r, g, b));
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.hairline);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.inactive.weak_bg_fill = rest;
    widgets.inactive.bg_fill = rest_fill;
    widgets.inactive.bg_stroke = Stroke::new(1.0, rest_line);
    widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.hovered.weak_bg_fill = over;
    widgets.hovered.bg_fill = over;
    widgets.hovered.bg_stroke = Stroke::new(1.0, over_line);
    widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    widgets.active.weak_bg_fill = press;
    widgets.active.bg_fill = press;
    widgets.active.bg_stroke = Stroke::new(1.0, press_line);
    widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    widgets.open.weak_bg_fill = over;
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
        color: Color32::from_black_alpha(if is_dark() { 120 } else { 40 }),
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

/// Theme glyph about 14 points wide: a crescent moon when `moon`, otherwise
/// a sun with eight rays.
pub fn paint_theme_icon(painter: &egui::Painter, center: Pos2, moon: bool, color: Color32) {
    if !moon {
        painter.circle_filled(center, 3.4, color);
        for step in 0..8 {
            let angle = step as f32 * std::f32::consts::FRAC_PI_4;
            let dir = Vec2::angled(angle);
            painter.line_segment([center + dir * 5.3, center + dir * 7.0], Stroke::new(1.5, color));
        }
        return;
    }
    // Disc of `radius` minus the same disc shifted right by `cut`, swept
    // top to bottom as a triangle strip and tilted toward the upper right.
    let (radius, cut, rows) = (6.5_f32, 4.6_f32, 24);
    let tilt = Vec2::angled(-std::f32::consts::FRAC_PI_4);
    let place = |x: f32, y: f32| center + Vec2::new(x * tilt.x - y * tilt.y, x * tilt.y + y * tilt.x);
    let mut mesh = egui::Mesh::default();
    for row in 0..=rows {
        let y = -radius + 2.0 * radius * row as f32 / rows as f32;
        let half = (radius * radius - y * y).max(0.0).sqrt();
        mesh.colored_vertex(place(-half, y), color);
        mesh.colored_vertex(place(half.min(cut - half), y), color);
        if row > 0 {
            let i = mesh.vertices.len() as u32;
            mesh.add_triangle(i - 4, i - 3, i - 2);
            mesh.add_triangle(i - 3, i - 2, i - 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Badge for an edited photo: three slider tracks with knobs on a dark disc
/// of `radius`.
pub fn paint_edit_mark(painter: &egui::Painter, center: Pos2, radius: f32) {
    painter.circle_filled(center, radius, Color32::from_black_alpha(110));
    let half = radius * 0.5;
    let track = Stroke::new((radius * 0.12).max(1.0), WHITE);
    for (row, knob) in [(-1.0, -0.4), (0.0, 0.45), (1.0, -0.1)] {
        let y = center.y + row * radius * 0.36;
        painter.line_segment([Pos2::new(center.x - half, y), Pos2::new(center.x + half, y)], track);
        painter.circle_filled(Pos2::new(center.x + knob * half, y), radius * 0.17, WHITE);
    }
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
