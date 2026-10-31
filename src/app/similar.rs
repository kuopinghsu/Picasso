//! Similar photos and bursts in a Trip, suggested as groups to choose from
//! (`crate::burst`). Looking at the photos runs on its own thread, can be
//! cancelled, and keeps each look with the thumbnail, so a second search
//! only reads what is new. A group can be selected in the grid; nothing is
//! deleted, moved, or starred.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eframe::egui::{self, Align, Layout, Margin, RichText, Vec2};

use crate::burst::{self, Shot};
use crate::i18n::{tr, trf};
use crate::trip::MemberQuery;
use crate::types::{is_video_path, Photo, Sort};
use crate::worker::UiEvent;

use super::{about, theme, Modal, PicassoApp};

/// Photos sent back between progress reports.
const REPORT_EVERY: usize = 25;
/// Photos of a group shown in its row.
const SHOWN: usize = 8;

/// A search for similar photos in one Trip, running or done.
pub(super) struct SimilarScan {
    run: u64,
    trip_id: i64,
    cancel: Arc<AtomicBool>,
    done: usize,
    total: usize,
    /// The Trip's photos and how they look, once every one has been seen.
    found: Option<(Vec<Shot>, HashMap<i64, Photo>)>,
    window: i64,
    distance: u32,
}

impl PicassoApp {
    /// Opens the search for the Trip's similar photos, starting it unless
    /// one for this Trip is open already.
    pub(super) fn show_similar(&mut self, ctx: &egui::Context, trip_id: i64) {
        if self.similar.as_ref().is_none_or(|scan| scan.trip_id != trip_id) {
            self.search_similar(ctx, trip_id);
        }
        self.modal = Modal::Similar;
    }

