//! Finding files whose content is identical, to move the extra copies to
//! the trash.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use eframe::egui::{self, Align, Layout, Margin, RichText, Stroke, Vec2};
use ring::digest;

use crate::db::Database;
use crate::i18n::{tr, trf};
use crate::shell;
use crate::types::{format_bytes, Photo};
use crate::worker::UiEvent;

use super::{about, theme, Modal, PicassoApp};

const ROW_HEIGHT: f32 = 56.0;
/// Bytes compared first; files that differ there are never read in full.
const HEAD: u64 = 64 * 1024;
/// File in the cache folder that remembers the hashes of earlier searches.
pub(super) const HASHES_FILE: &str = "hashes.db";

/// A duplicate search running on a background thread, or its result.
pub(super) struct DupeScan {
    run: u64,
    cancel: Arc<AtomicBool>,
    checked: usize,
    total: usize,
    /// Each group holds copies of one file; the first is the one kept.
    groups: Option<Vec<Vec<Photo>>>,
}

enum Row {
    Group(usize),
    Copy(usize, usize),
}

enum Action {
    Reveal(String),
    Trash(Vec<i64>),
}

impl PicassoApp {
    /// Shows the search running in the background or its result, or else
    /// starts one.
    pub(super) fn show_duplicates(&mut self, ctx: &egui::Context) {
        if self.dupes.is_some() {
            self.modal = Modal::Duplicates { confirm_all: false };
        } else {
            self.search_duplicates(ctx);
        }
    }

    /// Groups found by a finished search, for the sidebar.
    pub(super) fn duplicate_groups(&self) -> Option<usize> {
        self.dupes.as_ref()?.groups.as_ref().map(Vec::len)
    }

