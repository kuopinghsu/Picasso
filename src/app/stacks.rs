//! Stacks in the photo grid: photos taken a few seconds apart, or one after
//! another on the same day looking alike, shown as one tile with how many
//! photos it holds (`crate::burst::stacks`). A stack opens in place and
//! closes again from its number. Each kind can be turned on or off, and the
//! choice is kept. Nothing is changed in the library: a closed stack only
//! keeps its other photos out of the grid, so actions on its tile apply to
//! that photo alone.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Vec2};

use crate::burst::{self, StackRules, Tile};
use crate::i18n::{tr, trf};
use crate::types::{day_key, Nav, Photo};
use crate::worker::UiEvent;

use super::theme::{self, ACCENT, WHITE};
use super::PicassoApp;

/// Seconds apart photos can be to stack, unless changed.
pub(super) const DEFAULT_SECONDS: i64 = 3;
/// The most seconds apart the setting allows.
pub(super) const MAX_SECONDS: i64 = 60;
/// Looks sent back to the window at a time while they are being found.
const LOOKS_EVERY: usize = 200;

/// The grid's stacks and the settings that make them.
pub(super) struct Stacking {
    /// Stack photos taken at most `seconds` apart.
    pub by_time: bool,
    pub seconds: i64,
    /// Stack look-alikes taken one after another on the same day.
    pub alike: bool,
    /// How each photo looks, as far as it is known.
    looks: HashMap<i64, u64>,
    /// Photos whose look couldn't be found, so they aren't tried again.
    no_look: HashSet<i64>,
    /// The stacks shown open, by the photo on top.
    open: HashSet<i64>,
    /// The photo on top of each stack in view: how many photos the stack
    /// holds and whether it is open.
    tops: HashMap<i64, (usize, bool)>,
    /// The other photos of each closed stack, by the photo on top, in grid
    /// order.
    hidden: HashMap<i64, Vec<Photo>>,
    /// A stack to open or close before the next frame is drawn.
    toggle: Option<i64>,
    /// Looks being found in the background, and a way to stop it.
    finding: Option<Finding>,
    run: u64,
    /// The window, woken when looks arrive.
    ctx: egui::Context,
}

struct Finding {
    run: u64,
    cancel: Arc<AtomicBool>,
    done: usize,
    total: usize,
    /// The looks stored with the thumbnails have come back.
    stored: bool,
}

impl Stacking {
    /// The settings kept in the library, or stacks off when there are none.
    pub(super) fn load(db: &crate::db::Database, ctx: &egui::Context) -> Stacking {
        let on = |key: &str| db.setting(key).as_deref() == Some("1");
        let seconds = db
            .setting("stack_seconds")
            .and_then(|text| text.parse().ok())
            .map_or(DEFAULT_SECONDS, |seconds: i64| {
                seconds.clamp(1, MAX_SECONDS)
            });
        Stacking {
            by_time: on("stack_time"),
            seconds,
            alike: on("stack_alike"),
            looks: HashMap::new(),
            no_look: HashSet::new(),
            open: HashSet::new(),
            tops: HashMap::new(),
            hidden: HashMap::new(),
            toggle: None,
            finding: None,
            run: 0,
            ctx: ctx.clone(),
        }
    }

    fn save(&self, db: &crate::db::Database) {
        let flag = |on: bool| if on { "1" } else { "0" };
        db.set_setting("stack_time", flag(self.by_time));
        db.set_setting("stack_seconds", &self.seconds.to_string());
        db.set_setting("stack_alike", flag(self.alike));
    }

    /// How many photos are kept out of the grid in closed stacks.
    pub(super) fn hidden_count(&self) -> usize {
        self.hidden.values().map(Vec::len).sum()
    }
}

/// Whether `nav` shows photos in a grid that can hold stacks. The map, the
/// statistics, and the Trips page show every photo or none.
pub(super) fn shown_for(nav: &Nav) -> bool {
    matches!(
        nav,
        Nav::Library | Nav::Starred | Nav::Folder(_) | Nav::Dir(_) | Nav::Album(_) | Nav::Trip(_)
    )
}

