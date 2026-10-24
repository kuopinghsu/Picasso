//! Library grid and the browse toolbar.

use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::i18n::{tr, trf};
use crate::types::{day_key, format_day, format_month, Nav, Photo, Sort};

use super::theme::{self, ACCENT, STAR, THUMB_RADIUS, WHITE};
use super::PicassoApp;

/// Space between tiles when file names are written under them, sorted by
/// name, and when they are not.
const GAP: f32 = 14.0;
const TIGHT_GAP: f32 = 6.0;
const LABEL_H: f32 = 22.0;
const HEADER_H: f32 = 40.0;
const PAD: f32 = 2.0;
/// Seconds the scroll bar bubble takes to fade after the drag ends.
const SCROLL_HINT_FADE: f32 = 0.8;
/// Width of the year and month strip beside the grid.
const STRIP_W: f32 = 56.0;
/// Space above and below the strip's timeline.
const STRIP_PAD: f32 = 16.0;
/// Closest two year labels on the strip may be, center to center.
const YEAR_GAP: f32 = 16.0;
/// Seconds the strip keeps naming the month in view after scrolling stops,
/// before it fades like the scroll bar bubble.
const STRIP_HINT_HOLD: f32 = 0.7;
/// Seconds a click on the date strip takes to glide the grid there.
const STRIP_GLIDE: f32 = 0.35;
/// Seconds the date strip takes to brighten when pointed at or scrolled,
/// and to dim again.
const STRIP_WAKE: f32 = 0.18;
/// Shortest the date strip's handle gets, so it stays easy to take hold of.
const HANDLE_MIN: f32 = 18.0;

pub(super) struct CachedLayout {
    pub epoch: u64,
    pub width: i32,
    pub sort: Sort,
    pub thumb: i32,
    pub layout: Rc<GridLayout>,
}

pub(super) struct GridLayout {
    rows: Vec<GridRow>,
    height: f32,
    pub cols: usize,
    /// Width of a tile's image, which fills the row.
    tile: f32,
    gap: f32,
    /// Whether file names are written under the tiles rather than shown
    /// over them on hover.
    labels: bool,
    /// First day heading of each month, in grid order. Empty when sorted by name.
    months: Vec<MonthMark>,
    /// Top of the "Unknown date" heading and how many photos are under it.
    undated: Option<(f32, usize)>,
}

struct MonthMark {
    year: i32,
    month: u32,
    /// Top of the month's first day heading.
    y: f32,
    /// Photos taken that month.
    count: usize,
}

enum GridRow {
    /// `first` is the index of the first photo under the heading.
    Header { title: String, y: f32, first: usize },
    Photos {
        indices: Range<usize>,
        y: f32,
        h: f32,
    },
}

impl GridLayout {
    fn row_height(row: &GridRow) -> f32 {
        match row {
            GridRow::Header { .. } => HEADER_H,
            GridRow::Photos { h, .. } => *h,
        }
    }

    fn row_y(row: &GridRow) -> f32 {
        match row {
            GridRow::Header { y, .. } | GridRow::Photos { y, .. } => *y,
        }
    }

    /// Where the view is when its top edge is at `top`: the date heading
    /// above the first visible row, or with `Sort::Name` the file name of the
    /// first photo in that row.
    fn position_label(&self, photos: &[Photo], sort: Sort, top: f32) -> Option<String> {
        let first = self.rows.partition_point(|row| Self::row_y(row) + Self::row_height(row) <= top + 1.0);
        if sort.by_name() {
            self.rows.get(first..)?.iter().find_map(|row| match row {
                GridRow::Photos { indices, .. } => photos.get(indices.start).map(|photo| photo.filename.clone()),
                GridRow::Header { .. } => None,
            })
        } else {
            let first = first.min(self.rows.len().checked_sub(1)?);
            self.rows[..=first].iter().rev().find_map(|row| match row {
                GridRow::Header { title, .. } => Some(title.clone()),
                GridRow::Photos { .. } => None,
            })
        }
    }

    /// Index of the first photo in the first row of photos that is mostly
    /// in view when the view's top edge is at `top`. Scrolling to that photo
    /// finds the same row again.
    fn first_visible(&self, top: f32) -> Option<usize> {
        let first = self.rows.partition_point(|row| Self::row_y(row) + Self::row_height(row) / 2.0 <= top);
        self.rows.get(first..)?.iter().find_map(|row| match row {
            GridRow::Photos { indices, .. } => Some(indices.start),
            GridRow::Header { .. } => None,
        })
    }

    /// Month of the part of the grid at `y`, or "Unknown date" there, with
    /// how many photos it has.
    fn section_at(&self, y: f32) -> Option<(String, usize)> {
        let at = self.months.partition_point(|mark| mark.y <= y);
        let month = self.months.get(at.saturating_sub(1))?;
        let undated = self.undated.filter(|(top, _)| *top <= y && (at == 0 || month.y < *top));
        Some(match undated {
            Some((_, count)) => (tr("Unknown date").to_string(), count),
            None => (format_month(month.year, month.month), month.count),
        })
    }

    /// Title of the day heading the row at the top of a view at `top` sits
    /// under, and where to draw it pinned there: at `top`, or higher while
    /// the next heading pushes it out. `None` while that heading is itself
    /// in full view, or when sorted by name.
    fn pinned_heading(&self, top: f32) -> Option<(&str, f32)> {
        let at = self.rows.partition_point(|row| Self::row_y(row) <= top);
        let (index, title, y) = self.rows[..at].iter().enumerate().rev().find_map(|(index, row)| match row {
            GridRow::Header { title, y, .. } => Some((index, title.as_str(), *y)),
            GridRow::Photos { .. } => None,
        })?;
        if y >= top {
            return None;
        }
        let next = self.rows[index + 1..].iter().find_map(|row| match row {
            GridRow::Header { y, .. } => Some(*y),
            GridRow::Photos { .. } => None,
        });
        let push = next.map_or(0.0, |next| (next - (top + HEADER_H)).min(0.0));
        Some((title, top + push))
    }

