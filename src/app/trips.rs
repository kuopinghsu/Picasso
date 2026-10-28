//! Trips in the window: their section of the side bar, adding the selected
//! photos to one, the dialogs that create, change, and delete a Trip, and
//! the line about a Trip above its photos. `docs/TRAVEL.md` describes the
//! model; the library operations are in `src/db/trips.rs`.

use chrono::{Local, NaiveDate, TimeZone};
use eframe::egui::{self, Align, Layout, Margin, RichText, Vec2};

use crate::i18n::{tr, trf};
use crate::trip::{read_capture, TripQuery, AddPreview, DayFilter, DaySource, DaySuggestion, MemberQuery, Review, Trip, TripDay, TripDraft, TripImpact};
use crate::types::{format_day, Nav};

use super::story::story_name;
use super::{about, theme, tree_section, Glyph, Modal, PicassoApp, UiEvent};

/// Creating a Trip, from the selected photos or empty, or changing one.
pub(super) struct TripDialog {
    /// The Trip being changed; `None` for a new one.
    editing: Option<i64>,
    title: String,
    zone: String,
    trip_time: bool,
    destination: String,
    description: String,
    first_day: String,
    last_day: String,
    /// Photos a new Trip starts with, in the order shown.
    photos: Vec<i64>,
    preview: AddPreview,
    error: String,
}

/// Adding the selected photos to a Trip, after showing what that does.
pub(super) struct AddDialog {
    trip_id: i64,
    name: String,
    photos: Vec<i64>,
    preview: AddPreview,
}

/// Changing a Day's title, date, and notes.
pub(super) struct DayDialog {
    day_id: i64,
    title: String,
    date: String,
    notes: String,
    error: String,
}

/// Most unavailable photos the repair list shows at once.
const UNAVAILABLE_SHOWN: usize = 200;

/// The photos of a Trip that are no longer in the library, to link again.
pub(super) struct UnavailableDialog {
    trip_id: i64,
    error: String,
}

/// Deleting a Trip, after showing what goes with it.
pub(super) struct DeleteDialog {
    trip_id: i64,
    name: String,
    impact: TripImpact,
}

impl PicassoApp {
    pub(super) fn trips_sidebar(&mut self, ui: &mut egui::Ui) {
        tree_section(ui, tr("Trips"));
        let all = self.tree_row(ui, tr("All trips"), Glyph::Trip, self.nav == Nav::Trips, Some(self.trips.len() as i64));
        if all.on_hover_text(tr("Find, open, and compare your trips")).clicked() {
            self.set_nav(Nav::Trips);
        }
        let active: Vec<(i64, String, i64)> =
            self.trips.iter().filter(|trip| !trip.archived).map(|trip| (trip.id, trip.title.clone(), trip.available)).collect();
        for (id, title, count) in active {
            let response = self.tree_row(ui, &title, Glyph::Trip, self.nav == Nav::Trip(id), Some(count));
            if response.clicked() {
                self.set_nav(Nav::Trip(id));
            }
            response.context_menu(|ui| {
                if ui.button(tr("Edit trip…")).clicked() {
                    self.open_edit_trip(id);
                    ui.close();
                }
                if ui.button(tr("Archive trip")).clicked() {
                    self.db.set_trip_archived(id, true);
                    self.reload();
                    ui.close();
                }
                if ui.button(tr("Delete trip…")).clicked() {
                    self.open_delete_trip(id);
                    ui.close();
                }
            });
        }
        if ui.add(egui::Button::new(RichText::new(tr("+  New trip")).color(theme::muted())).frame(false)).clicked() {
            self.open_new_trip(Vec::new());
        }
        let archived: Vec<(i64, String, i64)> =
            self.trips.iter().filter(|trip| trip.archived).map(|trip| (trip.id, trip.title.clone(), trip.available)).collect();
        if archived.is_empty() {
            return;
        }
        let arrow = if self.show_archived_trips { "▾" } else { "▸" };
        let label = format!("{arrow}  {}", trf("Archived trips ({})", &[&archived.len()]));
        if ui.add(egui::Button::new(RichText::new(label).color(theme::muted())).frame(false)).clicked() {
            self.show_archived_trips = !self.show_archived_trips;
        }
        if !self.show_archived_trips {
            return;
        }
        for (id, title, count) in archived {
            let response = self.tree_row(ui, &title, Glyph::Trip, self.nav == Nav::Trip(id), Some(count));
            if response.clicked() {
                self.set_nav(Nav::Trip(id));
            }
            response.context_menu(|ui| {
                if ui.button(tr("Unarchive trip")).clicked() {
                    self.db.set_trip_archived(id, false);
                    self.reload();
                    ui.close();
                }
                if ui.button(tr("Delete trip…")).clicked() {
                    self.open_delete_trip(id);
                    ui.close();
                }
            });
        }
    }