    fn search_duplicates(&mut self, ctx: &egui::Context) {
        self.stop_duplicates();
        let run = self.dupes_run + 1;
        self.dupes_run = run;
        let cancel = Arc::new(AtomicBool::new(false));
        self.dupes = Some(DupeScan {
            run,
            cancel: cancel.clone(),
            checked: 0,
            total: 0,
            groups: None,
        });
        self.modal = Modal::Duplicates { confirm_all: false };
        let db_path = self.data_dir.join("picasso.db");
        let hashes_path = self.cache_dir.join(HASHES_FILE);
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-duplicates".into()).spawn(move || {
            let hashes = HashStore::open(&hashes_path);
            let groups = find(&db_path, &hashes, run, &cancel, &events, &ctx);
            if let Some(groups) = groups.filter(|_| !cancel.load(Ordering::Relaxed)) {
                let total = groups.iter().map(Vec::len).sum();
                let _ = events.send(UiEvent::Duplicates {
                    run,
                    checked: total,
                    total,
                    groups: Some(groups),
                });
                ctx.request_repaint();
            }
        });
    }

    fn stop_duplicates(&mut self) {
        if let Some(scan) = self.dupes.take() {
            scan.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn on_duplicates(&mut self, run: u64, checked: usize, total: usize, groups: Option<Vec<Vec<Photo>>>) {
        let Some(scan) = self.dupes.as_mut().filter(|scan| scan.run == run) else { return };
        scan.checked = checked;
        scan.total = total;
        if let Some(mut groups) = groups {
            for group in &mut groups {
                // Kept first: a starred or edited copy, else the first by path.
                group.sort_by_key(|photo| (!photo.starred, !self.edits.contains_key(&photo.id), photo.path.clone()));
            }
            groups.sort_by(|a, b| a[0].path.cmp(&b[0].path));
            scan.groups = Some(groups);
        }
    }

    pub(super) fn duplicates_modal(&mut self, ctx: &egui::Context) {
        let Modal::Duplicates { confirm_all } = self.modal else { return };
        let Some(scan) = self.dupes.take() else {
            self.modal = Modal::None;
            return;
        };
        let mut confirm = confirm_all;
        let mut close = false;
        let mut stop = false;
        let mut again = false;
        let mut action = None;
        let response = egui::Modal::new(egui::Id::unique("duplicates")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(760.0);
            about::title_bar(ui, &self.logo, tr("Duplicate files"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                let Some(groups) = &scan.groups else {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        let note = if scan.total == 0 {
                            tr("Looking for files of the same size…").to_string()
                        } else {
                            trf("Comparing {} of {} files…", &[&scan.checked, &scan.total])
                        };
                        ui.label(RichText::new(note).color(theme::muted()));
                    });
                    ui.add_space(4.0);
                    let hint = tr("Every copy is read in full to be sure, which can take a while. The search goes on when you close this window.");
                    ui.label(RichText::new(hint).size(12.0).color(theme::muted()));
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(tr("Close")).clicked() {
                            close = true;
                        }
                        if ui.button(tr("Stop")).clicked() {
                            stop = true;
                        }
                    });
                    return;
                };
                if groups.is_empty() {
                    ui.label(RichText::new(tr("No duplicate files found.")).color(theme::muted()));
                } else {
                    let extra: Vec<i64> = groups.iter().flat_map(|group| group[1..].iter().map(|photo| photo.id)).collect();
                    let freed: u64 = groups.iter().map(|group| group[0].bytes * (group.len() as u64 - 1)).sum();
                    let intro = trf(
                        "{} files have identical copies. Moving the extra copies to the trash frees {}.",
                        &[&groups.len(), &format_bytes(freed)],
                    );
                    ui.label(RichText::new(intro).color(theme::muted()));
                    ui.label(RichText::new(tr("The first file of each group is kept: a starred or edited one when there is one.")).color(theme::muted()));
                    ui.add_space(8.0);
                    let rows: Vec<Row> = groups
                        .iter()
                        .enumerate()
                        .flat_map(|(g, group)| std::iter::once(Row::Group(g)).chain((0..group.len()).map(move |c| Row::Copy(g, c))))
                        .collect();
                    egui::ScrollArea::vertical()
                        .max_height(440.0)
                        .auto_shrink([false, true])
                        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for row in &rows[range] {
                                let chosen = match *row {
                                    Row::Group(g) => group_row(ui, &groups[g]),
                                    Row::Copy(g, c) => self.copy_row(ui, &groups[g][c], c == 0),
                                };
                                if chosen.is_some() {
                                    action = chosen;
                                }
                            }
                        });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if confirm {
                            ui.label(trf("Move {} files to the system trash?", &[&extra.len()]));
                            if ui.add(theme::danger_button(tr("Move to trash"))).clicked() {
                                action = Some(Action::Trash(extra.clone()));
                                confirm = false;
                            }
                            if ui.button(tr("Cancel")).clicked() {
                                confirm = false;
                            }
                            return;
                        }
                        if ui.add(theme::danger_button(tr("Move all extra copies to trash"))).clicked() {
                            if self.confirm_trash {
                                confirm = true;
                            } else {
                                action = Some(Action::Trash(extra.clone()));
                            }
                        }
                    });
                }
                ui.add_space(4.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(tr("Close")).clicked() {
                        close = true;
                    }
                    if ui.button(tr("Search again")).clicked() {
                        again = true;
                    }
                });
            });
        });
        self.dupes = Some(scan);
        if again {
            self.search_duplicates(ctx);
            return;
        }
        if stop {
            self.stop_duplicates();
            self.modal = Modal::None;
            return;
        }
        match action {
            Some(Action::Reveal(path)) => shell::reveal(&path),
            Some(Action::Trash(ids)) => {
                let removed = self.trash_ids(&ids);
                if let Some(groups) = self.dupes.as_mut().and_then(|scan| scan.groups.as_mut()) {
                    for group in groups.iter_mut() {
                        group.retain(|photo| !removed.contains(&photo.id));
                    }
                    groups.retain(|group| group.len() > 1);
                }
            }
            None => {}
        }
        if close || response.should_close() {
            self.modal = Modal::None;
        } else {
            self.modal = Modal::Duplicates { confirm_all: confirm };
        }
    }

    /// One copy with its thumbnail, folder, and buttons.
    fn copy_row(&mut self, ui: &mut egui::Ui, photo: &Photo, kept: bool) -> Option<Action> {
        let plain = shell::plain_path(&photo.path);
        let parent = Path::new(&plain).parent().map(|path| path.display().to_string()).unwrap_or_default();
        let mut action = None;
        let size = Vec2::new(ui.available_width(), ROW_HEIGHT);
        let response = ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(size);
            if !kept && ui.button(tr("Move to trash")).clicked() {
                action = Some(Action::Trash(vec![photo.id]));
            }
            if ui.button(tr("Show in folder")).clicked() {
                action = Some(Action::Reveal(photo.path.clone()));
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(16.0);
                self.ensure_thumb(photo.id, &photo.path);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(60.0, 44.0), egui::Sense::hover());
                if let Some((texture, w, h)) = self.thumb_texture(photo.id) {
                    theme::paint_image_rounded(ui.painter(), &texture, rect, theme::cover_uv(w, h), photo.rotation, 4.0);
                } else {
                    ui.painter().rect_filled(rect, 4, theme::placeholder());
                }
                ui.add_space(6.0);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(9.0);
                    ui.horizontal(|ui| {
                        if kept {
                            ui.label(RichText::new(tr("Kept")).size(12.0).strong().color(theme::ACCENT));
                        }
                        let name = RichText::new(&photo.filename).size(13.5).color(theme::text());
                        ui.add(egui::Label::new(name).truncate()).on_hover_text(&plain);
                    });
                    let folder = RichText::new(&parent).size(12.0).color(theme::muted());
                    ui.add(egui::Label::new(folder).truncate()).on_hover_text(&parent);
                });
            });
        });
        let rect = response.response.rect;
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::hairline()));
        action
    }
}