    fn photo_y(&self, index: usize) -> Option<f32> {
        let at = self.rows.partition_point(|row| match row {
            GridRow::Header { first, .. } => *first <= index,
            GridRow::Photos { indices, .. } => indices.end <= index,
        });
        match self.rows.get(at)? {
            GridRow::Photos { indices, y, .. } if indices.contains(&index) => Some(*y),
            _ => None,
        }
    }
}

impl PicassoApp {
    pub(super) fn library_ui(&mut self, ui: &mut egui::Ui) {
        self.library_toolbar(ui);
        ui.add_space(4.0);
        if self.nav == Nav::Places {
            self.places_ui(ui);
            return;
        }
        if self.photos.is_empty() {
            if self.loading {
                ui.vertical_centered(|ui| {
                    ui.add_space(120.0);
                    ui.add(egui::Spinner::new().size(28.0));
                });
            } else {
                self.empty_library(ui);
            }
            return;
        }
        let full = ui.available_rect_before_wrap();
        let strip_w = if self.sort.by_name() { 0.0 } else { STRIP_W };
        let grid_rect = Rect::from_min_max(full.min, Pos2::new(full.max.x - strip_w, full.max.y));
        self.ensure_layout(grid_rect.width());
        let layout = Rc::clone(&self.layout.as_ref().unwrap().layout);
        self.grid_cols = layout.cols;
        let pending = self
            .scroll_to
            .and_then(|id| self.index_of(id))
            .and_then(|index| layout.photo_y(index))
            .map(|y| y - HEADER_H)
            .or(self.jump_to);
        self.scroll_to = None;
        self.jump_to = None;
        // Named, so the viewer's filmstrip in the same panel keeps its own
        // scroll position and the grid is where it was after the viewer.
        let mut area = egui::ScrollArea::vertical().id_salt("library-grid").auto_shrink([false, false]);
        // The date strip scrolls the grid itself, so a scroll bar beside it
        // would only repeat it.
        let scrubber = strip_w > 0.0 && !layout.months.is_empty();
        if scrubber {
            area = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        }
        if let Some(y) = pending {
            area = area.vertical_scroll_offset(y.max(0.0));
        }
        let mut grid = ui.new_child(egui::UiBuilder::new().max_rect(grid_rect));
        let output = area.show_viewport(&mut grid, |ui, viewport| {
            ui.set_height(layout.height);
            let first = layout
                .rows
                .partition_point(|row| GridLayout::row_y(row) + GridLayout::row_height(row) < viewport.min.y);
            let last = layout
                .rows
                .partition_point(|row| GridLayout::row_y(row) < viewport.max.y + 40.0)
                .min(layout.rows.len());
            let origin = ui.min_rect().min;
            for row in &layout.rows[first..last] {
                match row {
                    GridRow::Header { title, y, .. } => {
                        ui.painter().text(
                            origin + Vec2::new(PAD, *y + HEADER_H - 12.0),
                            Align2::LEFT_BOTTOM,
                            title,
                            FontId::proportional(15.0),
                            theme::text(),
                        );
                    }
                    GridRow::Photos { indices, y, h: _ } => {
                        let thumb = layout.tile;
                        let label_h = if layout.labels { LABEL_H } else { 0.0 };
                        for (col, index) in indices.clone().enumerate() {
                            let rect = Rect::from_min_size(
                                origin + Vec2::new(PAD + col as f32 * (thumb + layout.gap), *y),
                                Vec2::new(thumb, thumb + label_h),
                            );
                            self.photo_cell(ui, index, rect, layout.labels);
                        }
                    }
                }
            }
            // Asked for after the visible cells, so those are decoded first.
            let ahead = viewport.height();
            let above = layout
                .rows
                .partition_point(|row| GridLayout::row_y(row) + GridLayout::row_height(row) < viewport.min.y - ahead / 2.0);
            let below = layout
                .rows
                .partition_point(|row| GridLayout::row_y(row) < viewport.max.y + ahead)
                .max(last);
            for row in layout.rows[last..below].iter().chain(&layout.rows[above.min(first)..first]) {
                if let GridRow::Photos { indices, .. } = row {
                    for index in indices.clone() {
                        if let Some(photo) = self.photos.get(index) {
                            let (id, path) = (photo.id, photo.path.clone());
                            self.ensure_thumb(id, &path);
                        }
                    }
                }
            }
        });
        // Drawn over the scroll area rather than in it, so the fade egui
        // paints along its top edge doesn't wash the heading out.
        let view = output.inner_rect;
        if let Some((title, y)) = layout.pinned_heading(output.state.clamped_offset().y) {
            let top = view.top() + y - output.state.clamped_offset().y;
            let band = Rect::from_min_max(Pos2::new(view.left(), top), Pos2::new(view.right(), top + HEADER_H));
            grid.interact(band, egui::Id::unique("pinned-heading"), Sense::click());
            let painter = grid.painter().with_clip_rect(view);
            painter.rect_filled(band, 0, theme::bg());
            painter.line_segment([band.left_bottom(), band.right_bottom()], egui::Stroke::new(1.0, theme::hairline()));
            painter.text(
                Pos2::new(view.left() + PAD, band.bottom() - 12.0),
                Align2::LEFT_BOTTOM,
                title,
                FontId::proportional(15.0),
                theme::text(),
            );
        }
        ui.advance_cursor_after_rect(full);
        // The Library as a new window opens it, so the next start can come
        // back to the same photos.
        if self.nav == Nav::Library && self.search.is_empty() && self.sort == Sort::DateNewest && !self.loading {
            let top = layout.first_visible(output.state.clamped_offset().y);
            self.library_top = top.and_then(|index| self.photos.get(index)).map(|photo| photo.id);
        }
        self.scroll_hint(ui, &layout, &output);
        if scrubber {
            let strip = Rect::from_min_max(Pos2::new(grid_rect.max.x, full.min.y), full.max);
            self.date_strip(ui, strip, &layout, output.state.clamped_offset().y, output.inner_rect.height());
        }
    }

