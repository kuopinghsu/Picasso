//! Library grid and the browse toolbar.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::i18n::{tr, trf};
use crate::types::{format_day, Nav, Photo, Sort};

use super::theme::{self, ACCENT, MUTED, PLACEHOLDER, STAR, TEXT, THUMB_RADIUS, WHITE};
use super::PicassoApp;

const GAP: f32 = 14.0;
const LABEL_H: f32 = 22.0;
const HEADER_H: f32 = 40.0;
const PAD: f32 = 2.0;

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
        let epoch = self.view_epoch;
        let _ = epoch;
        area.show_viewport(ui, |ui, viewport| {
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
                            TEXT,
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
        });
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
            ui.label(egui::RichText::new(self.view_title()).size(22.0).strong().color(TEXT));
            ui.label(egui::RichText::new(super::photo_count(self.photos.len())).size(13.0).color(MUTED));
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
        painter.rect_filled(image_rect, THUMB_RADIUS, PLACEHOLDER);
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
        painter.text(
            Pos2::new(image_rect.center().x, image_rect.bottom() + 5.0),
            Align2::CENTER_TOP,
            elide(&photo.filename, ((thumb / 7.0) as usize).max(8)),
            FontId::proportional(11.5),
            MUTED,
        );
        let star_rect = Rect::from_min_size(image_rect.right_bottom() + Vec2::new(-22.0, -22.0), Vec2::splat(22.0));
        let on_star = response
            .interact_pointer_pos()
            .map(|pos| star_rect.contains(pos) && photo.starred)
            .unwrap_or(false);
        let over_place = |pos: Option<Pos2>| photo.place().is_some() && pos.is_some_and(|pos| place_rect.contains(pos));
        let response = if over_place(response.hover_pos()) {
            let text = if photo.pinned() { "Geotagged in Picasso — show on map" } else { "Has GPS location — show on map" };
            response.on_hover_text(tr(text))
        } else {
            response
        };
        if response.clicked() {
            let modifiers = ui.input(|input| input.modifiers);
            if on_star {
                self.toggle_star(photo.id);
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
            ui.label(egui::RichText::new(tr(title)).size(20.0).strong().color(TEXT));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tr("Import a folder and Picasso scans it for pictures.")).color(MUTED));
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
