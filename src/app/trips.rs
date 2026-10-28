//! Trips in the window: their section of the side bar, adding the selected
//! photos to one, the dialogs that create, change, and delete a Trip, and
//! the line about a Trip above its photos. `docs/TRAVEL.md` describes the
//! model; the library operations are in `src/db/trips.rs`.

use chrono::{Local, NaiveDate, TimeZone};
use eframe::egui::{self, Align, Layout, Margin, RichText};

use crate::i18n::{tr, trf};
use crate::trip::{AddPreview, MemberQuery, Trip, TripDraft, TripImpact};
use crate::types::{format_day, Nav};

use super::{about, theme, tree_section, Glyph, Modal, PicassoApp};

/// Creating a Trip, from the selected photos or empty, or changing one.
pub(super) struct TripDialog {
    /// The Trip being changed; `None` for a new one.
    editing: Option<i64>,
    title: String,
    zone: String,
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

    /// The toolbar menu that adds the selected photos to a Trip, or starts
    /// a new one with them.
    pub(super) fn trip_menu(&mut self, ui: &mut egui::Ui) {
        let trips: Vec<(i64, String)> = self.trips.iter().filter(|trip| !trip.archived).map(|trip| (trip.id, trip.title.clone())).collect();
        let mut create = false;
        let mut add: Option<(i64, String)> = None;
        let button = ui.button(tr("Add to Trip"));
        let popup = egui::Popup::menu(&button).show(|ui| {
            for (id, name) in &trips {
                if ui.selectable_label(false, name).clicked() {
                    add = Some((*id, name.clone()));
                }
            }
            if !trips.is_empty() {
                ui.separator();
            }
            if ui.button(tr("New trip…")).clicked() {
                create = true;
            }
        });
        if popup.is_none() {
            button.on_hover_text(tr("Add the selected photos to a trip"));
        }
        let photos = self.selection_in_order();
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
        let mut parts = Vec::new();
        match (trip.first_day, trip.last_day) {
            (Some(first), Some(last)) if first != last => parts.push(format!("{} – {}", day_label(first), day_label(last))),
            (Some(day), _) | (None, Some(day)) => parts.push(day_label(day)),
            (None, None) => {}
        }
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
        if filter != self.trip_filter {
            self.trip_filter = filter;
            self.selection.clear();
            self.reload_view();
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
                ui.add_space(4.0);
                ui.label(tr("Destination"));
                ui.add(egui::TextEdit::singleline(&mut dialog.destination).hint_text(tr("Optional")).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(tr("First day"));
                        ui.add(egui::TextEdit::singleline(&mut dialog.first_day).hint_text("2026-03-01").desired_width(120.0));
                    });
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label(tr("Last day"));
                        ui.add(egui::TextEdit::singleline(&mut dialog.last_day).hint_text("2026-03-07").desired_width(120.0));
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

/// A calendar day written the way the interface language writes dates.
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
}