    /// Every Trip as a card with its cover, dates, and what is in it. The
    /// header's search finds Trips by title, destination, or description.
    pub(super) fn trips_page(&mut self, ui: &mut egui::Ui) {
        const CARD: Vec2 = Vec2::new(260.0, 250.0);
        let query = TripQuery { archived: self.show_archived_trips, search: self.search.trim().to_string() };
        let trips = self.db.trips(&query);
        let mut open: Option<i64> = None;
        let mut archive: Option<(i64, bool)> = None;
        let mut edit: Option<i64> = None;
        let mut delete: Option<i64> = None;
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button(tr("New trip"))).clicked() {
                self.open_new_trip(Vec::new());
            }
            ui.add_space(8.0);
            ui.checkbox(&mut self.show_archived_trips, tr("Archived trips"));
            if !query.search.is_empty() {
                ui.label(RichText::new(trf("{} found", &[&trips.len()])).size(13.0).color(theme::muted()));
            }
        });
        ui.add_space(8.0);
        if trips.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                let text = if !query.search.is_empty() {
                    tr("No trip matches that search.")
                } else if query.archived {
                    tr("No archived trips.")
                } else {
                    tr("No trips yet. Select photos and choose Add to Trip, or start a new trip.")
                };
                ui.label(RichText::new(text).color(theme::muted()));
            });
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(14.0);
                for trip in &trips {
                    let (rect, response) = ui.allocate_exact_size(CARD, egui::Sense::click());
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    let painter = ui.painter_at(rect);
                    let hovered = response.hovered();
                    painter.rect_filled(rect, 8, theme::sidebar());
                    if hovered {
                        painter.rect_stroke(rect, 8, egui::Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
                    }
                    let cover = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), 150.0)).shrink(6.0);
                    painter.rect_filled(cover, 6, theme::placeholder());
                    if let Some(photo) = self.db.trip_cover_photo(trip.id).and_then(|id| self.db.photo(id)) {
                        self.ensure_thumb(photo.id, &photo.path);
                        if let Some((texture, w, h)) = self.thumb_texture(photo.id) {
                            theme::paint_image_rounded(&painter, &texture, cover, theme::cover_uv(w, h), photo.rotation, 6.0);
                        }
                    }
                    let mut y = cover.bottom() + 8.0;
                    let left = rect.left() + 12.0;
                    let width = rect.width() - 24.0;
                    // One line each, cut at the card's edge.
                    let mut line = |text: String, size: f32, color: egui::Color32| {
                        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(size), color);
                        let height = galley.size().y;
                        let clip = egui::Rect::from_min_size(egui::pos2(left, y), Vec2::new(width, height));
                        painter.with_clip_rect(clip).galley(clip.min, galley, color);
                        y += height + 4.0;
                    };
                    line(trip.title.clone(), 15.0, theme::text());
                    let mut place = trip_dates(trip).into_iter().collect::<Vec<_>>();
                    if !trip.destination.is_empty() {
                        place.push(trip.destination.clone());
                    }
                    if !place.is_empty() {
                        line(place.join(" · "), 12.5, theme::muted());
                    }
                    let tracks = self.db.trip_tracks(trip.id).len();
                    let counts = trf("{} photos · {} days · {} stories · {} tracks", &[&trip.members, &trip.days, &trip.stories, &tracks]);
                    line(counts, 12.0, theme::muted());
                    if trip.members > trip.available {
                        line(trf("{} unavailable", &[&(trip.members - trip.available)]), 12.0, theme::DANGER);
                    }
                    if response.clicked() {
                        open = Some(trip.id);
                    }
                    response.context_menu(|ui| {
                        if ui.button(tr("Edit trip…")).clicked() {
                            edit = Some(trip.id);
                            ui.close();
                        }
                        let label = if trip.archived { tr("Unarchive trip") } else { tr("Archive trip") };
                        if ui.button(label).clicked() {
                            archive = Some((trip.id, !trip.archived));
                            ui.close();
                        }
                        if ui.button(tr("Delete trip…")).clicked() {
                            delete = Some(trip.id);
                            ui.close();
                        }
                    });
                }
            });
        });
        if let Some(id) = edit {
            self.open_edit_trip(id);
        }
        if let Some((id, archived)) = archive {
            self.db.set_trip_archived(id, archived);
            self.reload();
        }
        if let Some(id) = delete {
            self.open_delete_trip(id);
        }
        if let Some(id) = open {
            self.set_nav(Nav::Trip(id));
        }
    }

    /// The toolbar menu that adds the selected photos to a Trip, or starts
    /// a new one with them.
    pub(super) fn trip_menu(&mut self, ui: &mut egui::Ui) {
        let mut pick = None;
        let button = ui.button(tr("Add to Trip"));
        let popup = egui::Popup::menu(&button).show(|ui| pick = self.trip_choices(ui));
        if popup.is_none() {
            button.on_hover_text(tr("Add the selected photos to a trip"));
        }
        if let Some(pick) = pick {
            let photos = self.selection_in_order();
            self.add_photos_to_trip(pick, photos);
        }
    }

    /// The Trips to add photos to, and New trip, as menu items. Returns
    /// the Trip picked, or `None` inside for a new one.
    pub(super) fn trip_choices(&self, ui: &mut egui::Ui) -> Option<Option<(i64, String)>> {
        let mut pick = None;
        let trips = self.trips.iter().filter(|trip| !trip.archived);
        let mut any = false;
        for trip in trips {
            any = true;
            if ui.selectable_label(false, &trip.title).clicked() {
                pick = Some(Some((trip.id, trip.title.clone())));
            }
        }
        if any {
            ui.separator();
        }
        if ui.button(tr("New trip…")).clicked() {
            pick = Some(None);
        }
        pick
    }

    /// Adds `photos` to the Trip picked from [`Self::trip_choices`], after a
    /// preview, or starts a new Trip with them.
    pub(super) fn add_photos_to_trip(&mut self, pick: Option<(i64, String)>, photos: Vec<i64>) {
        let add = pick.clone();
        let create = pick.is_none();
        if let Some((trip_id, name)) = add {
            let preview = self.db.preview_add_to_trip(trip_id, &photos);
            if preview.photos + preview.videos == 0 {
                self.activity = trf("All {} are already in this trip", &[&photos.len()]);
            } else {
                self.modal = Modal::AddToTrip(AddDialog { trip_id, name, photos: photos.clone(), preview });
            }
        }
        if create {
            self.open_new_trip(photos);
        }
    }

    pub(super) fn remove_selection_from_trip(&mut self) {
        let Nav::Trip(trip_id) = self.nav else { return };
        let ids = self.selection_in_order();
        match self.db.remove_photos_from_trip(trip_id, &ids) {
            Ok(removed) => self.activity = trf("Removed {} from trip", &[&removed]),
            Err(err) => self.activity = err,
        }
        self.selection.clear();
        self.drop_from_view(&ids);
        self.reload();
    }

    /// Dates, destination, Days, Stories, and unavailable photos of the Trip
    /// on view, under its title.
    pub(super) fn trip_header(&mut self, ui: &mut egui::Ui) {
        let Nav::Trip(trip_id) = self.nav else { return };
        let Some(trip) = self.trips.iter().find(|trip| trip.id == trip_id) else { return };
        let mut parts: Vec<String> = trip_dates(trip).into_iter().collect();
        if !trip.destination.is_empty() {
            parts.push(trip.destination.clone());
        }
        parts.push(trip.zone.clone());
        if trip.days > 0 {
            parts.push(trf(if trip.days == 1 { "{} day" } else { "{} days" }, &[&trip.days]));
        }
        if trip.stories > 0 {
            parts.push(trf(if trip.stories == 1 { "{} story" } else { "{} stories" }, &[&trip.stories]));
        }
        let missing = trip.members - trip.available;
        let trip_members = trip.members;
        let archived = trip.archived;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(parts.join("  ·  ")).size(13.0).color(theme::muted()));
            if missing > 0 {
                let unavailable = ui.link(RichText::new(format!("⚠ {}", trf("{} unavailable", &[&missing]))).size(13.0).color(theme::DANGER));
                let unavailable = unavailable.on_hover_text(tr(
                    "Photos of this trip that are no longer in the library. They stay in the trip and come back when their files do.",
                ));
                if unavailable.clicked() {
                    self.modal = Modal::Unavailable(UnavailableDialog { trip_id, error: String::new() });
                }
            }
            if archived {
                ui.label(RichText::new(tr("Archived")).size(13.0).color(theme::muted()));
            }
            if ui.link(RichText::new(tr("Edit trip…")).size(13.0)).clicked() {
                self.open_edit_trip(trip_id);
            }
        });
        // Narrow the grid; the counts and the list come from the same query.
        let mut filter = self.trip_filter;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Show")).size(13.0).color(theme::muted()));
            if ui.selectable_label(!filter.starred && !filter.no_location, tr("All")).clicked() {
                filter = MemberQuery::default();
            }
            if ui.selectable_label(filter.starred, format!("★ {}", tr("Starred"))).clicked() {
                filter.starred = !filter.starred;
            }
            let no_location = ui.selectable_label(filter.no_location, tr("No location"));
            if no_location.on_hover_text(tr("Photos without GPS from the camera or a place set by hand")).clicked() {
                filter.no_location = !filter.no_location;
            }
            if filter != MemberQuery::default() {
                let shown = self.db.count_trip_members(trip_id, &filter);
                ui.label(RichText::new(trf("{} of {}", &[&shown, &trip_members])).size(13.0).color(theme::muted()));
            }
        });
        let days = self.db.trip_days(trip_id);
        let unassigned = self.db.count_trip_members(trip_id, &MemberQuery { day: DayFilter::Unassigned, ..MemberQuery::default() });
        let mut edit_day: Option<TripDay> = None;
        let mut delete_day: Option<i64> = None;
        let mut move_day: Option<(usize, usize)> = None;
        let mut suggest = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Days")).size(13.0).color(theme::muted()));
            if ui.selectable_label(filter.day == DayFilter::Any, tr("All")).clicked() {
                filter.day = DayFilter::Any;
            }
            for (index, day) in days.iter().enumerate() {
                let label = format!("{} ({})", day_name(day), day.members);
                let chip = ui.selectable_label(filter.day == DayFilter::Day(day.id), label);
                if chip.clicked() {
                    filter.day = DayFilter::Day(day.id);
                }
                chip.context_menu(|ui| {
                    if ui.button(tr("Edit day…")).clicked() {
                        edit_day = Some(day.clone());
                        ui.close();
                    }
                    if ui.add_enabled(index > 0, egui::Button::new(tr("Move earlier"))).clicked() {
                        move_day = Some((index, index - 1));
                        ui.close();
                    }
                    if ui.add_enabled(index + 1 < days.len(), egui::Button::new(tr("Move later"))).clicked() {
                        move_day = Some((index, index + 1));
                        ui.close();
                    }
                    if ui.button(tr("Delete day")).on_hover_text(tr("Its photos stay in the trip, without a day")).clicked() {
                        delete_day = Some(day.id);
                        ui.close();
                    }
                });
            }
            let review = self.db.review_counts(trip_id);
            let to_review: usize = review.iter().map(|(_, count)| count).sum();
            if to_review > 0 || filter.day == DayFilter::Review {
                let chip = ui.selectable_label(filter.day == DayFilter::Review, trf("Needs review ({})", &[&to_review]));
                let mut why = tr("Suggest days couldn't tell which day these were taken. Put them in a day to settle them.").to_string();
                for &(reason, count) in &review {
                    why.push('\n');
                    why.push_str(&review_text(reason, count));
                }
                if chip.on_hover_text(why).clicked() {
                    filter.day = DayFilter::Review;
                }
            }
            if unassigned > 0 || filter.day == DayFilter::Unassigned {
                let label = trf("Not in a day ({})", &[&unassigned]);
                if ui.selectable_label(filter.day == DayFilter::Unassigned, label).clicked() {
                    filter.day = DayFilter::Unassigned;
                }
            }
            let busy = self.suggesting.is_some();
            let button = ui.add_enabled(!busy, egui::Button::new(tr("Suggest days")));
            if button
                .on_hover_text(tr("Put photos in days by when they were taken, in the trip's time zone. Days you chose stay as they are."))
                .clicked()
            {
                suggest = true;
            }
            if busy {
                ui.add(egui::Spinner::new().size(13.0));
            }
            if let Some((undo_trip, _)) = &self.day_undo {
                if *undo_trip == trip_id && ui.link(RichText::new(tr("Undo")).size(13.0)).clicked() {
                    self.undo_day_move();
                }
            }
        });
        if let Some(day) = edit_day {
            self.modal = Modal::DayDetails(DayDialog {
                day_id: day.id,
                date: day.day.map(|day| day.to_string()).unwrap_or_default(),
                title: day.title,
                notes: day.notes,
                error: String::new(),
            });
        }
        if let Some(day_id) = delete_day {
            if let Err(err) = self.db.delete_trip_day(day_id) {
                self.activity = err;
            }
            if filter.day == DayFilter::Day(day_id) {
                filter.day = DayFilter::Any;
            }
            self.reload();
        }
        if let Some((from, to)) = move_day {
            let mut order: Vec<i64> = days.iter().map(|day| day.id).collect();
            order.swap(from, to);
            if let Err(err) = self.db.reorder_trip_days(trip_id, &order) {
                self.activity = err;
            }
        }
        if suggest {
            self.suggest_days(ui.ctx(), trip_id);
        }
        self.tracks_row(ui, trip_id, &days);
        self.stories_row(ui, trip_id, &days);
        if filter != self.trip_filter {
            self.trip_filter = filter;
            self.selection.clear();
            self.reload_view();
        }
    }

    /// The Trip's Stories: open one, archive or delete it, or start a new one.
    fn stories_row(&mut self, ui: &mut egui::Ui, trip_id: i64, days: &[TripDay]) {
        let stories = self.db.stories(trip_id);
        let mut open: Option<i64> = None;
        let mut archive: Option<(i64, bool)> = None;
        let mut delete: Option<(i64, String)> = None;
        let mut blank = false;
        let mut from_days: Option<Vec<i64>> = None;
        let mut from_selection = false;
        let selected = !self.selection.is_empty();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Stories")).size(13.0).color(theme::muted()));
            for story in &stories {
                let name = story_name(story);
                let label = if story.archived {
                    RichText::new(trf("{} (archived)", &[&name])).color(theme::muted())
                } else {
                    RichText::new(name.clone())
                };
                let chip = ui.selectable_label(false, label).on_hover_text(tr("Click to edit. Right-click for more."));
                if chip.clicked() {
                    open = Some(story.id);
                }
                chip.context_menu(|ui| {
                    if ui.button(tr("Edit")).clicked() {
                        open = Some(story.id);
                        ui.close();
                    }
                    let label = if story.archived { tr("Restore story") } else { tr("Archive story") };
                    if ui.button(label).clicked() {
                        archive = Some((story.id, !story.archived));
                        ui.close();
                    }
                    if ui.button(tr("Delete story…")).clicked() {
                        delete = Some((story.id, name.clone()));
                        ui.close();
                    }
                });
            }
            let button = ui.button(tr("New story"));
            let popup = egui::Popup::menu(&button).show(|ui| {
                if ui.button(tr("Blank story")).clicked() {
                    blank = true;
                }
                let filled = days.iter().any(|day| day.members > 0);
                let by_day = ui.add_enabled(filled, egui::Button::new(tr("From the trip's days")));
                if by_day
                    .on_hover_text(tr("A heading for each day and its photos in order, to write around. Videos are left out."))
                    .on_disabled_hover_text(tr("Put photos in days first"))
                    .clicked()
                {
                    from_days = Some(days.iter().filter(|day| day.members > 0).map(|day| day.id).collect());
                }
                ui.add_enabled_ui(filled, |ui| {
                    ui.menu_button(tr("From one day"), |ui| {
                        for day in days.iter().filter(|day| day.members > 0) {
                            if ui.button(day_name(day)).clicked() {
                                from_days = Some(vec![day.id]);
                            }
                        }
                    });
                });
                let picked = ui.add_enabled(selected, egui::Button::new(tr("From the selected photos")));
                if picked.on_disabled_hover_text(tr("Select photos first")).clicked() {
                    from_selection = true;
                }
            });
            if popup.is_none() {
                button.on_hover_text(tr("Write about this trip"));
            }
        });
        if blank {
            match self.db.create_story(trip_id, "") {
                Ok(id) => open = Some(id),
                Err(err) => self.activity = err,
            }
        }
        if from_selection {
            match self.db.create_story(trip_id, &self.trip_title(trip_id)) {
                Ok(id) => {
                    self.add_selection_to_story(id, false);
                    open = Some(id);
                }
                Err(err) => self.activity = err,
            }
        }
        if let Some(ids) = from_days {
            let title = self.trip_title(trip_id);
            match self.db.create_story_from_days(trip_id, &title, &ids, day_label) {
                Ok((id, videos)) => {
                    if videos > 0 {
                        self.activity = trf("{} videos were left out of the story", &[&videos]);
                    }
                    open = Some(id);
                }
                Err(err) => self.activity = err,
            }
        }
        if let Some((id, archived)) = archive {
            self.db.set_story_archived(id, archived);
        }
        if let Some((id, name)) = delete {
            self.modal = Modal::DeleteStory(id, name);
        }
        if let Some(id) = open {
            self.open_story(id);
        }
    }

    /// The toolbar menu that adds the selected photos to one of the Trip's Stories.
    pub(super) fn add_to_story_menu(&mut self, ui: &mut egui::Ui) {
        let Nav::Trip(trip_id) = self.nav else { return };
        let stories: Vec<_> = self.db.stories(trip_id).into_iter().filter(|story| !story.archived).collect();
        if stories.is_empty() {
            return;
        }
        let mut pick: Option<(i64, bool)> = None;
        let button = ui.button(tr("Add to Story"));
        let popup = egui::Popup::menu(&button).show(|ui| {
            for story in &stories {
                ui.menu_button(story_name(story), |ui| {
                    if ui.button(tr("As separate photos")).clicked() {
                        pick = Some((story.id, false));
                    }
                    if ui.button(tr("As one gallery")).clicked() {
                        pick = Some((story.id, true));
                    }
                });
            }
        });
        if popup.is_none() {
            button.on_hover_text(tr("Add the selected photos to the end of a story"));
        }
        if let Some((story_id, gallery)) = pick {
            self.add_selection_to_story(story_id, gallery);
        }
    }

    /// The library's GPS tracks linked to the Trip, each to the whole Trip
    /// or to one of its Days. Shown once the library has a track.
    fn tracks_row(&mut self, ui: &mut egui::Ui, trip_id: i64, days: &[TripDay]) {
        let linked = self.db.trip_tracks(trip_id);
        let choices = self.db.track_choices();
        if linked.is_empty() && choices.is_empty() {
            return;
        }
        let mut link: Option<(i64, Option<i64>)> = None;
        let mut unlink: Option<i64> = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Tracks")).size(13.0).color(theme::muted()));
            for track in &linked {
                let day = track.day_id.and_then(|id| days.iter().find(|day| day.id == id));
                let mut label = match day {
                    Some(day) => format!("{} · {}", track.name, day_name(day)),
                    None => track.name.clone(),
                };
                if track.track_id.is_none() {
                    label = trf("{} (unavailable)", &[&label]);
                }
                let chip = ui.selectable_label(false, label);
                let chip = if track.track_id.is_none() {
                    chip.on_hover_text(tr("Its file is no longer in the library. The link comes back when the library finds it again."))
                } else {
                    chip.on_hover_text(tr("Right-click to choose its day or remove it"))
                };
                chip.context_menu(|ui| {
                    if let Some(track_id) = track.track_id {
                        if ui.selectable_label(track.day_id.is_none(), tr("Whole trip")).clicked() {
                            link = Some((track_id, None));
                            ui.close();
                        }
                        for day in days {
                            if ui.selectable_label(track.day_id == Some(day.id), day_name(day)).clicked() {
                                link = Some((track_id, Some(day.id)));
                                ui.close();
                            }
                        }
                        ui.separator();
                    }
                    if ui.button(tr("Remove from Trip")).clicked() {
                        unlink = Some(track.id);
                        ui.close();
                    }
                });
            }
            let unlinked: Vec<_> =
                choices.iter().filter(|(id, _, _)| !linked.iter().any(|track| track.track_id == Some(*id))).collect();
            if !unlinked.is_empty() {
                let button = ui.button(tr("Add track"));
                let popup = egui::Popup::menu(&button).show(|ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for (id, name, started) in &unlinked {
                            let label = match started {
                                Some(at) => format!("{name} · {}", format_day(*at)),
                                None => name.clone(),
                            };
                            if ui.selectable_label(false, label).clicked() {
                                link = Some((*id, None));
                            }
                        }
                    });
                });
                if popup.is_none() {
                    button.on_hover_text(tr("Link a GPS track from the library to this trip. The track itself stays where it is."));
                }
            }
        });
        if let Some((track_id, day_id)) = link {
            if let Err(err) = self.db.link_trip_track(trip_id, track_id, day_id) {
                self.activity = err;
            }
        }
        if let Some(link_id) = unlink {
            self.db.unlink_trip_track(link_id);
        }
    }

    pub(super) fn unavailable_modal(&mut self, ctx: &egui::Context) {
        let Modal::Unavailable(dialog) = &self.modal else { return };
        let trip_id = dialog.trip_id;
        let error = dialog.error.clone();
        let query = MemberQuery { unavailable: true, ..MemberQuery::default() };
        let members = self.db.trip_members(trip_id, &query, 0, UNAVAILABLE_SHOWN);
        let total = self.db.count_trip_members(trip_id, &query);
        let mut close = false;
        let mut locate: Option<i64> = None;
        let response = egui::Modal::new(egui::Id::unique("trip-unavailable")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(560.0);
            about::title_bar(ui, &self.logo, tr("Unavailable photos"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(RichText::new(tr(
                    "These photos are no longer in the library, perhaps because their drive is disconnected or their folder was removed. They come back by themselves when the library finds their files again. To link one by hand, locate its file.",
                ))
                .color(theme::muted()));
                if members.is_empty() {
                    ui.label(tr("Every photo of this trip is in the library."));
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for member in &members {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(430.0);
                                ui.label(&member.filename);
                                ui.label(RichText::new(&member.path).size(12.0).color(theme::muted()));
                            });
                            if ui.button(tr("Locate…")).clicked() {
                                locate = Some(member.media_id);
                            }
                        });
                        ui.separator();
                    }
                });
                if total > members.len() as i64 {
                    ui.label(RichText::new(trf("{} of {}", &[&members.len(), &total])).color(theme::muted()));
                }
                if !error.is_empty() {
                    ui.label(RichText::new(&error).color(theme::DANGER));
                }
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(theme::primary_button(tr("Done"))).clicked() {
                        close = true;
                    }
                });
            });
        });
        if let Some(media_id) = locate {
            let error = self.locate_member(media_id);
            if let Modal::Unavailable(dialog) = &mut self.modal {
                dialog.error = error;
            }
            return;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
        }
    }

    /// Reads the capture times the Trip's photos lack, then suggests their
    /// Days, on a thread of its own.
    fn suggest_days(&mut self, ctx: &egui::Context, trip_id: i64) {
        self.suggesting = Some(trip_id);
        self.day_undo = None;
        let db_path = self.data_dir.join("picasso.db");
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-days".into()).spawn(move || {
            let outcome = crate::db::Database::open(&db_path).and_then(|db| {
                for (media_id, path, modified_at) in db.captures_to_read(trip_id) {
                    db.store_capture(media_id, read_capture(std::path::Path::new(&path), modified_at), modified_at);
                }
                db.suggest_days(trip_id)
            });
            let _ = events.send(UiEvent::DaysSuggested { trip_id, outcome });
            ctx.request_repaint();
        });
    }

    pub(super) fn on_days_suggested(&mut self, trip_id: i64, outcome: Result<DaySuggestion, String>) {
        if self.suggesting == Some(trip_id) {
            self.suggesting = None;
        }
        self.activity = match outcome {
            Ok(outcome) => {
                let mut text = trf("Suggested days for {} photos", &[&outcome.assigned]);
                for &(reason, count) in &outcome.review {
                    text.push_str(" · ");
                    text.push_str(&review_text(reason, count));
                }
                text
            }
            Err(err) => err,
        };
        self.reload();
    }

    /// The toolbar menu that puts the selected photos in a Day of the Trip.
    pub(super) fn day_menu(&mut self, ui: &mut egui::Ui) {
        let Nav::Trip(trip_id) = self.nav else { return };
        let days = self.db.trip_days(trip_id);
        let mut target: Option<Option<i64>> = None;
        let mut create = false;
        let button = ui.button(tr("Move to Day"));
        let popup = egui::Popup::menu(&button).show(|ui| {
            for day in &days {
                if ui.selectable_label(false, day_name(day)).clicked() {
                    target = Some(Some(day.id));
                }
            }
            if !days.is_empty() {
                ui.separator();
            }
            if ui.button(tr("New day")).clicked() {
                create = true;
            }
            if ui.button(tr("No day")).clicked() {
                target = Some(None);
            }
        });
        if popup.is_none() {
            button.on_hover_text(tr("Put the selected photos in a day of this trip"));
        }
        if create {
            match self.db.add_trip_day(trip_id, None, "") {
                Ok(day_id) => target = Some(Some(day_id)),
                Err(err) => self.activity = err,
            }
        }
        let Some(day_id) = target else { return };
        let photos = self.selection_in_order();
        let media = self.db.add_to_trip(trip_id, &photos).unwrap_or_default();
        match self.db.move_to_day(trip_id, day_id, &media, DaySource::User) {
            Ok(before) => {
                self.activity = trf("Moved {} photos", &[&before.len()]);
                self.day_undo = Some((trip_id, before));
            }
            Err(err) => self.activity = err,
        }
        self.reload();
    }

    /// Moves the selected photos within the Day being shown.
    pub(super) fn arrange_menu(&mut self, ui: &mut egui::Ui) {
        let (Nav::Trip(trip_id), DayFilter::Day(day_id)) = (&self.nav, self.trip_filter.day) else { return };
        let trip_id = *trip_id;
        let mut step = None;
        let button = ui.button(tr("Arrange"));
        let popup = egui::Popup::menu(&button).show(|ui| {
            for (label, choice) in [
                ("Move to start", Arrange::Start),
                ("Move earlier", Arrange::Earlier),
                ("Move later", Arrange::Later),
                ("Move to end", Arrange::End),
            ] {
                if ui.button(tr(label)).clicked() {
                    step = Some(choice);
                }
            }
        });
        if popup.is_none() {
            button.on_hover_text(tr("Change where the selected photos are in this day"));
        }
        let Some(step) = step else { return };
        let query = MemberQuery { day: DayFilter::Day(day_id), ..MemberQuery::default() };
        let members = self.db.trip_members(trip_id, &query, 0, usize::MAX);
        let order: Vec<i64> = members.iter().map(|member| member.media_id).collect();
        let selected: Vec<bool> = members.iter().map(|member| member.photo_id.is_some_and(|id| self.selection.contains(&id))).collect();
        if let Err(err) = self.db.order_day(day_id, &arrange(&order, &selected, step)) {
            self.activity = err;
        }
        self.reload();
    }

    fn undo_day_move(&mut self) {
        let Some((trip_id, before)) = self.day_undo.take() else { return };
        if let Err(err) = self.db.restore_day_assignments(trip_id, &before) {
            self.activity = err;
        }
        self.reload();
    }

    pub(super) fn day_modal(&mut self, ctx: &egui::Context) {
        let Modal::DayDetails(dialog) = &mut self.modal else { return };
        let mut close = false;
        let mut save = false;
        let response = egui::Modal::new(egui::Id::unique("trip-day")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(400.0);
            about::title_bar(ui, &self.logo, tr("Edit day"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(tr("Title"));
                ui.add(egui::TextEdit::singleline(&mut dialog.title).hint_text(tr("Optional")).desired_width(f32::INFINITY));
                ui.label(tr("Date"));
                date_field(ui, "day-date", &mut dialog.date, "2026-03-01");
                ui.label(tr("Notes"));
                ui.add(egui::TextEdit::multiline(&mut dialog.notes).desired_rows(3).desired_width(f32::INFINITY));
                if !dialog.error.is_empty() {
                    ui.label(RichText::new(&dialog.error).color(theme::DANGER));
                }
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(theme::primary_button(tr("Save"))).clicked() {
                        save = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if save {
            let Some(date) = parse_day(&dialog.date) else {
                dialog.error = tr("Type dates such as 2026-03-01").into();
                return;
            };
            match self.db.update_trip_day(dialog.day_id, date, &dialog.title, &dialog.notes) {
                Ok(()) => {
                    self.modal = Modal::None;
                    self.reload();
                }
                Err(err) => dialog.error = err,
            }
            return;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
        }
    }

    /// Asks for the file of an unavailable member and links it, if the
    /// library has that file. Returns why it couldn't, or nothing.
    fn locate_member(&mut self, media_id: i64) -> String {
        let Some(file) = rfd::FileDialog::new().set_title(tr("Locate the photo")).pick_file() else {
            return String::new();
        };
        let Some(photo_id) = self.db.photo_at_path(&file) else {
            return tr("That file isn't in the library. Add its folder to the library first, then locate it again.").into();
        };
        match self.db.relink_media_to(media_id, photo_id) {
            Ok(()) => {
                self.activity = tr("Photo linked again").into();
                self.reload();
                String::new()
            }
            Err(_) => tr("That file is already linked to another photo of a trip or story.").into(),
        }
    }

    pub(super) fn open_new_trip(&mut self, photos: Vec<i64>) {
        let preview = if photos.is_empty() { AddPreview::default() } else { self.db.preview_add_to_trip(0, &photos) };
        self.modal = Modal::Trip(TripDialog {
            editing: None,
            title: String::new(),
            zone: String::new(),
            trip_time: false,
            destination: String::new(),
            description: String::new(),
            first_day: String::new(),
            last_day: String::new(),
            photos,
            preview,
            error: String::new(),
        });
    }

    fn open_edit_trip(&mut self, trip_id: i64) {
        let Some(trip) = self.db.trip(trip_id) else { return };
        self.modal = Modal::Trip(TripDialog {
            editing: Some(trip.id),
            first_day: trip.first_day.map(|day| day.to_string()).unwrap_or_default(),
            last_day: trip.last_day.map(|day| day.to_string()).unwrap_or_default(),
            title: trip.title,
            zone: trip.zone,
            trip_time: trip.trip_time,
            destination: trip.destination,
            description: trip.description,
            photos: Vec::new(),
            preview: AddPreview::default(),
            error: String::new(),
        });
    }

    fn open_delete_trip(&mut self, trip_id: i64) {
        let Some(trip) = self.db.trip(trip_id) else { return };
        self.modal = Modal::DeleteTrip(DeleteDialog { trip_id, name: trip.title, impact: self.db.trip_impact(trip_id) });
    }

    pub(super) fn trip_modal(&mut self, ctx: &egui::Context) {
        let Modal::Trip(dialog) = &mut self.modal else { return };
        let mut close = false;
        let mut submit = false;
        let title = if dialog.editing.is_some() { tr("Edit trip") } else { tr("New trip") };
        let response = egui::Modal::new(egui::Id::unique("trip-details")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(460.0);
            about::title_bar(ui, &self.logo, title, &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(tr("Trip name"));
                let name = ui.add(egui::TextEdit::singleline(&mut dialog.title).desired_width(f32::INFINITY));
                if dialog.editing.is_none() && dialog.title.is_empty() && !name.has_focus() && ui.memory(|memory| memory.focused().is_none()) {
                    name.request_focus();
                }
                ui.add_space(4.0);
                ui.label(tr("Time zone"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut dialog.zone).hint_text("+08:00").desired_width(160.0));
                    let here = crate::geotag::local_offset();
                    if ui.button(trf("Use this computer's ({})", &[&here])).clicked() {
                        dialog.zone = here;
                    }
                });
                ui.label(
                    RichText::new(tr("Days are counted in this time zone: a name such as Asia/Taipei, or an offset such as +08:00."))
                        .size(12.0)
                        .color(theme::muted()),
                );
                ui.checkbox(&mut dialog.trip_time, tr("Cameras that record no time zone were set to this one"))
                    .on_hover_text(tr("Many cameras keep local time without saying which. Turn this on if you set their clocks to the trip's time; otherwise their photos wait for review."));
                ui.add_space(4.0);
                ui.label(tr("Destination"));
                ui.add(egui::TextEdit::singleline(&mut dialog.destination).hint_text(tr("Optional")).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(tr("First day"));
                        date_field(ui, "trip-first-day", &mut dialog.first_day, "2026-03-01");
                    });
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label(tr("Last day"));
                        // An empty last day opens the calendar at the first.
                        let start = parse_day(&dialog.first_day).flatten();
                        date_field_from(ui, "trip-last-day", &mut dialog.last_day, "2026-03-07", start);
                    });
                });
                ui.label(tr("Description"));
                ui.add(egui::TextEdit::multiline(&mut dialog.description).hint_text(tr("Optional")).desired_rows(3).desired_width(f32::INFINITY));
                if !dialog.photos.is_empty() {
                    ui.add_space(4.0);
                    preview_lines(ui, &dialog.preview);
                }
                if !dialog.error.is_empty() {
                    ui.label(RichText::new(&dialog.error).color(theme::DANGER));
                }
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if dialog.editing.is_some() { tr("Save") } else { tr("Create") };
                    if ui.add(theme::primary_button(label)).clicked() {
                        submit = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if submit {
            self.save_trip_dialog();
            return;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
        }
    }

    fn save_trip_dialog(&mut self) {
        let Modal::Trip(dialog) = &mut self.modal else { return };
        let (Some(first_day), Some(last_day)) = (parse_day(&dialog.first_day), parse_day(&dialog.last_day)) else {
            dialog.error = tr("Type dates such as 2026-03-01").into();
            return;
        };
        let draft = TripDraft {
            title: dialog.title.clone(),
            description: dialog.description.clone(),
            destination: dialog.destination.clone(),
            first_day,
            last_day,
            zone: dialog.zone.clone(),
            trip_time: dialog.trip_time,
        };
        let saved = match dialog.editing {
            Some(id) => self.db.update_trip(id, &draft).map(|_| id),
            None => self.db.create_trip(&draft).and_then(|id| {
                if !dialog.photos.is_empty() {
                    self.db.add_to_trip(id, &dialog.photos)?;
                }
                Ok(id)
            }),
        };
        match saved {
            Ok(id) => {
                let added = dialog.preview.photos + dialog.preview.videos;
                if dialog.editing.is_none() && added > 0 {
                    self.activity = trf("Added {} to trip", &[&added]);
                }
                self.modal = Modal::None;
                if self.nav != Nav::Trip(id) {
                    self.set_nav(Nav::Trip(id));
                }
                self.reload();
            }
            Err(err) => dialog.error = err,
        }
    }

    pub(super) fn add_to_trip_modal(&mut self, ctx: &egui::Context) {
        let Modal::AddToTrip(dialog) = &self.modal else { return };
        let mut close = false;
        let mut add = false;
        let response = egui::Modal::new(egui::Id::unique("add-to-trip")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(400.0);
            about::title_bar(ui, &self.logo, &trf("Add to “{}”", &[&dialog.name]), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                preview_lines(ui, &dialog.preview);
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(theme::primary_button(tr("Add"))).clicked() {
                        add = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if add {
            let Modal::AddToTrip(dialog) = std::mem::replace(&mut self.modal, Modal::None) else { return };
            match self.db.add_to_trip(dialog.trip_id, &dialog.photos) {
                Ok(_) => self.activity = trf("Added {} to trip", &[&(dialog.preview.photos + dialog.preview.videos)]),
                Err(err) => self.activity = err,
            }
            self.reload();
            return;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
        }
    }

    pub(super) fn delete_trip_modal(&mut self, ctx: &egui::Context) {
        let Modal::DeleteTrip(dialog) = &self.modal else { return };
        let mut close = false;
        let mut delete = false;
        let response = egui::Modal::new(egui::Id::unique("delete-trip")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(420.0);
            about::title_bar(ui, &self.logo, &trf("Delete “{}”?", &[&dialog.name]), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                let impact = &dialog.impact;
                ui.label(trf(
                    "Its {} days, {} stories, and {} linked tracks are deleted with it. Its {} photos stay in your library, and no file is changed.",
                    &[&impact.days, &impact.stories, &impact.tracks, &impact.members],
                ));
                ui.label(RichText::new(tr("To keep everything but hide the trip, archive it instead.")).color(theme::muted()));
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(theme::danger_button(tr("Delete trip"))).clicked() {
                        delete = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if delete {
            let Modal::DeleteTrip(dialog) = std::mem::replace(&mut self.modal, Modal::None) else { return };
            match self.db.delete_trip(dialog.trip_id) {
                Ok(()) => {
                    if self.nav == Nav::Trip(dialog.trip_id) {
                        self.set_nav(Nav::Library);
                    }
                }
                Err(err) => self.activity = err,
            }
            self.reload();
            return;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
        }
    }

    /// The name of the Trip on view, for the title.
    pub(super) fn trip_title(&self, trip_id: i64) -> String {
        self.trips.iter().find(|trip| trip.id == trip_id).map(|trip: &Trip| trip.title.clone()).unwrap_or_else(|| tr("Trip").into())
    }
}

/// What adding photos to a Trip will do.
fn preview_lines(ui: &mut egui::Ui, preview: &AddPreview) {
    ui.label(trf("{} photos and {} videos to add", &[&preview.photos, &preview.videos]));
    if let Some((first, last)) = preview.taken {
        let (first, last) = (format_day(first), format_day(last));
        let when = if first == last { first } else { format!("{first} – {last}") };
        ui.label(RichText::new(trf("Taken {}", &[&when])).color(theme::muted()));
    }
    if preview.already > 0 {
        ui.label(RichText::new(trf("Already in this trip: {}", &[&preview.already])).color(theme::muted()));
    }
}

/// A typed day: `Some(None)` when empty, `None` when it can't be read.
fn parse_day(text: &str) -> Option<Option<NaiveDate>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    NaiveDate::parse_from_str(&text.replace(['/', '.'], "-"), "%Y-%m-%d").ok().map(Some)
}

/// A Day's title, else its date, else its number in the Trip.
fn day_name(day: &TripDay) -> String {
    if !day.title.is_empty() {
        return day.title.clone();
    }
    match day.day {
        Some(date) => day_label(date),
        None => trf("Day {}", &[&(day.position + 1)]),
    }
}

/// A calendar day written the way the interface language writes dates.
/// How many members are to review for `reason`.
fn review_text(reason: Review, count: usize) -> String {
    let text = match reason {
        Review::NoOffset => "{} without a time zone",
        Review::NoTime => "{} without a capture time",
        Review::Ambiguous => "{} at a time that is unclear around a clock change",
        Review::Unread => "{} not read yet",
    };
    trf(text, &[&count])
}

#[derive(Clone, Copy)]
enum Arrange {
    Start,
    Earlier,
    Later,
    End,
}

/// `order` with its `selected` items moved by `step`, keeping the order
/// among the selected and among the others.
fn arrange(order: &[i64], selected: &[bool], step: Arrange) -> Vec<i64> {
    let pick = |want: bool| order.iter().zip(selected).filter(move |(_, &chosen)| chosen == want).map(|(&id, _)| id);
    let mut items: Vec<(i64, bool)> = order.iter().copied().zip(selected.iter().copied()).collect();
    match step {
        Arrange::Start => return pick(true).chain(pick(false)).collect(),
        Arrange::End => return pick(false).chain(pick(true)).collect(),
        Arrange::Earlier => {
            for i in 1..items.len() {
                if items[i].1 && !items[i - 1].1 {
                    items.swap(i, i - 1);
                }
            }
        }
        Arrange::Later => {
            for i in (1..items.len()).rev() {
                if items[i - 1].1 && !items[i].1 {
                    items.swap(i, i - 1);
                }
            }
        }
    }
    items.into_iter().map(|(id, _)| id).collect()
}

/// The Trip's dates as shown, if it has any.
pub(super) fn trip_dates(trip: &Trip) -> Option<String> {
    match (trip.first_day, trip.last_day) {
        (Some(first), Some(last)) if first != last => Some(format!("{} – {}", day_label(first), day_label(last))),
        (Some(day), _) | (None, Some(day)) => Some(day_label(day)),
        (None, None) => None,
    }
}

/// A date typed as text, with a calendar button beside it to pick one.
fn date_field(ui: &mut egui::Ui, id: &str, text: &mut String, hint: &str) {
    date_field_from(ui, id, text, hint, None);
}

/// [`date_field`], opening at `start` while no date is typed.
fn date_field_from(ui: &mut egui::Ui, id: &str, text: &mut String, hint: &str, start: Option<NaiveDate>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(96.0));
        let button = ui.add(egui::Button::new("    ")).on_hover_text(tr("Choose a date"));
        paint_calendar_icon(ui, button.rect);
        let popup = egui::Popup::menu(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let typed = parse_day(text).flatten();
            let shown_id = ui.make_persistent_id(("calendar-month", id));
            let today = Local::now().date_naive();
            let shown = ui.data(|data| data.get_temp::<NaiveDate>(shown_id)).or(typed).or(start).unwrap_or(today);
            if let Some(picked) = calendar(ui, shown_id, shown, typed, today) {
                *text = picked.map(|day| day.to_string()).unwrap_or_default();
                ui.data_mut(|data| data.remove::<NaiveDate>(shown_id));
                ui.close();
            }
        });
        if popup.is_none() {
            // Opening again starts from the date in the field.
            ui.data_mut(|data| data.remove::<NaiveDate>(ui.make_persistent_id(("calendar-month", id))));
        }
    });
}