/// The heading of a group: how many copies, their size, and a button that
/// trashes all but the kept one.
fn group_row(ui: &mut egui::Ui, group: &[Photo]) -> Option<Action> {
    let mut action = None;
    let size = Vec2::new(ui.available_width(), ROW_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Max), |ui| {
        ui.set_min_size(size);
        if ui.button(tr("Trash extra copies")).clicked() {
            action = Some(Action::Trash(group[1..].iter().map(|photo| photo.id).collect()));
        }
        ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
            let first = &group[0];
            let mut text = trf("{} copies · {}", &[&group.len(), &format_bytes(first.bytes)]);
            if first.width > 0 {
                text.push_str(&format!(" · {} × {}", first.width, first.height));
            }
            ui.label(RichText::new(text).strong().size(13.0).color(theme::text()));
        });
    });
    action
}

/// Groups of identical files among those the catalog lists with equal
/// sizes, or `None` when stopped.
fn find(db_path: &Path, hashes: &HashStore, run: u64, cancel: &AtomicBool, events: &Sender<UiEvent>, ctx: &egui::Context) -> Option<Vec<Vec<Photo>>> {
    let candidates = Database::open_reader(db_path).ok()?.duplicate_candidates();
    hashes.keep_only(candidates.iter().flatten().map(|photo| photo.path.as_str()));
    let mut progress = Progress {
        run,
        checked: 0,
        total: candidates.iter().map(Vec::len).sum(),
        sent: Instant::now(),
        events,
        ctx,
    };
    progress.send();
    let same_head = split(candidates, Part::Head, hashes, cancel, &mut progress)?;
    let (mut found, large): (Vec<_>, Vec<_>) = same_head.into_iter().partition(|group| group[0].bytes <= HEAD);
    progress.total += large.iter().map(Vec::len).sum::<usize>();
    found.extend(split(large, Part::Whole, hashes, cancel, &mut progress)?);
    Some(found)
}

/// How much of a file a hash covers.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    /// The first [`HEAD`] bytes.
    Head,
    Whole,
}

