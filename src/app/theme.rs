//! Light and dark color themes shared by every page.
//!
//! Colors that depend on the theme are read through functions such as
//! [`text`] and [`accent`]; the constants below look the same in both.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::i18n::tr;

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Shadow, Stroke, TextStyle, Vec2};

/// Lagoon teal, dark enough for white text on it: filled buttons and marks
/// over photos or map tiles, which look the same in either theme. For text
/// and marks on the page itself, use [`accent`].
pub const ACCENT: Color32 = Color32::from_rgb(0x09, 0x72, 0x7B);
/// Golden-hour amber: places with photos on the travel map, and the marks
/// that lead there.
pub const GOLD: Color32 = Color32::from_rgb(0xE9, 0xA3, 0x20);
/// Coral red of map pins and of everything about where photos were taken.
pub const PIN: Color32 = Color32::from_rgb(0xE5, 0x53, 0x3D);
/// Green of Statistics, from its sidebar icon to its charts.
pub const GREEN: Color32 = Color32::from_rgb(0x10, 0xB9, 0x81);
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

/// Paper and sand: near-neutral so photos keep their color, a touch warm.
const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xFD, 0xFD, 0xFB),
    toolbar: Color32::from_rgb(0xFD, 0xFD, 0xFB),
    tray: Color32::from_rgb(0xF8, 0xF7, 0xF4),
    sidebar: Color32::from_rgb(0xF5, 0xF4, 0xF0),
    stage: Color32::from_rgb(0xE8, 0xE7, 0xE3),
    well: Color32::from_rgb(0xEC, 0xEB, 0xE6),
    select: Color32::from_rgb(0xDC, 0xEF, 0xF0),
    hover: Color32::from_rgb(0xED, 0xEC, 0xE7),
    placeholder: Color32::from_rgb(0xEE, 0xED, 0xE9),
    text: Color32::from_rgb(0x1C, 0x21, 0x24),
    muted: Color32::from_rgb(0x63, 0x68, 0x6C),
    hairline: Color32::from_rgb(0xE3, 0xE1, 0xDB),
};

/// Night sea: charcoal with a little blue-green, the same steps as light.
const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x11, 0x15, 0x17),
    toolbar: Color32::from_rgb(0x15, 0x1A, 0x1C),
    tray: Color32::from_rgb(0x15, 0x1A, 0x1C),
    sidebar: Color32::from_rgb(0x19, 0x1F, 0x22),
    stage: Color32::from_rgb(0x0C, 0x0F, 0x10),
    well: Color32::from_rgb(0x22, 0x29, 0x2C),
    select: Color32::from_rgb(0x10, 0x39, 0x3D),
    hover: Color32::from_rgb(0x28, 0x30, 0x33),
    placeholder: Color32::from_rgb(0x24, 0x2B, 0x2E),
    text: Color32::from_rgb(0xE6, 0xEA, 0xEB),
    muted: Color32::from_rgb(0x98, 0xA3, 0xA7),
    hairline: Color32::from_rgb(0x2A, 0x32, 0x35),
};