/// What [`fold`] makes of the view.
type Folded = (
    Vec<Photo>,
    HashMap<i64, (usize, bool)>,
    HashMap<i64, Vec<Photo>>,
);

/// `photos` in grid order with the stacks `ranges` found in them: each
/// stack's photo on top, then its other photos when the stack is open or
/// `keep` is one of them, else kept aside under the photo on top. Returns
/// the photos to show, the stacks by the photo on top, and those kept aside.
fn fold(
    photos: Vec<Photo>,
    ranges: &[std::ops::Range<usize>],
    open: &HashSet<i64>,
    keep: Option<i64>,
) -> Folded {
    let mut shown = Vec::with_capacity(photos.len());
    let mut tops = HashMap::new();
    let mut hidden = HashMap::new();
    let mut ranges = ranges.iter().peekable();
    let mut stack: Option<(i64, usize, bool)> = None;
    for (index, photo) in photos.into_iter().enumerate() {
        if let Some((top, end, is_open)) = stack {
            if index < end {
                if is_open {
                    shown.push(photo);
                } else {
                    hidden.entry(top).or_insert_with(Vec::new).push(photo);
                }
                continue;
            }
            stack = None;
        }
        if let Some(range) = ranges.next_if(|range| range.start == index) {
            let top = photo.id;
            tops.insert(top, (range.len(), false));
            stack = Some((top, range.end, open.contains(&top)));
        }
        shown.push(photo);
    }
    // A stack holding the photo open in the viewer opens, so the viewer can
    // stay on it; its photos go back after the one on top.
    if let Some(keep) = keep {
        let holder = hidden
            .iter()
            .find(|(_, members)| members.iter().any(|photo: &Photo| photo.id == keep))
            .map(|(top, _)| *top);
        if let Some(top) = holder {
            let members = hidden.remove(&top).unwrap_or_default();
            if let Some(at) = shown.iter().position(|photo| photo.id == top) {
                shown.splice(at + 1..at + 1, members);
            }
        }
    }
    for (top, (_, is_open)) in tops.iter_mut() {
        *is_open = !hidden.contains_key(top);
    }
    (shown, tops, hidden)
}

impl PicassoApp {
    /// The rules for the grid as it is now: none outside a grid or when the
    /// photos aren't in the order they were taken.
    pub(super) fn stack_rules(&self) -> StackRules {
        let stacking = &self.stacking;
        if !shown_for(&self.nav) || !self.sort.by_date() {
            return StackRules::default();
        }
        StackRules {
            within: stacking.by_time.then_some(stacking.seconds),
            alike: stacking.alike,
        }
    }

    /// Every photo of the view, closed stacks' included, in grid order.
    pub(super) fn all_photos(&mut self) -> Vec<Photo> {
        let mut hidden = std::mem::take(&mut self.stacking.hidden);
        let photos = std::mem::take(&mut self.photos);
        if hidden.is_empty() {
            return photos;
        }
        let mut all =
            Vec::with_capacity(photos.len() + hidden.values().map(Vec::len).sum::<usize>());
        for photo in photos {
            let members = hidden.remove(&photo.id);
            all.push(photo);
            all.extend(members.into_iter().flatten());
        }
        all
    }

    /// Stacks the photos now in `photos`, which hold the whole view, as the
    /// settings say. Called by `show_photos` before the photos are indexed.
    pub(super) fn stack_photos(&mut self, keep: Option<i64>) {
        self.stacking.tops.clear();
        self.stacking.hidden.clear();
        let rules = self.stack_rules();
        if !rules.any() {
            return;
        }
        if rules.alike {
            self.find_looks();
        }
        let tiles: Vec<Tile> = self
            .photos
            .iter()
            .map(|photo| Tile {
                taken_at: photo.taken_at,
                day: day_key(photo.taken_at),
                look: self.stacking.looks.get(&photo.id).copied(),
                video: photo.is_video(),
            })
            .collect();
        let ranges = burst::stacks(&tiles, rules);
        if ranges.is_empty() {
            return;
        }
        let keep = keep.filter(|_| self.mode == super::Mode::View);
        let photos = std::mem::take(&mut self.photos);
        let (shown, tops, hidden) = fold(photos, &ranges, &self.stacking.open, keep);
        for (top, (_, open)) in &tops {
            if *open {
                self.stacking.open.insert(*top);
            }
        }
        self.photos = shown;
        self.stacking.tops = tops;
        self.stacking.hidden = hidden;
    }