/// Hashes from earlier searches in `hashes.db` in the cache folder, by path.
/// A hash counts only while the file keeps the size and modification time
/// it had when it was read. Without the file, every file is read again.
struct HashStore {
    conn: Option<rusqlite::Connection>,
}

impl HashStore {
    fn open(path: &Path) -> HashStore {
        let conn = rusqlite::Connection::open(path).ok().filter(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
                 CREATE TABLE IF NOT EXISTS hashes (
                     path TEXT PRIMARY KEY,
                     bytes INTEGER NOT NULL,
                     modified_at INTEGER NOT NULL,
                     head BLOB NOT NULL,
                     whole BLOB
                 );",
            )
            .is_ok()
        });
        HashStore { conn }
    }

    fn get(&self, path: &str, stamp: (i64, i64), part: Part) -> Option<Vec<u8>> {
        let sql = match part {
            Part::Head => "SELECT head FROM hashes WHERE path=?1 AND bytes=?2 AND modified_at=?3",
            Part::Whole => "SELECT whole FROM hashes WHERE path=?1 AND bytes=?2 AND modified_at=?3 AND whole IS NOT NULL",
        };
        self.conn
            .as_ref()?
            .query_row(sql, rusqlite::params![path, stamp.0, stamp.1], |row| row.get(0))
            .ok()
    }

    fn put(&self, path: &str, stamp: (i64, i64), part: Part, hash: &[u8]) {
        let Some(conn) = &self.conn else { return };
        let sql = match part {
            // A new head drops the whole-file hash of an older version.
            Part::Head => {
                "INSERT INTO hashes(path, bytes, modified_at, head) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(path) DO UPDATE SET head=excluded.head,
                     whole=CASE WHEN bytes=excluded.bytes AND modified_at=excluded.modified_at THEN whole END,
                     bytes=excluded.bytes, modified_at=excluded.modified_at"
            }
            Part::Whole => "UPDATE hashes SET whole=?4 WHERE path=?1 AND bytes=?2 AND modified_at=?3",
        };
        let _ = conn.execute(sql, rusqlite::params![path, stamp.0, stamp.1, hash]);
    }

    /// Forgets files that are no longer candidates.
    fn keep_only<'a>(&self, paths: impl Iterator<Item = &'a str>) {
        let Some(conn) = &self.conn else { return };
        let keep: std::collections::HashSet<&str> = paths.collect();
        let stored: Vec<String> = conn
            .prepare("SELECT path FROM hashes")
            .and_then(|mut stmt| stmt.query_map([], |row| row.get(0))?.collect())
            .unwrap_or_default();
        let Ok(tx) = conn.unchecked_transaction() else { return };
        for path in stored.iter().filter(|path| !keep.contains(path.as_str())) {
            let _ = tx.execute("DELETE FROM hashes WHERE path=?1", [path]);
        }
        let _ = tx.commit();
    }
}

/// Size and modification time of the file on disk now.
fn stamp(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((meta.len() as i64, modified.as_nanos() as i64))
}

struct Progress<'a> {
    run: u64,
    checked: usize,
    total: usize,
    sent: Instant,
    events: &'a Sender<UiEvent>,
    ctx: &'a egui::Context,
}

impl Progress<'_> {
    fn step(&mut self) {
        self.checked += 1;
        if self.sent.elapsed() >= Duration::from_millis(100) {
            self.send();
        }
    }

    fn send(&mut self) {
        self.sent = Instant::now();
        let _ = self.events.send(UiEvent::Duplicates {
            run: self.run,
            checked: self.checked,
            total: self.total,
            groups: None,
        });
        self.ctx.request_repaint();
    }
}