const ACCENT_DARK: Color32 = Color32::from_rgb(0x3F, 0xB5, 0xBF);

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
/// The accent for text, outlines, and charts on the page: [`ACCENT`] on
/// light, a brighter teal on dark so small text and thin lines still read.
pub fn accent() -> Color32 {
    if is_dark() {
        ACCENT_DARK
    } else {
        ACCENT
    }
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
    visuals.extreme_bg_color = if dark { Color32::from_rgb(0x20, 0x27, 0x2A) } else { WHITE };
    visuals.faint_bg_color = if dark { Color32::from_rgb(0x20, 0x27, 0x2A) } else { p.sidebar };
    visuals.selection.bg_fill = if dark { Color32::from_rgb(0x17, 0x55, 0x5B) } else { p.select };
    visuals.selection.stroke = Stroke::new(1.0, if dark { ACCENT_DARK } else { ACCENT });
    visuals.hyperlink_color = if dark { ACCENT_DARK } else { ACCENT };
    visuals.slider_trailing_fill = true;

    // Fill and outline of buttons and fields: resting, hovered, pressed.
    let [rest, rest_fill, rest_line, over, over_line, press, press_line] = if dark {
        [(0x24, 0x2B, 0x2E), (0x2E, 0x36, 0x39), (0x2E, 0x36, 0x39), (0x31, 0x39, 0x3C), (0x41, 0x4B, 0x4F), (0x3A, 0x44, 0x47), (0x4A, 0x55, 0x59)]
    } else {
        [(0xF1, 0xF0, 0xEC), (0xE3, 0xE1, 0xDB), (0xD8, 0xD6, 0xCF), (0xEB, 0xEA, 0xE5), (0xC8, 0xC6, 0xBE), (0xDE, 0xDC, 0xD5), (0xB8, 0xB5, 0xAC)]
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

/// Darkens the bottom `depth` of `rect`, whose corners are rounded by
/// `radius`, from clear to black at `alpha`, so white text there reads on
/// any photo.
pub fn paint_bottom_fade(painter: &egui::Painter, rect: Rect, radius: f32, depth: f32, alpha: u8) {
    let depth = depth.min(rect.height());
    let top = rect.bottom() - depth;
    let radius = radius.min(rect.width() * 0.5).min(depth);
    let rows = 32;
    let mut mesh = egui::Mesh::default();
    for row in 0..=rows {
        let t = row as f32 / rows as f32;
        let y = top + depth * t;
        let below = y - (rect.bottom() - radius);
        let inset = if below > 0.0 { radius - (radius * radius - below * below).max(0.0).sqrt() } else { 0.0 };
        let color = Color32::from_black_alpha((f32::from(alpha) * t.powf(1.5)).round() as u8);
        mesh.colored_vertex(Pos2::new(rect.left() + inset, y), color);
        mesh.colored_vertex(Pos2::new(rect.right() - inset, y), color);
        if row > 0 {
            let i = mesh.vertices.len() as u32;
            mesh.add_triangle(i - 4, i - 3, i - 2);
            mesh.add_triangle(i - 3, i - 2, i - 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Glyphs for [`icon_button`], each about 14 points wide.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    RotateLeft,
    RotateRight,
    Undo,
    Redo,
    AddToAlbum,
    RemoveFromAlbum,
    Clipping,
    More,
}

/// A button the size of a short text button with `icon` drawn in it.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, enabled: bool, selected: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new("     ").selected(selected));
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled() && enabled, selected, icon_label(icon)));
    let color = if enabled { text() } else { muted().gamma_multiply(0.6) };
    paint_icon(ui.painter(), icon, response.rect.center(), color);
    response
}

/// Actions at the right end of a header row made with
/// `horizontal_wrapped`. When they didn't fit beside what comes first,
/// as measured last frame under `id`, they go on a line of their own, so a
/// narrow window or a long translation never lays them over the title.
pub fn end_actions<R>(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let key = egui::Id::unique(("end-actions", id));
    let needed = ui.ctx().data(|data| data.get_temp::<f32>(key)).unwrap_or(0.0);
    // A wrapping row offers its whole width to a layout; only what is
    // after the cursor is free.
    if ui.max_rect().right() - ui.cursor().min.x < needed {
        ui.end_row();
    }
    // Wider than a line of their own, right to left would push them out
    // past the left edge; they wrap from the left instead.
    if needed > ui.max_rect().width() {
        let row = ui.spacing().interact_size.y;
        return ui
            .scope_builder(egui::UiBuilder::new().layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true)), |ui| {
                let inner = add(ui);
                // On one line again, they are as wide as they need.
                if ui.min_rect().height() <= row + 1.0 {
                    ui.ctx().data_mut(|data| data.insert_temp(key, ui.min_rect().width()));
                    ui.ctx().request_discard("the header's actions fit on one line again");
                }
                inner
            })
            .inner;
    }
    trailing(ui, id, add)
}

/// Actions against the right end of a row, added in reading order so Tab
/// goes through them left to right, as a right-to-left layout wouldn't.
/// They start as far from the end as they were wide last frame, as
/// measured under `id`.
pub fn trailing<R>(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let key = egui::Id::unique(("end-actions", id));
    let needed = ui.ctx().data(|data| data.get_temp::<f32>(key)).unwrap_or(0.0);
    let top = ui.cursor().min;
    let right = ui.max_rect().right();
    let left = (right - needed).max(top.x);
    let rect = egui::Rect::from_min_max(egui::pos2(left, top.y), egui::pos2(right, top.y + ui.spacing().interact_size.y));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        let inner = add(ui);
        let width = ui.min_rect().width();
        if (width - needed).abs() > 0.5 {
            ui.ctx().data_mut(|data| data.insert_temp(key, width));
            // Lay the frame out again with the width now known, before it shows.
            ui.ctx().request_discard("the actions at the end of a row changed width");
        }
        inner
    })
    .inner
}