    fn search_similar(&mut self, ctx: &egui::Context, trip_id: i64) {
        if let Some(scan) = self.similar.take() {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        let run = self.similar_run + 1;
        self.similar_run = run;
        let cancel = Arc::new(AtomicBool::new(false));
        self.similar = Some(SimilarScan {
            run,
            trip_id,
            cancel: cancel.clone(),
            done: 0,
            total: 0,
            found: None,
            window: burst::DEFAULT_WINDOW,
            distance: burst::DEFAULT_DISTANCE,
        });
        let db_path = self.data_dir.join("picasso.db");
        let cache = self.cache_dir.clone();
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-similar".into()).spawn(move || {
            let Ok(db) = crate::db::Database::open(&db_path) else { return };
            let store = crate::thumbs::ThumbStore::open(&cache);
            let photos: Vec<Photo> = db
                .trip_photos(trip_id, &MemberQuery::default(), Sort::DateOldest)
                .into_iter()
                .filter(|photo| !is_video_path(Path::new(&photo.path)))
                .collect();
            let total = photos.len();
            let mut shots = Vec::with_capacity(total);
            for (done, photo) in photos.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if done % REPORT_EVERY == 0 {
                    let _ = events.send(UiEvent::Similar { run, done, total, found: None });
                    ctx.request_repaint();
                }
                let look = store.look(photo.id).or_else(|| {
                    let thumb = crate::worker::plain_thumb(&store, photo.id, Path::new(&photo.path))?;
                    let look = burst::look(&thumb);
                    store.put_look(photo.id, look);
                    Some(look)
                });
                if let Some(look) = look {
                    shots.push(Shot { id: photo.id, taken_at: photo.taken_at, look });
                }
            }
            let photos = photos.into_iter().map(|photo| (photo.id, photo)).collect();
            let _ = events.send(UiEvent::Similar { run, done: total, total, found: Some((shots, photos)) });
            ctx.request_repaint();
        });
    }

    pub(super) fn on_similar(&mut self, run: u64, done: usize, total: usize, found: Option<(Vec<Shot>, HashMap<i64, Photo>)>) {
        let Some(scan) = self.similar.as_mut().filter(|scan| scan.run == run) else { return };
        (scan.done, scan.total) = (done, total);
        if found.is_some() {
            scan.found = found;
        }
    }

    pub(super) fn similar_modal(&mut self, ctx: &egui::Context) {
        let Some(mut scan) = self.similar.take() else {
            self.modal = Modal::None;
            return;
        };
        let mut close = false;
        let mut stop = false;
        let mut pick: Option<Vec<i64>> = None;
        let response = egui::Modal::new(egui::Id::unique("similar-photos")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(620.0);
            about::title_bar(ui, &self.logo, tr("Similar photos"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.label(
                    RichText::new(tr("Photos taken close together that look alike, such as a burst. Select a group to compare its photos in the grid; nothing is deleted, moved, or starred."))
                        .size(12.5)
                        .color(theme::muted()),
                );
                let Some((shots, photos)) = &scan.found else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(trf("Looking at {} of {} photos…", &[&scan.done, &scan.total]));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button(tr("Cancel")).clicked() {
                                stop = true;
                            }
                        });
                    });
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(tr("Taken within"));
                    ui.add(egui::DragValue::new(&mut scan.window).range(1..=600).suffix(" s"))
                        .on_hover_text(tr("Longest time between one photo of a group and the next"));
                    ui.add_space(16.0);
                    ui.label(tr("Alike"));
                    // Fewer differing bits is stricter; the slider reads from strict to loose.
                    ui.add(egui::Slider::new(&mut scan.distance, 2..=24).show_value(false).text(tr("loose")))
                        .on_hover_text(tr("How alike photos of a group must look: left only near copies, right more of a scene"));
                });
                let groups = burst::group(shots, scan.window, scan.distance);
                let grouped: usize = groups.iter().map(Vec::len).sum();
                let summary = if groups.is_empty() {
                    tr("No similar photos with these settings.").to_string()
                } else {
                    trf("{} groups with {} photos", &[&groups.len(), &grouped])
                };
                ui.label(RichText::new(summary).strong());
                egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                    for group in &groups {
                        if let Some(ids) = self.similar_row(ui, group, photos) {
                            pick = Some(ids);
                        }
                    }
                });
            });
        });
        if stop {
            scan.cancel.store(true, Ordering::Relaxed);
            self.modal = Modal::None;
            return;
        }
        if let Some(ids) = pick {
            self.selection = ids.into_iter().collect::<HashSet<i64>>();
            self.modal = Modal::None;
        } else if close || response.should_close() {
            self.modal = Modal::None;
        }
        self.similar = Some(scan);
    }

    /// One group: its first photos, how many and when, and Select.
    fn similar_row(&mut self, ui: &mut egui::Ui, group: &[i64], photos: &HashMap<i64, Photo>) -> Option<Vec<i64>> {
        let mut pick = None;
        let size = Vec2::new(ui.available_width(), 56.0);
        ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(size);
            if ui.button(tr("Select")).on_hover_text(tr("Select these photos in the grid")).clicked() {
                pick = Some(group.to_vec());
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                for id in group.iter().take(SHOWN) {
                    let Some(photo) = photos.get(id) else { continue };
                    self.ensure_thumb(photo.id, &photo.path);
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(54.0, 40.0), egui::Sense::hover());
                    match self.thumb_texture(photo.id) {
                        Some((texture, w, h)) => theme::paint_image_rounded(ui.painter(), &texture, rect, theme::cover_uv(w, h), photo.rotation, 4.0),
                        None => {
                            ui.painter().rect_filled(rect, 4, theme::placeholder());
                        }
                    }
                    response.on_hover_text(&photo.filename);
                }
                if group.len() > SHOWN {
                    ui.label(RichText::new(format!("+{}", group.len() - SHOWN)).color(theme::muted()));
                }
                ui.add_space(6.0);
                let first = group.first().and_then(|id| photos.get(id)).map_or(0, |photo| photo.taken_at);
                let when = crate::types::format_timestamp(first);
                ui.label(RichText::new(format!("{} · {when}", trf("{} photos", &[&group.len()]))).size(12.5).color(theme::muted()));
            });
        });
        ui.separator();
        pick
    }
}