    /// Timeline of the library beside the grid, top to bottom as the grid
    /// is. Years sit where their photos are, months are ticks, and a bar
    /// marks the part in view. Pointing at it names the month there; a click
    /// goes to that month and dragging scrubs through the library. For a
    /// moment after scrolling it names the month in view.
    fn date_strip(&mut self, ui: &egui::Ui, rect: Rect, layout: &GridLayout, offset: f32, view_h: f32) {
        let marks = &layout.months;
        let track = Rect::from_min_max(
            Pos2::new(rect.left(), rect.top() + STRIP_PAD),
            Pos2::new(rect.right(), rect.bottom() - STRIP_PAD),
        );
        if marks.is_empty() || track.height() < 4.0 * YEAR_GAP {
            return;
        }
        let total = layout.height.max(1.0);
        let to_strip = |y: f32| track.top() + (y / total).clamp(0.0, 1.0) * track.height();
        let to_grid = |y: f32| ((y - track.top()) / track.height()).clamp(0.0, 1.0) * total;
        let line_x = track.right() - 12.0;

        let current = marks.partition_point(|mark| mark.y <= offset + HEADER_H).saturating_sub(1);
        let year_now = marks[current].year;
        let mut years: Vec<(i32, f32)> = Vec::new();
        for mark in marks {
            if years.last().is_none_or(|(year, _)| *year != mark.year) {
                years.push((mark.year, mark.y));
            }
        }
        // The year in view is always labelled; others only where they don't
        // crowd a label already placed.
        let mut labels: Vec<(i32, f32, f32)> = Vec::new();
        let by_priority = years.iter().filter(|(year, _)| *year == year_now).chain(years.iter().filter(|(year, _)| *year != year_now));
        for &(year, top) in by_priority {
            let at = to_strip(top);
            if labels.iter().all(|(_, other, _)| (other - at).abs() >= YEAR_GAP) {
                labels.push((year, at, top));
            }
        }

        let ctx = ui.ctx().clone();
        let scrollable = total > view_h;
        let max_offset = (total - view_h).max(0.0);
        let (handle_top, handle_bottom) = (to_strip(offset), to_strip(offset + view_h));
        let grow = ((HANDLE_MIN - (handle_bottom - handle_top)) / 2.0).max(0.0);
        let handle_span = (handle_top - grow)..=(handle_bottom + grow);

        let response = ui.interact(rect, egui::Id::unique("date-strip"), Sense::click_and_drag());
        if response.drag_started() {
            let origin = ui.input(|input| input.pointer.press_origin()).map(|pos| pos.y);
            self.strip_glide = None;
            self.strip_grab = origin.filter(|y| scrollable && handle_span.contains(y)).map(|y| y - handle_top);
        }
        if !response.dragged() {
            self.strip_grab = None;
        }
        let active = response.dragged() || response.is_pointer_button_down_on();
        let pointer_y = if active {
            ui.input(|input| input.pointer.latest_pos()).map(|pos| pos.y.clamp(track.top(), track.bottom()))
        } else {
            response.hover_pos().map(|pos| pos.y)
        };
        let over_handle = scrollable && pointer_y.is_some_and(|y| handle_span.contains(&y));
        let cursor = if self.strip_grab.is_some() {
            egui::CursorIcon::Grabbing
        } else if over_handle {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::PointingHand
        };
        let response = response.on_hover_cursor(cursor);

        // Where the pointer leads: dragging the handle keeps the spot it was
        // taken by under the pointer; elsewhere a year label leads to the
        // start of its year rather than to whatever month the exact pixel
        // under it falls in.
        let target = pointer_y.map(|y| match self.strip_grab {
            Some(grab) => to_grid(y - grab) + HEADER_H,
            None => labels
                .iter()
                .find(|(_, at, _)| (at - y).abs() < YEAR_GAP / 2.0)
                .map_or_else(|| to_grid(y), |(_, _, top)| *top),
        });
        if let Some(target) = target {
            if response.dragged() {
                self.jump_to = Some((target - HEADER_H).clamp(0.0, max_offset));
            } else if response.clicked() {
                let at = marks.partition_point(|mark| mark.y <= target).saturating_sub(1);
                let to = (marks[at].y - 8.0).clamp(0.0, max_offset);
                self.strip_glide = Some((offset, to, Instant::now()));
            }
        }
        if let Some((from, to, start)) = self.strip_glide {
            let wheel = ui.input(|input| input.smooth_scroll_delta.y != 0.0);
            let t = (start.elapsed().as_secs_f32() / STRIP_GLIDE).min(1.0);
            if wheel {
                self.strip_glide = None;
            } else {
                let eased = 1.0 - (1.0 - t).powi(3);
                self.jump_to = Some(from + (to - from) * eased);
                if t >= 1.0 {
                    self.strip_glide = None;
                }
                ctx.request_repaint();
            }
        }

        if (offset - self.strip_offset).abs() > 0.5 {
            self.strip_offset = offset;
            self.strip_moved_at = Some(Instant::now());
        }
        let hot = ctx.animate_bool_with_time(egui::Id::unique("date-strip-hot"), response.hovered() || active, STRIP_WAKE);
        let awake = ctx.animate_bool_with_time(
            egui::Id::unique("date-strip-awake"),
            response.hovered() || active || self.strip_moved_at.is_some(),
            STRIP_WAKE,
        );

        let painter = ui.painter_at(rect);
        if hot > 0.0 {
            painter.rect_filled(rect.shrink2(Vec2::new(4.0, 2.0)), 10, theme::overlay((10.0 * hot) as u8));
        }
        painter.line_segment(
            [Pos2::new(line_x, track.top()), Pos2::new(line_x, track.bottom())],
            egui::Stroke::new(1.0, theme::hairline().gamma_multiply(0.8 + 0.2 * awake)),
        );
        let (month_tick, year_tick) = (theme::muted().gamma_multiply(0.35 + 0.25 * awake), theme::muted().gamma_multiply(0.7 + 0.3 * awake));
        for (index, mark) in marks.iter().enumerate() {
            let new_year = index == 0 || marks[index - 1].year != mark.year;
            let (half, color) = if new_year { (4.0 + hot, year_tick) } else { (2.0 + hot, month_tick) };
            let y = to_strip(mark.y);
            painter.line_segment([Pos2::new(line_x - half, y), Pos2::new(line_x + half, y)], egui::Stroke::new(1.0, color));
        }
        for &(year, at, _) in &labels {
            let (size, color) = if year == year_now {
                (12.5, ACCENT)
            } else {
                (11.5, theme::muted().gamma_multiply(0.8 + 0.2 * awake))
            };
            painter.text(Pos2::new(line_x - 10.0, at), Align2::RIGHT_CENTER, year.to_string(), FontId::proportional(size), color);
        }
        if let (Some(y), false) = (pointer_y, self.strip_grab.is_some() || over_handle) {
            painter.line_segment(
                [Pos2::new(rect.left() + 8.0, y), Pos2::new(line_x + 6.0, y)],
                egui::Stroke::new(1.0, theme::text().gamma_multiply(0.3 * hot)),
            );
        }
        if scrollable {
            let half_w = 2.0 + 1.5 * hot;
            let handle = Rect::from_min_max(
                Pos2::new(line_x - half_w, (handle_top - grow).max(track.top())),
                Pos2::new(line_x + half_w, (handle_bottom + grow).min(track.bottom())),
            );
            if self.strip_grab.is_some() {
                painter.rect_filled(handle.expand(3.0), 6, ACCENT.gamma_multiply(0.2));
            }
            painter.rect_filled(handle, half_w, ACCENT.gamma_multiply(0.75 + 0.25 * awake));
        }

        let tip_x = rect.left() + 2.0;
        if let (Some(y), Some(target)) = (pointer_y, target) {
            if let Some((title, count)) = layout.section_at(target) {
                paint_date_bubble(&ctx, title, Some(super::photo_count(count)), Pos2::new(tip_x, y), rect, 1.0);
            }
        } else if let Some(at) = self.strip_moved_at {
            let alpha = 1.0 - ((at.elapsed().as_secs_f32() - STRIP_HINT_HOLD) / SCROLL_HINT_FADE).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                self.strip_moved_at = None;
            } else if let Some((title, count)) = layout.section_at(offset + HEADER_H) {
                ctx.request_repaint();
                let y = (handle_top + handle_bottom) / 2.0;
                paint_date_bubble(&ctx, title, Some(super::photo_count(count)), Pos2::new(tip_x, y), rect, alpha);
            }
        }
    }

    /// Bubble beside the scroll bar handle while it is dragged, naming the
    /// part of the library in view. It fades out after the drag ends.
    fn scroll_hint(&mut self, ui: &egui::Ui, layout: &GridLayout, output: &egui::scroll_area::ScrollAreaOutput<()>) {
        let ctx = ui.ctx();
        if ctx.is_being_dragged(output.id.with(1)) {
            self.scroll_hint_at = Some(Instant::now());
        }
        let Some(at) = self.scroll_hint_at else { return };
        let alpha = 1.0 - (at.elapsed().as_secs_f32() / SCROLL_HINT_FADE).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            self.scroll_hint_at = None;
            return;
        }
        ctx.request_repaint();
        let view = output.inner_rect;
        let content = output.content_size.y;
        if content <= view.height() {
            return;
        }
        let offset = output.state.clamped_offset().y;
        let Some(text) = layout.position_label(&self.photos, self.sort, offset) else { return };

        // The handle as egui places it, so the bubble follows the pointer.
        let style = &ui.spacing().scroll;
        let handle = (view.height() * view.height() / content).max(style.handle_min_length);
        let progress = offset / (content - view.height());
        let handle_y = view.top() + handle / 2.0 + progress.clamp(0.0, 1.0) * (view.height() - handle);
        let bar_left = view.right() - style.bar_outer_margin - style.bar_width;
        paint_date_bubble(ctx, text, None, Pos2::new(bar_left - 4.0, handle_y), view, alpha);
    }

    fn ensure_layout(&mut self, width: f32) {
        let width_key = width.round() as i32;
        let thumb_key = self.thumb_size.round() as i32;
        let stale = self.layout.as_ref().map_or(true, |cache| {
            cache.epoch != self.view_epoch
                || cache.width != width_key
                || cache.sort != self.sort
                || cache.thumb != thumb_key
        });
        if stale {
            self.layout = Some(CachedLayout {
                epoch: self.view_epoch,
                width: width_key,
                sort: self.sort,
                thumb: thumb_key,
                layout: Rc::new(build_layout(&self.photos, self.sort, width, self.thumb_size)),
            });
        }
    }

    fn library_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.view_title()).size(22.0).strong().color(theme::text()));
            ui.label(egui::RichText::new(super::photo_count(self.photos.len())).size(13.0).color(theme::muted()));
            if self.loading {
                ui.add(egui::Spinner::new().size(13.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut sort = self.sort;
                egui::ComboBox::from_id_salt("sort")
                    .selected_text(tr(match sort {
                        Sort::DateNewest => "Sort by Date",
                        Sort::DateOldest => "Sort by Date, oldest",
                        Sort::Name => "Sort by Name",
                        Sort::NameEdited => "Sort by Name, edited",
                    }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sort, Sort::DateNewest, tr("Sort by Date"));
                        ui.selectable_value(&mut sort, Sort::DateOldest, tr("Sort by Date, oldest"));
                        ui.selectable_value(&mut sort, Sort::Name, tr("Sort by Name"));
                        ui.selectable_value(&mut sort, Sort::NameEdited, tr("Sort by Name, edited"))
                            .on_hover_text(tr("Only photos with edits"));
                    });
                if sort != self.sort {
                    self.sort = sort;
                    self.reload_view();
                }
                if ui
                    .button(egui::RichText::new("★").color(STAR))
                    .on_hover_text(tr("Select starred photos"))
                    .clicked()
                {
                    self.select_starred();
                }
                if !self.selection.is_empty() {
                    ui.add(egui::Separator::default().vertical().spacing(10.0));
                    let starred = self.primary_starred();
                    let star = egui::RichText::new(if starred { "★" } else { "☆" }).color(if starred { STAR } else { theme::text() });
                    if ui.button(star).on_hover_text(tr(if starred { "Unstar" } else { "Star" })).clicked() {
                        self.toggle_star_selection();
                    }
                    let rotate = theme::icon_button(ui, theme::Icon::RotateRight, true, false);
                    if rotate.on_hover_text(tr("Rotate clockwise (Ctrl+])")).clicked() {
                        self.rotate_current(90);
                    }
                    let rotate = theme::icon_button(ui, theme::Icon::RotateLeft, true, false);
                    if rotate.on_hover_text(tr("Rotate counterclockwise (Ctrl+[)")).clicked() {
                        self.rotate_current(-90);
                    }
                    if matches!(self.nav, Nav::Album(_)) {
                        let remove = theme::icon_button(ui, theme::Icon::RemoveFromAlbum, true, false);
                        if remove.on_hover_text(tr("Remove from Album")).clicked() {
                            self.remove_selection_from_album();
                        }
                    }
                    self.album_menu(ui);
                }
            });
        });
        ui.add_space(6.0);
    }

    fn album_menu(&mut self, ui: &mut egui::Ui) {
        let albums: Vec<(i64, String)> = self.albums.iter().map(|a| (a.id, a.name.clone())).collect();
        let mut create = false;
        let mut add: Option<i64> = None;
        let button = theme::icon_button(ui, theme::Icon::AddToAlbum, true, false);
        let popup = egui::Popup::menu(&button).show(|ui| {
            for (id, name) in &albums {
                if ui.selectable_label(false, name).clicked() {
                    add = Some(*id);
                }
            }
            if !albums.is_empty() {
                ui.separator();
            }
            if ui.button(tr("New album…")).clicked() {
                create = true;
            }
        });
        if popup.is_none() {
            button.on_hover_text(tr("Add to Album"));
        }
        if let Some(id) = add {
            let photos: Vec<i64> = self.selection.iter().copied().collect();
            self.db.add_to_album(id, &photos);
            self.albums = self.db.albums();
            self.activity = trf("Added {} to album", &[&photos.len()]);
        }
        if create {
            self.modal = super::Modal::NewAlbum {
                name: String::new(),
                error: String::new(),
            };
        }
    }

    fn photo_cell(&mut self, ui: &mut egui::Ui, index: usize, rect: Rect, labeled: bool) {
        let Some(photo) = self.photos.get(index).cloned() else {
            return;
        };
        self.ensure_thumb(photo.id, &photo.path);
        let selected = self.selection.contains(&photo.id);
        let thumb = rect.width();
        let image_rect = Rect::from_min_size(rect.min, Vec2::splat(thumb));
        let response = ui
            .interact(rect, egui::Id::unique(("photo", photo.id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let painter = ui.painter();
        if response.hovered() {
            painter.add(theme::card_shadow().as_shape(image_rect, THUMB_RADIUS));
        }
        painter.rect_filled(image_rect, THUMB_RADIUS, theme::placeholder());
        if let Some((texture, w, h)) = self.thumb_texture(photo.id) {
            theme::paint_image_rounded(
                ui.painter(),
                &texture,
                image_rect,
                theme::cover_uv(w, h),
                photo.rotation,
                THUMB_RADIUS as f32,
            );
        }
        let painter = ui.painter();
        if !labeled && response.hovered() {
            theme::paint_bottom_fade(painter, image_rect, THUMB_RADIUS as f32, 44.0, 150);
        }
        let stroke = if selected {
            egui::Stroke::new(3.0, ACCENT)
        } else {
            egui::Stroke::new(1.0, Color32::from_black_alpha(18))
        };
        painter.rect_stroke(image_rect, THUMB_RADIUS, stroke, egui::StrokeKind::Inside);
        let check_rect = Rect::from_center_size(image_rect.left_top() + Vec2::splat(15.0), Vec2::splat(24.0));
        let over_check = |pos: Option<Pos2>| pos.is_some_and(|pos| check_rect.contains(pos));
        if !selected && (response.hovered() || self.selection.len() > 1) {
            let center = check_rect.center();
            let ring = if over_check(response.hover_pos()) { WHITE } else { WHITE.gamma_multiply(0.85) };
            painter.circle_filled(center, 9.0, Color32::from_black_alpha(if over_check(response.hover_pos()) { 90 } else { 50 }));
            painter.circle_stroke(center, 9.0, egui::Stroke::new(1.6, ring));
        }
        if selected {
            let center = check_rect.center();
            painter.circle_filled(center, 9.0, ACCENT);
            painter.circle_stroke(center, 9.0, egui::Stroke::new(2.0, WHITE));
            painter.add(egui::Shape::line(
                vec![
                    center + Vec2::new(-4.0, 0.0),
                    center + Vec2::new(-1.0, 3.0),
                    center + Vec2::new(4.0, -3.0),
                ],
                egui::Stroke::new(2.0, WHITE),
            ));
        }
        if photo.starred {
            let center = image_rect.right_bottom() + Vec2::new(-14.0, -14.0);
            painter.circle_filled(center, 11.0, Color32::from_black_alpha(110));
            painter.text(center, Align2::CENTER_CENTER, "★", FontId::proportional(14.0), STAR);
        }
        if photo.is_video() {
            super::viewer::paint_play_button(painter, image_rect.center(), (thumb * 0.14).clamp(14.0, 24.0));
            if photo.duration > 0.0 {
                let galley = painter.layout_no_wrap(
                    crate::types::format_duration(photo.duration),
                    FontId::proportional(11.0),
                    WHITE,
                );
                let pill = Rect::from_min_size(
                    image_rect.right_top() + Vec2::new(-galley.size().x - 14.0, 6.0),
                    galley.size() + Vec2::new(10.0, 4.0),
                );
                painter.rect_filled(pill, 5, Color32::from_black_alpha(150));
                painter.galley(pill.min + Vec2::new(5.0, 2.0), galley, WHITE);
            }
        }
        let place_rect = Rect::from_center_size(image_rect.left_bottom() + Vec2::new(14.0, -14.0), Vec2::splat(22.0));
        if photo.place().is_some() {
            painter.circle_filled(place_rect.center(), 11.0, Color32::from_black_alpha(110));
            theme::paint_pin(painter, place_rect.center(), 4.6, WHITE, Color32::from_black_alpha(110));
        }
        let edit_rect = Rect::from_center_size(image_rect.right_top() + Vec2::new(-14.0, 14.0), Vec2::splat(22.0));
        let edited = self.edits.contains_key(&photo.id);
        if edited {
            theme::paint_edit_mark(painter, edit_rect.center(), 11.0);
        }
        if labeled {
            painter.text(
                Pos2::new(image_rect.center().x, image_rect.bottom() + 5.0),
                Align2::CENTER_TOP,
                elide(&photo.filename, ((thumb / 7.0) as usize).max(8)),
                FontId::proportional(11.5),
                theme::muted(),
            );
        } else if response.hovered() {
            // Clear of the place and star badges in the bottom corners.
            let badges = photo.place().is_some() || photo.starred;
            let mut job = egui::text::LayoutJob::simple_singleline(photo.filename.clone(), FontId::proportional(11.5), WHITE);
            job.wrap = egui::text::TextWrapping::truncate_at_width(thumb - if badges { 64.0 } else { 16.0 });
            let galley = painter.layout_job(job);
            let at = Pos2::new(image_rect.center().x - galley.size().x / 2.0, image_rect.bottom() - 8.0 - galley.size().y);
            painter.galley(at, galley, WHITE);
        }
        let star_rect = Rect::from_min_size(image_rect.right_bottom() + Vec2::new(-22.0, -22.0), Vec2::splat(22.0));
        let on_star = response
            .interact_pointer_pos()
            .map(|pos| star_rect.contains(pos) && photo.starred)
            .unwrap_or(false);
        let over_place = |pos: Option<Pos2>| photo.place().is_some() && pos.is_some_and(|pos| place_rect.contains(pos));
        let over_edit = |pos: Option<Pos2>| edited && pos.is_some_and(|pos| edit_rect.contains(pos));
        let response = if over_place(response.hover_pos()) {
            let text = if photo.pinned() { "Geotagged in Picasso — show on map" } else { "Has GPS location — show on map" };
            response.on_hover_text(tr(text))
        } else if over_edit(response.hover_pos()) {
            response.on_hover_text(tr("Edited — open in the editor"))
        } else if over_check(response.hover_pos()) {
            response.on_hover_text(tr("Select"))
        } else {
            response
        };
        if response.clicked() {
            let modifiers = ui.input(|input| input.modifiers);
            if over_check(response.interact_pointer_pos()) {
                // Adds to the selection or takes the photo out, as a
                // Command-click does.
                self.select_click(index, egui::Modifiers::COMMAND);
            } else if on_star {
                self.toggle_star(photo.id);
            } else if over_edit(response.interact_pointer_pos()) {
                self.open_at(index);
                self.editing = true;
            } else if over_place(response.interact_pointer_pos()) {
                self.select_click(index, egui::Modifiers::NONE);
                if self.side != super::SidePanel::Map {
                    self.toggle_side(super::SidePanel::Map);
                }
            } else if response.double_clicked() {
                self.open_at(index);
            } else {
                self.select_click(index, modifiers);
            }
        }
        let path = photo.path.clone();
        let id = photo.id;
        response.context_menu(|ui| {
            if ui.button(tr("Open")).clicked() {
                self.open_at(index);
                ui.close();
            }
            if !photo.is_video() && ui.button(tr("Edit")).clicked() {
                self.open_at(index);
                self.editing = true;
                ui.close();
            }
            if ui.button(tr(if photo.starred { "Unstar" } else { "Star" })).clicked() {
                self.toggle_star(id);
                ui.close();
            }
            if ui.button(tr("Rotate Clockwise")).clicked() {
                self.rotate_from_menu(id, 90);
                ui.close();
            }
            if ui.button(tr("Rotate Counterclockwise")).clicked() {
                self.rotate_from_menu(id, -90);
                ui.close();
            }
            if self.edits.contains_key(&id) && ui.button(tr("Copy edit settings")).clicked() {
                self.copy_settings(Some(id));
                ui.close();
            }
            if self.edit_clipboard.is_some() && ui.button(tr("Paste settings")).clicked() {
                let ids: Vec<i64> = if self.selection.contains(&id) {
                    self.selection.iter().copied().collect()
                } else {
                    vec![id]
                };
                self.paste_settings(&ids);
                ui.close();
            }
            if ui.button(tr("Export…")).clicked() {
                let ids = if self.selection.contains(&id) { self.selection_in_order() } else { vec![id] };
                self.open_export(&ids);
                ui.close();
            }
            if ui.button(tr("Adjust date…")).clicked() {
                let ids = if self.selection.contains(&id) { self.selection_in_order() } else { vec![id] };
                self.open_adjust_date(ids);
                ui.close();
            }
            if ui.button(tr("Show in folder")).clicked() {
                crate::shell::reveal(&path);
                ui.close();
            }
            if ui.button(tr("Show on map")).clicked() {
                self.show_on_map(id);
                ui.close();
            }
            if ui.button(tr("Move to trash")).clicked() {
                self.modal = super::Modal::Trash { ids: vec![id] };
                ui.close();
            }
        });
    }

    fn empty_library(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(96.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            theme::paint_logo(ui.painter(), &self.logo, rect);
            ui.add_space(16.0);
            if self.sort.edited_only() {
                ui.label(egui::RichText::new(tr("No edited photos")).size(20.0).strong().color(theme::text()));
                ui.add_space(6.0);
                let hint = tr("Photos you edit show up here. Choose another sort order to see every photo.");
                ui.label(egui::RichText::new(hint).color(theme::muted()));
                return;
            }
            let title = if self.search.trim().is_empty() { "No photos found" } else { "No results found" };
            ui.label(egui::RichText::new(tr(title)).size(20.0).strong().color(theme::text()));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tr("Import a folder and Picasso scans it for pictures.")).color(theme::muted()));
            ui.add_space(16.0);
            if ui.add(theme::primary_button(tr("+  Import folder"))).clicked() {
                self.modal = super::Modal::AddFolder {
                    path: String::new(),
                    error: String::new(),
                };
            }
        });
    }
}

