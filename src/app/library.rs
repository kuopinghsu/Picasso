//! Library grid and the browse toolbar.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::i18n::{tr, trf};
use crate::types::{format_day, Nav, Photo, Sort};

use super::theme::{self, ACCENT, STAR, THUMB_RADIUS, WHITE};
use super::PicassoApp;

const GAP: f32 = 14.0;
const LABEL_H: f32 = 22.0;
const HEADER_H: f32 = 40.0;
const PAD: f32 = 2.0;
/// Seconds the scroll bar bubble takes to fade after the drag ends.
const SCROLL_HINT_FADE: f32 = 0.8;

pub(super) struct CachedLayout {
    pub epoch: u64,
    pub width: i32,
    pub sort: Sort,
    pub thumb: i32,
    pub layout: GridLayout,
}

#[derive(Clone)]
pub(super) struct GridLayout {
    rows: Vec<GridRow>,
    height: f32,
    pub cols: usize,
}

#[derive(Clone)]
enum GridRow {
    Header { title: String, y: f32 },
    Photos {
        indices: Vec<usize>,
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
        if sort == Sort::Name {
            self.rows.get(first..)?.iter().find_map(|row| match row {
                GridRow::Photos { indices, .. } => photos.get(*indices.first()?).map(|photo| photo.filename.clone()),
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

    fn photo_y(&self, index: usize) -> Option<f32> {
        for row in &self.rows {
            if let GridRow::Photos { indices, y, .. } = row {
                if indices.contains(&index) {
                    return Some(*y);
                }
            }
        }
        None
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
            self.empty_library(ui);
            return;
        }
        let width = ui.available_width();
        self.ensure_layout(width);
        let layout = self.layout.as_ref().unwrap().layout.clone();
        self.grid_cols = layout.cols;
        let pending = self.scroll_to.and_then(|id| {
            self.photos
                .iter()
                .position(|photo| photo.id == id)
                .and_then(|index| layout.photo_y(index))
        });
        self.scroll_to = None;
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(y) = pending {
            area = area.vertical_scroll_offset((y - 24.0).max(0.0));
        }
        let output = area.show_viewport(ui, |ui, viewport| {
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
                        let thumb = self.thumb_size;
                        for (col, index) in indices.iter().enumerate() {
                            let rect = Rect::from_min_size(
                                origin + Vec2::new(PAD + col as f32 * (thumb + GAP), *y),
                                Vec2::new(thumb, thumb + LABEL_H),
                            );
                            self.photo_cell(ui, *index, rect);
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
                    for &index in indices {
                        if let Some(photo) = self.photos.get(index) {
                            let (id, path) = (photo.id, photo.path.clone());
                            self.ensure_thumb(id, &path);
                        }
                    }
                }
            }
        });
        self.scroll_hint(ui, &layout, &output);
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

        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("scroll-hint")));
        let color = theme::text().gamma_multiply(alpha);
        let galley = painter.layout_no_wrap(text, FontId::proportional(14.0), color);
        let size = galley.size() + Vec2::new(28.0, 16.0);
        let top = (handle_y - size.y / 2.0).clamp(view.top() + 4.0, view.bottom() - size.y - 4.0);
        let bubble = Rect::from_min_size(Pos2::new(bar_left - 10.0 - size.x, top), size);
        let mut shadow = theme::card_shadow();
        shadow.color = shadow.color.gamma_multiply(alpha);
        painter.add(shadow.as_shape(bubble, 10));
        painter.rect(
            bubble,
            10,
            theme::bg().gamma_multiply(alpha),
            egui::Stroke::new(1.0, theme::hairline().gamma_multiply(alpha)),
            egui::StrokeKind::Inside,
        );
        painter.galley(bubble.min + Vec2::new(14.0, 8.0), galley, color);
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
                layout: build_layout(&self.photos, self.sort, width, self.thumb_size),
            });
        }
    }

    fn library_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.view_title()).size(22.0).strong().color(theme::text()));
            ui.label(egui::RichText::new(super::photo_count(self.photos.len())).size(13.0).color(theme::muted()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut sort = self.sort;
                egui::ComboBox::from_id_salt("sort")
                    .selected_text(tr(match sort {
                        Sort::DateNewest => "Sort by Date",
                        Sort::DateOldest => "Sort by Date, oldest",
                        Sort::Name => "Sort by Name",
                    }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sort, Sort::DateNewest, tr("Sort by Date"));
                        ui.selectable_value(&mut sort, Sort::DateOldest, tr("Sort by Date, oldest"));
                        ui.selectable_value(&mut sort, Sort::Name, tr("Sort by Name"));
                    });
                if sort != self.sort {
                    self.sort = sort;
                    self.reload();
                }
                if ui
                    .button(egui::RichText::new("★").color(STAR))
                    .on_hover_text(tr("Select starred photos"))
                    .clicked()
                {
                    self.select_starred();
                }
                if !self.selection.is_empty() {
                    if ui.button(tr(if self.primary_starred() { "Unstar" } else { "Star" })).clicked() {
                        self.toggle_star_selection();
                    }
                    if ui.button(tr("Rotate Right")).on_hover_text(tr("Rotate clockwise (Ctrl+])")).clicked() {
                        self.rotate_current(90);
                    }
                    if ui.button(tr("Rotate Left")).on_hover_text(tr("Rotate counterclockwise (Ctrl+[)")).clicked() {
                        self.rotate_current(-90);
                    }
                    self.album_menu(ui);
                    if matches!(self.nav, Nav::Album(_)) && ui.button(tr("Remove from Album")).clicked() {
                        self.remove_selection_from_album();
                    }
                }
            });
        });
        ui.add_space(6.0);
    }

    fn album_menu(&mut self, ui: &mut egui::Ui) {
        let albums: Vec<(i64, String)> = self.albums.iter().map(|a| (a.id, a.name.clone())).collect();
        let mut create = false;
        let mut add: Option<i64> = None;
        egui::ComboBox::from_id_salt("add-album")
            .selected_text(tr("Add to Album"))
            .show_ui(ui, |ui| {
                for (id, name) in &albums {
                    if ui.selectable_label(false, name).clicked() {
                        add = Some(*id);
                    }
                }
                if ui.button(tr("New album…")).clicked() {
                    create = true;
                }
            });
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

    fn photo_cell(&mut self, ui: &mut egui::Ui, index: usize, rect: Rect) {
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
        let stroke = if selected {
            egui::Stroke::new(3.0, ACCENT)
        } else {
            egui::Stroke::new(1.0, Color32::from_black_alpha(18))
        };
        painter.rect_stroke(image_rect, THUMB_RADIUS, stroke, egui::StrokeKind::Inside);
        if selected {
            let center = image_rect.left_top() + Vec2::splat(15.0);
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
        painter.text(
            Pos2::new(image_rect.center().x, image_rect.bottom() + 5.0),
            Align2::CENTER_TOP,
            elide(&photo.filename, ((thumb / 7.0) as usize).max(8)),
            FontId::proportional(11.5),
            theme::muted(),
        );
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
        } else {
            response
        };
        if response.clicked() {
            let modifiers = ui.input(|input| input.modifiers);
            if on_star {
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
            if ui.button(tr("Show in folder")).clicked() {
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    let _ = open::that(parent);
                }
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

fn build_layout(photos: &[Photo], sort: Sort, width: f32, thumb: f32) -> GridLayout {
    let cols = ((width - PAD * 2.0).max(thumb) / (thumb + GAP)).floor().max(1.0) as usize;
    let row_h = thumb + LABEL_H + GAP;
    let mut rows = Vec::new();
    let mut y = 8.0;
    if matches!(sort, Sort::Name) {
        let mut offset = 0;
        for chunk in photos.chunks(cols) {
            let indices: Vec<usize> = (offset..offset + chunk.len()).collect();
            offset += chunk.len();
            rows.push(GridRow::Photos { indices, y, h: row_h });
            y += row_h;
        }
    } else {
        let mut index = 0;
        while index < photos.len() {
            let title = format_day(photos[index].taken_at);
            let start = index;
            index += 1;
            while index < photos.len() && format_day(photos[index].taken_at) == title {
                index += 1;
            }
            rows.push(GridRow::Header { title, y });
            y += HEADER_H;
            let mut cursor = start;
            while cursor < index {
                let end = (cursor + cols).min(index);
                rows.push(GridRow::Photos {
                    indices: (cursor..end).collect(),
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
        // Two columns of 100 px thumbnails; rows are 136 px and headers 40 px.
        let dated = build_layout(&photos, Sort::DateNewest, 232.0, 100.0);
        assert_eq!(dated.cols, 2);
        let (title_a, title_b) = (format_day(day_a), format_day(day_b));
        assert_eq!(dated.position_label(&photos, Sort::DateNewest, 0.0), Some(title_a.clone()));
        assert_eq!(dated.position_label(&photos, Sort::DateNewest, 200.0), Some(title_a));
        assert_eq!(dated.position_label(&photos, Sort::DateNewest, 330.0), Some(title_b.clone()));
        assert_eq!(dated.position_label(&photos, Sort::DateNewest, 10_000.0), Some(title_b));

        let named = build_layout(&photos, Sort::Name, 232.0, 100.0);
        assert_eq!(named.position_label(&photos, Sort::Name, 0.0).as_deref(), Some("a"));
        assert_eq!(named.position_label(&photos, Sort::Name, 150.0).as_deref(), Some("c"));
        assert_eq!(named.position_label(&photos, Sort::Name, 10_000.0), None);
    }
}