    /// Stacks the view again, after a setting, a stack, or a look changed.
    fn restack(&mut self) {
        let keep = self.photos.get(self.viewer_index).map(|photo| photo.id);
        let photos = self.all_photos();
        self.show_photos(photos, keep);
    }

    /// Opens or closes the stack asked for last frame. Called before the
    /// grid is drawn, so it never changes under the frame drawing it.
    pub(super) fn apply_stack_toggle(&mut self) {
        let Some(top) = self.stacking.toggle.take() else {
            return;
        };
        if !self.stacking.open.remove(&top) {
            self.stacking.open.insert(top);
        }
        self.restack();
    }

    /// The stack settings, beside Edited only.
    pub(super) fn stack_controls(&mut self, ui: &mut egui::Ui) {
        let before = (
            self.stacking.by_time,
            self.stacking.seconds,
            self.stacking.alike,
        );
        let hint = tr("Photos taken one after another this close together show as one, with how many there are; click the number to see them all");
        let time = ui.checkbox(&mut self.stacking.by_time, tr("Stack photos taken within"));
        time.on_hover_text(hint);
        let seconds = ui.add_enabled(
            self.stacking.by_time,
            egui::DragValue::new(&mut self.stacking.seconds)
                .range(1..=MAX_SECONDS)
                .suffix(" s"),
        );
        theme::named(seconds, tr("Seconds apart"));
        let alike = ui.checkbox(&mut self.stacking.alike, tr("Stack look-alikes"));
        alike.on_hover_text(tr("Photos taken one after another on the same day that look alike show as one, with how many there are; click the number to see them all"));
        if let Some(finding) = &self.stacking.finding {
            ui.label(
                egui::RichText::new(trf(
                    "Finding look-alikes: {} of {}",
                    &[&finding.done, &finding.total],
                ))
                .color(theme::muted())
                .size(12.0),
            );
        }
        let after = (
            self.stacking.by_time,
            self.stacking.seconds,
            self.stacking.alike,
        );
        if after != before {
            if !self.stacking.alike {
                if let Some(finding) = self.stacking.finding.take() {
                    finding.cancel.store(true, Ordering::Relaxed);
                }
            }
            self.stacking.save(&self.db);
            self.restack();
        }
        if !self.sort.by_date() && (self.stacking.by_time || self.stacking.alike) {
            ui.label(
                egui::RichText::new(tr("Stacks show when sorted by date"))
                    .color(theme::muted())
                    .size(12.0),
            );
        }
    }