/// The keyboard focus ring around `rect`, drawn while `response` has focus,
/// for areas that draw themselves instead of being a widget.
pub fn focus_ring(ui: &egui::Ui, response: &egui::Response, rect: egui::Rect) {
    if response.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 6, egui::Stroke::new(2.0, ACCENT), egui::StrokeKind::Outside);
    }
}

/// A ring around whichever widget has keyboard focus, while focus is being
/// moved with the keyboard. egui draws a focused button like a pressed one,
/// which is hard to tell from its other states.
pub fn keyboard_focus_ring(ctx: &egui::Context) {
    let id = egui::Id::unique("keyboard-focus");
    let (tab, pointer) = ctx.input(|input| (input.key_pressed(egui::Key::Tab), input.pointer.any_pressed()));
    if tab || pointer {
        ctx.data_mut(|data| data.insert_temp(id, tab));
    }
    if !ctx.data(|data| data.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    let Some(response) = ctx.memory(|memory| memory.focused()).and_then(|focused| ctx.read_response(focused)) else { return };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, id));
    painter.rect_stroke(response.rect, 6, egui::Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
}

/// Names a button that shows only a symbol or a painted icon, for screen
/// readers, and shows the name on hover.
pub fn named(response: egui::Response, name: &str) -> egui::Response {
    let enabled = response.enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
    response.on_hover_text(name)
}

fn icon_label(icon: Icon) -> &'static str {
    tr(match icon {
        Icon::RotateLeft => "Rotate Counterclockwise",
        Icon::RotateRight => "Rotate Clockwise",
        Icon::Undo => "Undo",
        Icon::Redo => "Redo",
        Icon::AddToAlbum => "Add to Album",
        Icon::RemoveFromAlbum => "Remove from Album",
        Icon::Clipping => "Show clipped highlights and shadows",
        Icon::More => "More",
    })
}

pub fn paint_icon(painter: &egui::Painter, icon: Icon, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    let arrowhead = |tip: Pos2, dir: Vec2| {
        let side = Vec2::new(-dir.y, dir.x) * 2.8;
        let back = tip - dir * 3.4;
        painter.add(egui::Shape::convex_polygon(vec![tip, back + side, back - side], color, Stroke::NONE));
    };
    match icon {
        Icon::RotateLeft | Icon::RotateRight => {
            // An open ring swept clockwise from upper right round to the
            // top, ending in an arrowhead; mirrored for the other way.
            let flip = if icon == Icon::RotateRight { 1.0 } else { -1.0 };
            let radius = 5.2;
            let (start, end) = (-40_f32.to_radians(), 255_f32.to_radians());
            let at = |angle: f32| center + Vec2::new(flip * angle.cos(), angle.sin()) * radius;
            let points: Vec<Pos2> = (0..=20).map(|step| at(start + (end - start) * step as f32 / 20.0)).collect();
            painter.add(egui::Shape::line(points, stroke));
            let dir = Vec2::new(-flip * end.sin(), end.cos());
            arrowhead(at(end) + dir * 2.4, dir);
        }
        Icon::Undo | Icon::Redo => {
            // A straight stroke out to the arrowhead, coming back round a
            // half turn underneath.
            let flip = if icon == Icon::Undo { 1.0 } else { -1.0 };
            let point = |x: f32, y: f32| center + Vec2::new(flip * x, y);
            let mut points = vec![point(-3.0, -2.5), point(2.0, -2.5)];
            for step in 1..=12 {
                let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * step as f32 / 12.0;
                points.push(point(2.0 + 3.25 * angle.cos(), 0.75 + 3.25 * angle.sin()));
            }
            points.push(point(-1.5, 4.0));
            painter.add(egui::Shape::line(points, stroke));
            arrowhead(point(-6.0, -2.5), Vec2::new(-flip, 0.0));
        }
        Icon::AddToAlbum | Icon::RemoveFromAlbum => {
            let front = Rect::from_center_size(center + Vec2::new(-1.0, 1.0), Vec2::new(11.0, 9.0));
            painter.rect_stroke(front, 2.0, stroke, egui::StrokeKind::Middle);
            painter.line_segment([front.left_top() + Vec2::new(2.5, -2.5), front.right_top() + Vec2::new(2.5, -2.5)], stroke);
            painter.line_segment([front.right_top() + Vec2::new(2.5, -2.5), front.right_bottom() + Vec2::new(2.5, -2.5)], stroke);
            let mid = front.center();
            painter.line_segment([mid - Vec2::new(2.5, 0.0), mid + Vec2::new(2.5, 0.0)], stroke);
            if icon == Icon::AddToAlbum {
                painter.line_segment([mid - Vec2::new(0.0, 2.5), mid + Vec2::new(0.0, 2.5)], stroke);
            }
        }
        Icon::Clipping => {
            let top = center + Vec2::new(0.0, -6.0);
            let corners = vec![top, center + Vec2::new(6.5, 5.0), center + Vec2::new(-6.5, 5.0)];
            painter.add(egui::Shape::closed_line(corners, stroke));
            painter.line_segment([center + Vec2::new(0.0, -2.0), center + Vec2::new(0.0, 1.5)], stroke);
            painter.circle_filled(center + Vec2::new(0.0, 3.4), 0.9, color);
        }
        Icon::More => {
            for x in [-5.0, 0.0, 5.0] {
                painter.circle_filled(center + Vec2::new(x, 0.0), 1.6, color);
            }
        }
    }
}