/// How many tiles a row of `width` holds and how wide each is: the count
/// that keeps them nearest `thumb`, then widened or narrowed a little so the
/// row ends at the right edge instead of leaving a gap there.
fn grid_columns(width: f32, thumb: f32, gap: f32) -> (usize, f32) {
    let room = (width - PAD * 2.0).max(thumb);
    let cols = ((room + gap) / (thumb + gap)).round().max(1.0) as usize;
    let tile = (room - gap * (cols - 1) as f32) / cols as f32;
    // A single column can be far wider than asked; past this it stays
    // short of the edge rather than blow the thumbnail up.
    (cols, tile.min(thumb * 1.3))
}

fn build_layout(photos: &[Photo], sort: Sort, width: f32, thumb: f32) -> GridLayout {
    let labels = sort.by_name();
    let gap = if labels { GAP } else { TIGHT_GAP };
    let (cols, tile) = grid_columns(width, thumb, gap);
    let row_h = tile + if labels { LABEL_H } else { 0.0 } + gap;
    let mut rows = Vec::new();
    let mut months: Vec<MonthMark> = Vec::new();
    let mut undated = None;
    let mut y = 8.0;
    if sort.by_name() {
        let mut offset = 0;
        for chunk in photos.chunks(cols) {
            let indices = offset..offset + chunk.len();
            offset += chunk.len();
            rows.push(GridRow::Photos { indices, y, h: row_h });
            y += row_h;
        }
    } else {
        let mut index = 0;
        while index < photos.len() {
            let day = day_key(photos[index].taken_at);
            let start = index;
            index += 1;
            while index < photos.len() && day_key(photos[index].taken_at) == day {
                index += 1;
            }
            if let Some(date) = day.and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
                use chrono::Datelike;
                let (year, month) = (date.year(), date.month());
                match months.last_mut() {
                    Some(last) if (last.year, last.month) == (year, month) => last.count += index - start,
                    _ => months.push(MonthMark { year, month, y, count: index - start }),
                }
            } else {
                undated.get_or_insert((y, 0)).1 += index - start;
            }
            rows.push(GridRow::Header {
                title: format_day(photos[start].taken_at),
                y,
                first: start,
            });
            y += HEADER_H;
            let mut cursor = start;
            while cursor < index {
                let end = (cursor + cols).min(index);
                rows.push(GridRow::Photos {
                    indices: cursor..end,
                    y,
                    h: row_h,
                });
                y += row_h;
                cursor = end;
            }
        }
    }
    GridLayout {
        rows,
        height: y + 24.0,
        cols,
        tile,
        gap,
        labels,
        months,
        undated,
    }
}