    /// The number on the photo on top of a stack, which opens or closes it.
    pub(super) fn stack_badge(&mut self, ui: &mut egui::Ui, id: i64, image_rect: Rect) {
        let Some(&(count, open)) = self.stacking.tops.get(&id) else {
            return;
        };
        let painter = ui.painter();
        let galley = painter.layout_no_wrap(count.to_string(), FontId::proportional(11.5), WHITE);
        let icon = Vec2::new(12.0, 10.0);
        let size = Vec2::new(
            icon.x + galley.size().x + 16.0,
            galley.size().y.max(icon.y) + 6.0,
        );
        // Right of the selection circle in the top-left corner.
        let pill = Rect::from_min_size(image_rect.left_top() + Vec2::new(32.0, 6.0), size);
        let name = stack_name(count, open);
        let response = ui
            .interact(pill, egui::Id::unique(("stack", id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        let fill = if open {
            ACCENT
        } else if response.hovered() {
            Color32::from_black_alpha(190)
        } else {
            Color32::from_black_alpha(150)
        };
        let painter = ui.painter();
        painter.rect_filled(pill, 8, fill);
        // Two cards, one behind the other.
        let card = Rect::from_min_size(
            pill.left_center() + Vec2::new(6.0, -icon.y / 2.0 + 2.0),
            Vec2::new(icon.x - 3.0, icon.y - 3.0),
        );
        let stroke = egui::Stroke::new(1.2, WHITE);
        painter.rect_stroke(
            card.translate(Vec2::new(3.0, -2.0)),
            1,
            stroke,
            egui::StrokeKind::Inside,
        );
        painter.rect_filled(card, 1, fill);
        painter.rect_stroke(card, 1, stroke, egui::StrokeKind::Inside);
        painter.text(
            pill.right_center() - Vec2::new(6.0, 0.0),
            Align2::RIGHT_CENTER,
            count.to_string(),
            FontId::proportional(11.5),
            WHITE,
        );
        if response.on_hover_text(&name).clicked() {
            self.stacking.toggle = Some(id);
        }
    }

    /// The stack item of a photo's menu, when the photo is on top of one.
    pub(super) fn stack_menu_item(&mut self, ui: &mut egui::Ui, id: i64) {
        if let Some(&(count, open)) = self.stacking.tops.get(&id) {
            if ui.button(stack_name(count, open)).clicked() {
                self.stacking.toggle = Some(id);
                ui.close();
            }
        }
    }

    /// Starts finding the looks of the view's photos that have none yet, on
    /// a thread of its own, unless that is already running. The looks stored
    /// with the thumbnails come back first.
    fn find_looks(&mut self) {
        if self.stacking.finding.is_some() {
            return;
        }
        let wanted: Vec<(i64, String)> = self
            .photos
            .iter()
            .chain(self.stacking.hidden.values().flatten())
            .filter(|photo| {
                !photo.is_video()
                    && !self.stacking.looks.contains_key(&photo.id)
                    && !self.stacking.no_look.contains(&photo.id)
            })
            .map(|photo| (photo.id, photo.path.clone()))
            .collect();
        if wanted.is_empty() {
            return;
        }
        self.stacking.run += 1;
        let run = self.stacking.run;
        let cancel = Arc::new(AtomicBool::new(false));
        self.stacking.finding = Some(Finding {
            run,
            cancel: cancel.clone(),
            done: 0,
            total: wanted.len(),
            stored: false,
        });
        let cache = self.cache_dir.clone();
        let events = self.events_tx.clone();
        let ctx = self.stacking.ctx.clone();
        let _ = std::thread::Builder::new()
            .name("picasso-looks".into())
            .spawn(move || {
                let store = crate::thumbs::ThumbStore::open(&cache);
                let stored = store.looks();
                let send =
                    |looks: Vec<(i64, u64)>, no_look: Vec<i64>, done: usize, finished: bool| {
                        let _ = events.send(UiEvent::Looks {
                            run,
                            looks,
                            no_look,
                            done,
                            finished,
                        });
                        ctx.request_repaint();
                    };
                let (known, missing): (Vec<_>, Vec<_>) = wanted
                    .into_iter()
                    .partition(|(id, _)| stored.contains_key(id));
                let mut done = known.len();
                send(
                    known.iter().map(|(id, _)| (*id, stored[id])).collect(),
                    Vec::new(),
                    done,
                    missing.is_empty(),
                );
                let (mut looks, mut no_look) = (Vec::new(), Vec::new());
                let count = missing.len();
                for (index, (id, path)) in missing.into_iter().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    match crate::worker::plain_thumb(&store, id, Path::new(&path)) {
                        Some(thumb) => {
                            let look = burst::look(&thumb);
                            store.put_look(id, look);
                            looks.push((id, look));
                        }
                        None => no_look.push(id),
                    }
                    done += 1;
                    let last = index + 1 == count;
                    if last || looks.len() + no_look.len() >= LOOKS_EVERY {
                        send(
                            std::mem::take(&mut looks),
                            std::mem::take(&mut no_look),
                            done,
                            last,
                        );
                    }
                }
            });
    }

    /// Looks found in the background: the grid stacks again with the first
    /// ones, which were stored already, and once all have been found.
    pub(super) fn on_looks(
        &mut self,
        run: u64,
        looks: Vec<(i64, u64)>,
        no_look: Vec<i64>,
        done: usize,
        finished: bool,
    ) {
        let Some(finding) = self.stacking.finding.as_mut().filter(|f| f.run == run) else {
            return;
        };
        let first = !std::mem::replace(&mut finding.stored, true);
        finding.done = done;
        self.stacking.looks.extend(looks);
        self.stacking.no_look.extend(no_look);
        if finished {
            self.stacking.finding = None;
        }
        if (first || finished) && self.stack_rules().alike {
            self.restack();
        }
    }
}

/// What the number on a stack says to a screen reader and on hover.
fn stack_name(count: usize, open: bool) -> String {
    if open {
        trf("Stack these {} photos again", &[&count])
    } else {
        trf("Show the {} photos of this stack", &[&count])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(id: i64) -> Photo {
        Photo {
            id,
            folder_id: 0,
            path: String::new(),
            filename: format!("{id}.jpg"),
            width: 0,
            height: 0,
            bytes: 0,
            taken_at: 0,
            modified_at: 0,
            starred: false,
            caption: String::new(),
            camera: String::new(),
            lens: String::new(),
            focal_mm: 0.0,
            gps_lat: None,
            gps_lon: None,
            pin_lat: None,
            pin_lon: None,
            rotation: 0,
            duration: 0.0,
        }
    }

    fn ids(photos: &[Photo]) -> Vec<i64> {
        photos.iter().map(|photo| photo.id).collect()
    }

    #[test]
    fn a_closed_stack_shows_its_top_and_keeps_the_rest_aside() {
        let photos: Vec<Photo> = (1..=7).map(photo).collect();
        let ranges = [1..4, 5..7];
        let (shown, tops, hidden) = fold(photos, &ranges, &HashSet::new(), None);
        // Indices 1–3 are photos 2–4, indices 5–6 photos 6–7.
        assert_eq!(ids(&shown), [1, 2, 5, 6]);
        assert_eq!(tops, HashMap::from([(2, (3, false)), (6, (2, false))]));
        assert_eq!(ids(&hidden[&2]), [3, 4]);
        assert_eq!(ids(&hidden[&6]), [7]);
    }

    #[test]
    fn an_open_stack_and_the_photo_in_the_viewer_stay_in_view() {
        let photos: Vec<Photo> = (1..=6).map(photo).collect();
        let ranges = [0..2, 3..6];
        let (shown, tops, hidden) = fold(photos.clone(), &ranges, &HashSet::from([1]), None);
        assert_eq!(ids(&shown), [1, 2, 3, 4]);
        assert_eq!(tops[&1], (2, true));
        assert_eq!(tops[&4], (3, false));
        assert_eq!(ids(&hidden[&4]), [5, 6]);
        // The viewer is on photo 6, in the closed stack: it opens.
        let (shown, tops, hidden) = fold(photos, &ranges, &HashSet::new(), Some(6));
        assert_eq!(ids(&shown), [1, 3, 4, 5, 6]);
        assert_eq!(tops[&4], (3, true));
        assert!(!hidden.contains_key(&4));
        assert_eq!(ids(&hidden[&1]), [2]);
    }

    #[test]
    fn only_grids_in_date_order_stack() {
        for nav in [
            Nav::Library,
            Nav::Starred,
            Nav::Album(1),
            Nav::Trip(1),
            Nav::Folder(1),
        ] {
            assert!(shown_for(&nav), "{nav:?}");
        }
        for nav in [Nav::Places, Nav::TravelMap, Nav::Statistics, Nav::Trips] {
            assert!(!shown_for(&nav), "{nav:?}");
        }
    }

    #[test]
    fn the_number_names_what_it_does() {
        assert_eq!(stack_name(4, false), "Show the 4 photos of this stack");
        assert_eq!(stack_name(4, true), "Stack these 4 photos again");
    }
}