/// Help glyph about 15 points wide: a question mark in a ring.
pub fn paint_help_icon(painter: &egui::Painter, center: Pos2, color: Color32) {
    painter.circle_stroke(center, 7.2, Stroke::new(1.4, color));
    painter.text(center + Vec2::new(0.0, 0.5), egui::Align2::CENTER_CENTER, "?", FontId::proportional(11.0), color);
}

/// Gear outline with eight teeth, the same size as the help icon.
pub fn paint_settings_icon(painter: &egui::Painter, center: Pos2, color: Color32) {
    let (inner, outer) = (5.4_f32, 7.4_f32);
    let mut outline = Vec::with_capacity(32);
    for tooth in 0..8 {
        let angle = tooth as f32 * std::f32::consts::FRAC_PI_4;
        for (offset, radius) in [(-0.26, inner), (-0.15, outer), (0.15, outer), (0.26, inner)] {
            outline.push(center + Vec2::angled(angle + offset) * radius);
        }
    }
    painter.add(egui::Shape::closed_line(outline, Stroke::new(1.3, color)));
    painter.circle_stroke(center, 2.3, Stroke::new(1.3, color));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_buttons_expose_names_and_states_to_accessibility() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let icons = [Icon::RotateLeft, Icon::RotateRight, Icon::Undo, Icon::Redo, Icon::AddToAlbum, Icon::RemoveFromAlbum, Icon::Clipping, Icon::More];
        let mut buttons = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                for icon in icons {
                    let response = icon_button(ui, icon, icon != Icon::Undo, icon == Icon::Clipping);
                    buttons.push((response.id.accesskit_id(), icon));
                }
            });
        });
        output.textures_delta.clear();
        let update = output.platform_output.accesskit_update.take().expect("accessibility update");
        for (id, icon) in buttons {
            let node = &update.nodes.iter().find(|(node_id, _)| *node_id == id).expect("button node").1;
            assert_eq!(node.role(), egui::Role::Button);
            assert_eq!(node.label(), Some(icon_label(icon)));
            assert_eq!(node.is_disabled(), icon == Icon::Undo);
            assert_eq!(node.toggled(), Some(egui::accesskit::Toggled::from(icon == Icon::Clipping)));
        }
    }

    /// WCAG contrast ratio between two opaque colors, from 1 to 21.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |color: Color32| {
            let linear = |channel: u8| {
                let c = f32::from(channel) / 255.0;
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn both_themes_keep_text_readable() {
        for (name, p, accent) in [("light", &LIGHT, ACCENT), ("dark", &DARK, ACCENT_DARK)] {
            for (place, fill) in [("bg", p.bg), ("sidebar", p.sidebar), ("toolbar", p.toolbar), ("select", p.select)] {
                assert!(contrast(p.text, fill) >= 7.0, "{name} text on {place}");
                assert!(contrast(accent, fill) >= 4.5, "{name} accent text on {place}");
            }
            for (place, fill) in [("bg", p.bg), ("sidebar", p.sidebar), ("toolbar", p.toolbar), ("select", p.select), ("hover", p.hover)] {
                assert!(contrast(p.muted, fill) >= 4.5, "{name} muted on {place}");
            }
        }
        assert!(contrast(WHITE, ACCENT) >= 4.5, "button text");
        assert!(contrast(WHITE, DANGER) >= 4.0, "danger button text");
    }
}