/// Rounded label over the grid naming a part of the library, with an
/// optional smaller second line. Its pointer's tip is at `tip`, the bubble
/// to its left, kept within `bounds` vertically while the pointer follows.
fn paint_date_bubble(ctx: &egui::Context, title: String, detail: Option<String>, tip: Pos2, bounds: Rect, alpha: f32) {
    const NOTCH: f32 = 6.0;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("date-bubble")));
    let color = theme::text().gamma_multiply(alpha);
    let title = painter.layout_no_wrap(title, FontId::proportional(14.0), color);
    let detail = detail.map(|text| painter.layout_no_wrap(text, FontId::proportional(11.5), theme::muted().gamma_multiply(alpha)));
    let text_w = detail.as_ref().map_or(title.size().x, |detail| title.size().x.max(detail.size().x));
    let text_h = title.size().y + detail.as_ref().map_or(0.0, |detail| detail.size().y + 1.0);
    let size = Vec2::new(text_w + 28.0, text_h + 16.0);
    let top = (tip.y - size.y / 2.0).clamp(bounds.top() + 4.0, (bounds.bottom() - size.y - 4.0).max(bounds.top() + 4.0));
    let bubble = Rect::from_min_size(Pos2::new(tip.x - NOTCH - size.x, top), size);
    let mut shadow = theme::card_shadow();
    shadow.color = shadow.color.gamma_multiply(alpha);
    painter.add(shadow.as_shape(bubble, 10));
    let fill = theme::bg().gamma_multiply(alpha);
    let line = egui::Stroke::new(1.0, theme::hairline().gamma_multiply(alpha));
    painter.rect(bubble, 10, fill, line, egui::StrokeKind::Inside);
    let notch_y = tip.y.clamp(bubble.top() + 14.0, bubble.bottom() - 14.0);
    let (upper, lower) = (Pos2::new(bubble.right() - 1.0, notch_y - NOTCH), Pos2::new(bubble.right() - 1.0, notch_y + NOTCH));
    let point = Pos2::new(bubble.right() + NOTCH - 1.0, notch_y);
    painter.add(egui::Shape::convex_polygon(vec![upper, point, lower], fill, egui::Stroke::NONE));
    painter.line_segment([upper, point], line);
    painter.line_segment([point, lower], line);
    let title_x = bubble.left() + 14.0;
    let title_y = bubble.top() + 8.0;
    let title_h = title.size().y;
    painter.galley(Pos2::new(title_x, title_y), title, color);
    if let Some(detail) = detail {
        painter.galley(Pos2::new(title_x, title_y + title_h + 1.0), detail, color);
    }
}

