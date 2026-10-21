//! Folder tree in the sidebar: each added folder with its subfolders nested
//! below it, the way Picasa 3 shows folders in tree view.

use std::path::MAIN_SEPARATOR;

use eframe::egui::{self, Color32, Rect, Shape, Stroke, Vec2};

use crate::types::{folder_label, Folder, Nav};

use super::theme::{self, MUTED, TEXT};
use super::PicassoApp;

const INDENT: f32 = 14.0;

pub(super) struct DirNode {
    path: String,
    name: String,
    /// Photos in this folder and every folder below it.
    count: i64,
    children: Vec<DirNode>,
}

pub(super) struct FolderRoot {
    id: i64,
    path: String,
    label: String,
    count: i64,
    children: Vec<DirNode>,
}

/// Builds one tree per added folder from per-directory photo counts.
pub(super) fn build(folders: &[Folder], dirs: &[(String, i64)]) -> Vec<FolderRoot> {
    folders
        .iter()
        .map(|folder| {
            let root = folder.path.trim_end_matches(MAIN_SEPARATOR);
            let mut children: Vec<DirNode> = Vec::new();
            for (dir, count) in dirs {
                let Some(rel) = dir.strip_prefix(root).and_then(|rest| rest.strip_prefix(MAIN_SEPARATOR)) else {
                    continue;
                };
                let mut level = &mut children;
                let mut path = root.to_string();
                for name in rel.split(MAIN_SEPARATOR).filter(|name| !name.is_empty()) {
                    path.push(MAIN_SEPARATOR);
                    path.push_str(name);
                    let index = match level.iter().position(|node| node.name == name) {
                        Some(index) => index,
                        None => {
                            level.push(DirNode {
                                path: path.clone(),
                                name: name.to_string(),
                                count: 0,
                                children: Vec::new(),
                            });
                            level.len() - 1
                        }
                    };
                    let node = &mut level[index];
                    node.count += count;
                    level = &mut node.children;
                }
            }
            sort_nodes(&mut children);
            FolderRoot {
                id: folder.id,
                path: root.to_string(),
                label: folder_label(&folder.path),
                count: folder.count,
                children,
            }
        })
        .collect()
}

fn sort_nodes(nodes: &mut [DirNode]) {
    nodes.sort_by_key(|node| node.name.to_lowercase());
    for node in nodes {
        sort_nodes(&mut node.children);
    }
}

/// One visible line of the tree.
struct Row {
    nav: Nav,
    path: String,
    label: String,
    count: i64,
    depth: usize,
    open: Option<bool>,
}

impl PicassoApp {
    fn tree_is_open(&self, path: &str, depth: usize) -> bool {
        self.tree_open.get(path).copied().unwrap_or(depth == 0)
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for root in &self.folder_tree {
            let open = self.tree_is_open(&root.path, 0);
            rows.push(Row {
                nav: Nav::Folder(root.id),
                path: root.path.clone(),
                label: root.label.clone(),
                count: root.count,
                depth: 0,
                open: (!root.children.is_empty()).then_some(open),
            });
            if open {
                self.push_rows(&root.children, 1, &mut rows);
            }
        }
        rows
    }

    fn push_rows(&self, nodes: &[DirNode], depth: usize, rows: &mut Vec<Row>) {
        for node in nodes {
            let open = self.tree_is_open(&node.path, depth);
            rows.push(Row {
                nav: Nav::Dir(node.path.clone()),
                path: node.path.clone(),
                label: node.name.clone(),
                count: node.count,
                depth,
                open: (!node.children.is_empty()).then_some(open),
            });
            if open {
                self.push_rows(&node.children, depth + 1, rows);
            }
        }
    }