/// A month of days to click, from Monday. Returns `Some(day)` for a
/// picked day, `Some(None)` for Clear. `shown` is any day of the month on view.
fn calendar(ui: &mut egui::Ui, shown_id: egui::Id, shown: NaiveDate, selected: Option<NaiveDate>, today: NaiveDate) -> Option<Option<NaiveDate>> {
    use chrono::{Datelike, Duration, Months};
    let first = shown.with_day(1).unwrap_or(shown);
    let mut picked = None;
    let mut month = first;
    // Seven day cells and the gaps between them; nothing in the popup may be
    // wider, or the menu stretches past the grid.
    const CELL: f32 = 28.0;
    const GAP: f32 = 2.0;
    let width = 7.0 * CELL + 6.0 * GAP;
    ui.set_width(width);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let arrow = Vec2::new(CELL, 22.0);
        if ui.add_sized(arrow, egui::Button::new("‹")).on_hover_text(tr("Previous month")).clicked() {
            month = first.checked_sub_months(Months::new(1)).unwrap_or(first);
        }
        let title = RichText::new(crate::types::format_month(first.year(), first.month())).strong();
        ui.add_sized(Vec2::new(width - 2.0 * CELL, 22.0), egui::Label::new(title));
        if ui.add_sized(arrow, egui::Button::new("›")).on_hover_text(tr("Next month")).clicked() {
            month = first.checked_add_months(Months::new(1)).unwrap_or(first);
        }
    });
    // Without these a column is as wide as a button, wider than a day.
    egui::Grid::new(("calendar", shown_id)).spacing([GAP, GAP]).min_col_width(CELL).max_col_width(CELL).show(ui, |ui| {
        for weekday in 0..7 {
            let name = RichText::new(crate::types::short_weekday(weekday)).size(11.0).color(theme::muted());
            ui.add_sized([CELL, 16.0], egui::Label::new(name));
        }
        ui.end_row();
        let lead = first.weekday().num_days_from_monday() as i64;
        let mut day = first - Duration::days(lead);
        for _ in 0..6 {
            for _ in 0..7 {
                let in_month = day.month() == first.month();
                let mut label = RichText::new(day.day().to_string()).size(12.5);
                if !in_month {
                    label = label.color(theme::muted().gamma_multiply(0.6));
                }
                if day == today {
                    label = label.strong().underline();
                }
                let cell = ui.add_sized([CELL, 24.0], egui::Button::selectable(selected == Some(day), label));
                if cell.clicked() {
                    picked = Some(Some(day));
                }
                day += Duration::days(1);
            }
            ui.end_row();
            if day.month() != first.month() {
                break;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.set_width(width);
        if ui.link(tr("Today")).clicked() {
            picked = Some(Some(today));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.link(tr("Clear")).clicked() {
                picked = Some(None);
            }
        });
    });
    if month != first {
        ui.data_mut(|data| data.insert_temp(shown_id, month));
    }
    picked
}