/// Splits each group by content, the first `limit` bytes or all of it, and
/// returns the parts with more than one file; unreadable files drop out.
/// Files are read in path order, so a disk with heads mostly reads on
/// through a folder. `None` when stopped.
fn split(groups: Vec<Vec<Photo>>, part: Part, hashes: &HashStore, cancel: &AtomicBool, progress: &mut Progress) -> Option<Vec<Vec<Photo>>> {
    let mut files: Vec<(usize, Photo)> = groups
        .into_iter()
        .enumerate()
        .flat_map(|(group, photos)| photos.into_iter().map(move |photo| (group, photo)))
        .collect();
    files.sort_by(|a, b| a.1.path.cmp(&b.1.path));
    let mut by_hash: HashMap<(usize, Vec<u8>), Vec<Photo>> = HashMap::new();
    for (group, photo) in files {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let path = Path::new(&photo.path);
        let Some(stamp) = stamp(path) else {
            progress.step();
            continue;
        };
        let hash = hashes.get(&photo.path, stamp, part).or_else(|| {
            let limit = (part == Part::Head).then_some(HEAD);
            let hash = hash_file(path, limit)?;
            hashes.put(&photo.path, stamp, part, &hash);
            Some(hash)
        });
        if let Some(hash) = hash {
            by_hash.entry((group, hash)).or_default().push(photo);
        }
        progress.step();
    }
    Some(by_hash.into_values().filter(|group| group.len() > 1).collect())
}

fn hash_file(path: &Path, limit: Option<u64>) -> Option<Vec<u8>> {
    let file = std::fs::File::open(PathBuf::from(path)).ok()?;
    let mut reader: Box<dyn Read> = match limit {
        Some(limit) => Box::new(file.take(limit)),
        None => Box::new(file),
    };
    let mut context = digest::Context::new(&digest::SHA256);
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => context.update(&buffer[..read]),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(context.finish().as_ref().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(id: i64, path: &Path) -> Photo {
        Photo {
            id,
            folder_id: 1,
            path: path.display().to_string(),
            filename: path.file_name().unwrap().to_string_lossy().into_owned(),
            width: 0,
            height: 0,
            bytes: std::fs::metadata(path).unwrap().len(),
            taken_at: 0,
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
    fn only_files_with_the_same_content_are_grouped() {
        let dir = std::env::temp_dir().join(format!("picasso-dupes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut body = vec![7u8; (HEAD as usize) * 2];
        let write = |name: &str, body: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            path
        };
        let a = write("a.jpg", &body);
        let b = write("b.jpg", &body);
        // Same size and same first bytes, but a different end.
        *body.last_mut().unwrap() = 8;
        let c = write("c.jpg", &body);
        let other = write("d.jpg", &body[..10]);
        let (events, _rx) = crossbeam_channel::unbounded();
        let ctx = egui::Context::default();
        let mut progress = Progress {
            run: 1,
            checked: 0,
            total: 4,
            sent: Instant::now(),
            events: &events,
            ctx: &ctx,
        };
        let cancel = AtomicBool::new(false);
        let hashes = HashStore::open(&dir.join(HASHES_FILE));
        let search = |progress: &mut Progress| {
            let groups = vec![vec![photo(1, &a), photo(2, &b), photo(3, &c)], vec![photo(4, &other)]];
            let groups = split(groups, Part::Head, &hashes, &cancel, progress).unwrap();
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].len(), 3);
            let full = split(groups, Part::Whole, &hashes, &cancel, progress).unwrap();
            let mut ids: Vec<Vec<i64>> = full
                .iter()
                .map(|group| {
                    let mut ids: Vec<i64> = group.iter().map(|photo| photo.id).collect();
                    ids.sort();
                    ids
                })
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(search(&mut progress), [vec![1, 2]]);
        // The second search uses the stored hashes and finds the same.
        assert_eq!(search(&mut progress), [vec![1, 2]]);
        // A file written again is read again: c now matches a and b.
        std::thread::sleep(Duration::from_millis(20));
        *body.last_mut().unwrap() = 7;
        std::fs::write(&c, &body).unwrap();
        assert_eq!(search(&mut progress), [vec![1, 2, 3]]);
        drop(hashes);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