    pub(super) fn folder_tree_ui(&mut self, ui: &mut egui::Ui) {
        for row in self.visible_rows() {
            let selected = self.nav == row.nav;
            let (response, toggled) = tree_line(ui, &row, selected);
            if toggled {
                let open = row.open.unwrap_or(false);
                self.tree_open.insert(row.path.clone(), !open);
            } else if response.clicked() {
                if row.open == Some(false) {
                    self.tree_open.insert(row.path.clone(), true);
                }
                self.set_nav(row.nav.clone());
            }
            let path = row.path.clone();
            response.context_menu(|ui| {
                let reveal = if cfg!(target_os = "macos") { "Show in Finder" } else { "Show in Explorer" };
                if ui.button(crate::i18n::tr(reveal)).clicked() {
                    let _ = open::that(&path);
                    ui.close();
                }
                if let Nav::Folder(id) = row.nav {
                    if ui.button(crate::i18n::tr("Refresh Thumbnails")).clicked() {
                        let _ = self.cmd_tx.send(crate::worker::Command::Scan(id));
                        ui.close();
                    }
                    if ui.button(crate::i18n::tr("Remove from Picasso…")).clicked() {
                        self.db.remove_folder(id);
                        if matches!(self.nav, Nav::Folder(_) | Nav::Dir(_)) {
                            self.nav = Nav::Library;
                        }
                        self.reload();
                        self.sync_watches();
                        ui.close();
                    }
                }
            });
        }
    }
}

/// Draws one tree line. Returns the response and whether the disclosure
/// arrow was clicked.
fn tree_line(ui: &mut egui::Ui, row: &Row, selected: bool) -> (egui::Response, bool) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.0), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, theme::RADIUS, theme::SELECT);
    } else if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS, theme::HOVER);
    }
    let indent = row.depth as f32 * INDENT;
    let arrow_x = rect.left() + 6.0 + indent;
    let cy = rect.center().y;
    if let Some(open) = row.open {
        let points = if open {
            vec![egui::pos2(arrow_x - 1.0, cy - 2.5), egui::pos2(arrow_x + 7.0, cy - 2.5), egui::pos2(arrow_x + 3.0, cy + 2.5)]
        } else {
            vec![egui::pos2(arrow_x + 1.0, cy - 4.0), egui::pos2(arrow_x + 6.0, cy), egui::pos2(arrow_x + 1.0, cy + 4.0)]
        };
        painter.add(Shape::convex_polygon(points, MUTED, Stroke::NONE));
    }
    let icon = egui::pos2(arrow_x + 14.0, cy - 7.0);
    painter.rect_filled(Rect::from_min_size(icon + Vec2::new(0.0, 1.0), Vec2::new(7.0, 4.0)), 2, Color32::from_rgb(0xD9, 0x9A, 0x1E));
    painter.rect_filled(Rect::from_min_size(icon + Vec2::new(0.0, 3.0), Vec2::new(14.0, 10.0)), 2, theme::FOLDER);

    let count_rect = painter.text(
        egui::pos2(rect.right() - 10.0, cy),
        egui::Align2::RIGHT_CENTER,
        row.count.to_string(),
        egui::FontId::proportional(12.0),
        MUTED,
    );
    let clip = Rect::from_min_max(rect.min, egui::pos2(count_rect.left() - 8.0, rect.max.y));
    painter.with_clip_rect(clip).text(
        egui::pos2(icon.x + 21.0, cy),
        egui::Align2::LEFT_CENTER,
        &row.label,
        egui::FontId::proportional(13.5),
        if selected { theme::ACCENT } else { TEXT },
    );

    let toggled = row.open.is_some()
        && response.clicked()
        && response
            .interact_pointer_pos()
            .is_some_and(|pos| pos.x < arrow_x + 12.0);
    (response, toggled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nests_subfolders_with_recursive_counts() {
        let join = |parts: &[&str]| parts.join(&MAIN_SEPARATOR.to_string());
        let root = join(&["", "photos"]);
        let folders = [Folder {
            id: 1,
            path: root.clone(),
            count: 10,
        }];
        let dirs = [
            (root.clone(), 2),
            (join(&[&root, "b"]), 4),
            (join(&[&root, "a"]), 1),
            (join(&[&root, "a", "x"]), 3),
            (join(&["", "photos2"]), 5),
        ];
        let tree = build(&folders, &dirs);
        assert_eq!(tree.len(), 1);
        let names: Vec<(&str, i64)> = tree[0].children.iter().map(|node| (node.name.as_str(), node.count)).collect();
        assert_eq!(names, [("a", 4), ("b", 4)]);
        assert_eq!(tree[0].children[0].children[0].path, join(&[&root, "a", "x"]));
    }
}