fn elide(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_string()
    } else {
        let trimmed: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{trimmed}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(name: &str, taken_at: i64) -> Photo {
        Photo {
            id: 0,
            folder_id: 0,
            path: String::new(),
            filename: name.into(),
            width: 0,
            height: 0,
            bytes: 0,
            taken_at,
            modified_at: 0,
            starred: false,
            caption: String::new(),
            camera: String::new(),
            gps_lat: None,
            gps_lon: None,
            pin_lat: None,
            pin_lon: None,
            rotation: 0,
            duration: 0.0,
        }
    }

    #[test]
    fn labels_the_top_of_the_view() {
        // Noon UTC, two days apart, so both stay separate days in any zone.
        let (day_a, day_b) = (1_780_000_000 / 86_400 * 86_400 + 43_200, 1_780_000_000 / 86_400 * 86_400 - 129_600);
        let photos: Vec<Photo> = ["a", "b", "c", "d", "e"]
            .iter()
            .enumerate()
            .map(|(index, name)| photo(name, if index < 3 { day_a } else { day_b }))
            .collect();
        // Two columns of 100 px thumbnails. By date, rows are 106 px with no
        // names under them and headers 40 px; by name, rows are 136 px.
        let dated = build_layout(&photos, Sort::DateNewest, 210.0, 100.0);
        assert_eq!((dated.cols, dated.tile, dated.labels), (2, 100.0, false));
        let (title_a, title_b) = (format_day(day_a), format_day(day_b));
        assert_eq!(dated.position_label(&photos, Sort::DateNewest, 0.0), Some(title_a.clone()));
        assert_eq!(dated.position_label(&photos, Sort::DateNewest, 200.0), Some(title_a));
        assert_eq!(dated.position_label(&photos, Sort::DateNewest, 330.0), Some(title_b.clone()));
        assert_eq!(dated.position_label(&photos, Sort::DateNewest, 10_000.0), Some(title_b));

        let named = build_layout(&photos, Sort::Name, 218.0, 100.0);
        assert_eq!((named.cols, named.tile, named.labels), (2, 100.0, true));
        assert_eq!(named.position_label(&photos, Sort::Name, 0.0).as_deref(), Some("a"));
        assert_eq!(named.position_label(&photos, Sort::Name, 150.0).as_deref(), Some("c"));
        assert_eq!(named.position_label(&photos, Sort::Name, 10_000.0), None);
    }

    #[test]
    fn tiles_fill_the_row() {
        assert_eq!(grid_columns(218.0, 100.0, GAP), (2, 100.0), "an exact fit");
        let (cols, tile) = grid_columns(544.0, 176.0, GAP);
        assert_eq!(cols, 3, "a third column that nearly fits is added");
        assert!((tile * 3.0 + GAP * 2.0 - 540.0).abs() < 0.01);
        assert_eq!(grid_columns(404.0, 176.0, GAP), (2, 193.0), "two columns widen rather than leave a gap");
        assert_eq!(grid_columns(204.0, 150.0, GAP), (1, 195.0), "a lone column grows by at most 30%");
    }

    #[test]
    fn names_the_month_at_each_part_of_the_grid() {
        // Mid-month noons in UTC stay in the same month in any zone.
        let march = chrono::NaiveDate::from_ymd_opt(2026, 3, 15).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        let january = chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        let photos: Vec<Photo> = [march, march, january, i64::MAX].iter().map(|&at| photo("p", at)).collect();
        let layout = build_layout(&photos, Sort::DateNewest, 210.0, 100.0);
        assert_eq!(layout.months.len(), 2);
        let (first, second, (undated, _)) = (layout.months[0].y, layout.months[1].y, layout.undated.unwrap());
        assert!(first < second && second < undated);
        assert_eq!(layout.section_at(0.0), Some((format_month(2026, 3), 2)));
        assert_eq!(layout.section_at(second - 1.0), Some((format_month(2026, 3), 2)));
        assert_eq!(layout.section_at(second + 1.0), Some((format_month(2026, 1), 1)));
        assert_eq!(layout.section_at(undated + 1.0), Some((tr("Unknown date").to_string(), 1)));
        assert!(build_layout(&photos, Sort::Name, 218.0, 100.0).months.is_empty());
    }

    #[test]
    fn pins_the_heading_of_the_rows_in_view() {
        let noon = 1_780_000_000 / 86_400 * 86_400 + 43_200;
        let photos: Vec<Photo> = [noon, noon, noon, noon - 2 * 86_400].iter().map(|&at| photo("p", at)).collect();
        // Two columns: heading at 8, rows at 48 and 154, next heading at 260.
        let layout = build_layout(&photos, Sort::DateNewest, 210.0, 100.0);
        let (first, second) = (format_day(noon), format_day(noon - 2 * 86_400));
        assert_eq!(layout.pinned_heading(0.0), None, "the heading is in full view");
        assert_eq!(layout.pinned_heading(100.0), Some((first.as_str(), 100.0)));
        assert_eq!(layout.pinned_heading(240.0), Some((first.as_str(), 220.0)), "pushed up by the next heading");
        assert_eq!(layout.pinned_heading(270.0), Some((second.as_str(), 270.0)));
        assert_eq!(build_layout(&photos, Sort::Name, 218.0, 100.0).pinned_heading(100.0), None);
    }

    #[test]
    fn headings_follow_the_named_days() {
        let noon = 1_780_000_000 / 86_400 * 86_400 + 43_200;
        let times = [noon + 3_600, noon, noon - 60, noon - 86_400, noon - 86_400 - 7_200, noon - 5 * 86_400, i64::MAX, i64::MAX];
        let photos: Vec<Photo> = times.iter().map(|&at| photo("p", at)).collect();
        let layout = build_layout(&photos, Sort::DateNewest, 210.0, 100.0);

        let mut expected: Vec<(String, usize)> = Vec::new();
        for (index, photo) in photos.iter().enumerate() {
            let title = format_day(photo.taken_at);
            if expected.last().is_none_or(|(last, _)| *last != title) {
                expected.push((title, index));
            }
        }
        let headings: Vec<(String, usize)> = layout
            .rows
            .iter()
            .filter_map(|row| match row {
                GridRow::Header { title, first, .. } => Some((title.clone(), *first)),
                GridRow::Photos { .. } => None,
            })
            .collect();
        assert_eq!(headings, expected);

        for index in 0..photos.len() {
            let row = layout.rows.iter().find(|row| matches!(row, GridRow::Photos { indices, .. } if indices.contains(&index)));
            assert_eq!(layout.photo_y(index), row.map(GridLayout::row_y), "photo {index}");
        }
        assert_eq!(layout.photo_y(photos.len()), None);
    }
}
