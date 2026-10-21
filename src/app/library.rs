//! Library grid and the browse toolbar.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::types::{format_day, Nav, Photo, Sort};

use super::theme::{self, ACCENT, HAIRLINE, MUTED, STAR, TEXT, WHITE};
use super::PicassoApp;

const GAP: f32 = 16.0;
const LABEL_H: f32 = 16.0;
const HEADER_H: f32 = 26.0;
const PAD: f32 = 10.0;

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
                        let pos = origin + Vec2::new(PAD, *y + 4.0);
                        let galley = ui.painter().layout_no_wrap(title.clone(), FontId::proportional(13.0), TEXT);
                        let text_w = galley.size().x;
                        ui.painter().galley(pos, galley, TEXT);
                        let line_y = pos.y + 8.0;
                        ui.painter().line_segment(
                            [
                                Pos2::new(pos.x + text_w + 10.0, line_y),
                                Pos2::new(origin.x + width - PAD, line_y),
                            ],
                            egui::Stroke::new(1.0, HAIRLINE),
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
            ui.label(egui::RichText::new(self.view_title()).size(16.0).strong().color(TEXT));
            let count = self.photos.len();
            let noun = if count == 1 { "photo" } else { "photos" };
            ui.label(egui::RichText::new(format!("{count} {noun}")).size(12.0).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut sort = self.sort;
                egui::ComboBox::from_id_salt("sort")
                    .selected_text(match sort {
                        Sort::DateNewest => "Sort by Date",
                        Sort::DateOldest => "Sort by Date, oldest",
                        Sort::Name => "Sort by Name",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sort, Sort::DateNewest, "Sort by Date");
                        ui.selectable_value(&mut sort, Sort::DateOldest, "Sort by Date, oldest");
                        ui.selectable_value(&mut sort, Sort::Name, "Sort by Name");
                    });
                if sort != self.sort {
                    self.sort = sort;
                    self.reload();
                }
                if ui.button("★").on_hover_text("Select starred photos").clicked() {
                    self.select_starred();
                }
                if !self.selection.is_empty() {
                    if ui.button(if self.primary_starred() { "Unstar" } else { "Star" }).clicked() {
                        self.toggle_star_selection();
                    }
                    if ui.button("Rotate Right").on_hover_text("Rotate clockwise (Ctrl+])").clicked() {
                        self.rotate_current(90);
                    }
                    if ui.button("Rotate Left").on_hover_text("Rotate counterclockwise (Ctrl+[)").clicked() {
                        self.rotate_current(-90);
                    }
                    self.album_menu(ui);
                    if matches!(self.nav, Nav::Album(_)) && ui.button("Remove from Album").clicked() {
                        self.remove_selection_from_album();
                    }
                }
            });
        });
        ui.separator();
    }

    fn album_menu(&mut self, ui: &mut egui::Ui) {
        let albums: Vec<(i64, String)> = self.albums.iter().map(|a| (a.id, a.name.clone())).collect();
        let mut create = false;
        let mut add: Option<i64> = None;
        egui::ComboBox::from_id_salt("add-album")
            .selected_text("Add to Album")
            .show_ui(ui, |ui| {
                for (id, name) in &albums {
                    if ui.selectable_label(false, name).clicked() {
                        add = Some(*id);
                    }
                }
                if ui.button("New album…").clicked() {
                    create = true;
                }
            });
        if let Some(id) = add {
            let photos: Vec<i64> = self.selection.iter().copied().collect();
            self.db.add_to_album(id, &photos);
            self.albums = self.db.albums();
            self.activity = format!("Added {} to album", photos.len());
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
        let response = ui.interact(rect, egui::Id::unique(("photo", photo.id)), Sense::click());
        ui.painter().rect_filled(image_rect, 0, WHITE);
        if let Some((texture, w, h)) = self.thumb_texture(photo.id) {
            theme::paint_image(ui.painter(), &texture, image_rect, theme::cover_uv(w, h), photo.rotation);
        } else {
            ui.painter().rect_filled(image_rect, 0, Color32::from_rgb(0xF0, 0xF0, 0xF0));
        }
        let stroke = if selected {
            egui::Stroke::new(3.0, ACCENT)
        } else if response.hovered() {
            egui::Stroke::new(1.0, Color32::from_rgb(0x8A, 0xB8, 0xE0))
        } else {
            egui::Stroke::new(1.0, HAIRLINE)
        };
        ui.painter().rect_stroke(image_rect, 0, stroke, egui::StrokeKind::Inside);
        if photo.starred {
            ui.painter().text(
                image_rect.right_bottom() + Vec2::new(-4.0, -2.0),
                Align2::RIGHT_BOTTOM,
                "★",
                FontId::proportional(16.0),
                STAR,
            );
        }
        ui.painter().text(
            Pos2::new(image_rect.center().x, image_rect.bottom() + 2.0),
            Align2::CENTER_TOP,
            elide(&photo.filename, ((thumb / 7.0) as usize).max(8)),
            FontId::proportional(11.0),
            MUTED,
        );
        let star_rect = Rect::from_min_size(image_rect.right_bottom() + Vec2::new(-22.0, -22.0), Vec2::splat(22.0));
        let on_star = response
            .interact_pointer_pos()
            .map(|pos| star_rect.contains(pos) && photo.starred)
            .unwrap_or(false);
        if response.clicked() {
            let modifiers = ui.input(|input| input.modifiers);
            if on_star {
                self.toggle_star(photo.id);
            } else if response.double_clicked() {
                self.open_at(index);
            } else {
                self.select_click(index, modifiers);
            }
        }
        let path = photo.path.clone();
        let id = photo.id;
        response.context_menu(|ui| {
            if ui.button("Open").clicked() {
                self.open_at(index);
                ui.close();
            }
            if ui.button(if photo.starred { "Unstar" } else { "Star" }).clicked() {
                self.toggle_star(id);
                ui.close();
            }
            if ui.button("Rotate Clockwise").clicked() {
                self.rotate_from_menu(id, 90);
                ui.close();
            }
            if ui.button("Rotate Counterclockwise").clicked() {
                self.rotate_from_menu(id, -90);
                ui.close();
            }
            if ui.button("Show in folder").clicked() {
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    let _ = open::that(parent);
                }
                ui.close();
            }
            if ui.button("Show on map").clicked() {
                self.show_on_map(id);
                ui.close();
            }
            if ui.button("Move to trash").clicked() {
                self.modal = super::Modal::Trash { ids: vec![id] };
                ui.close();
            }
        });
    }

    fn empty_library(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(88.0);
            let title = if self.search.trim().is_empty() {
                "No photos found"
            } else {
                "No results found in this album"
            };
            ui.label(egui::RichText::new(title).size(18.0).strong().color(TEXT));
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Import a folder and Picasso scans it for pictures.").color(MUTED));
            ui.add_space(12.0);
            if ui.button("Import").clicked() {
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