/// A small calendar page drawn in `rect`: a frame, a top band, and dots for days.
fn paint_calendar_icon(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    let color = theme::text();
    let page = egui::Rect::from_center_size(rect.center(), egui::vec2(12.0, 11.0));
    painter.rect_stroke(page, 1.5, egui::Stroke::new(1.2, color), egui::StrokeKind::Inside);
    painter.rect_filled(egui::Rect::from_min_max(page.min, egui::pos2(page.max.x, page.min.y + 3.0)), 1.0, color);
    for row in 0..2 {
        for col in 0..3 {
            let at = egui::pos2(page.min.x + 3.0 + col as f32 * 3.0, page.min.y + 5.5 + row as f32 * 2.8);
            painter.circle_filled(at, 0.7, color);
        }
    }
}

fn day_label(day: NaiveDate) -> String {
    // Noon is on the same day in every time zone the computer may be in.
    let noon = day.and_hms_opt(12, 0, 0).and_then(|time| Local.from_local_datetime(&time).earliest());
    noon.map_or_else(|| day.to_string(), |time| format_day(time.timestamp()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_days_are_read_with_dashes_slashes_or_dots() {
        let day = NaiveDate::from_ymd_opt(2026, 3, 1);
        assert_eq!(parse_day("2026-03-01"), Some(day));
        assert_eq!(parse_day(" 2026/3/1 "), Some(day));
        assert_eq!(parse_day("2026.03.01"), Some(day));
        assert_eq!(parse_day(""), Some(None), "a day is optional");
        assert_eq!(parse_day("March 1"), None);
        assert_eq!(parse_day("2026-02-30"), None);
    }

    #[test]
    fn selected_photos_move_together_and_keep_their_order() {
        let order = [1, 2, 3, 4, 5];
        let selected = [false, true, false, true, false];
        assert_eq!(arrange(&order, &selected, Arrange::Start), [2, 4, 1, 3, 5]);
        assert_eq!(arrange(&order, &selected, Arrange::End), [1, 3, 5, 2, 4]);
        assert_eq!(arrange(&order, &selected, Arrange::Earlier), [2, 1, 4, 3, 5]);
        assert_eq!(arrange(&order, &selected, Arrange::Later), [1, 3, 2, 5, 4]);
        let first = [true, false, false, false, false];
        assert_eq!(arrange(&order, &first, Arrange::Earlier), order, "the first stays first");
    }
}
