//! Exporting a Trip as a Trip package and importing one
//! (`docs/TRAVEL.md`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::i18n::{tr, trf};
use crate::story_html;
use crate::trip::MemberQuery;
use crate::trip_package::{self, PackDay, PackPhoto, PackStory, PackTrip, Package};
use crate::types::Nav;
use crate::worker::Unpacked;

use super::{PicassoApp, UiEvent};

impl PicassoApp {
    /// Asks where to save a Trip package of Trip `trip_id`, then writes it
    /// on its own thread: every photo of the Trip in its Day, and every
    /// Story.
    pub(super) fn export_trip_package(&mut self, ctx: &egui::Context, trip_id: i64) {
        let Some(trip) = self.db.trip(trip_id) else {
            return;
        };
        // The open Story's edits are part of what is carried.
        if !self.save_story() {
            return;
        }
        let name = format!("{}.{}", story_html::folder_name(&trip.title), trip_package::EXTENSION);
        let mut picker =
            rfd::FileDialog::new().set_title(tr("Save the trip package")).set_file_name(&name).add_filter(tr("Trip package"), &[trip_package::EXTENSION]);
        if let Some(folder) = self.config.text("trip_package_folder").filter(|folder| Path::new(folder).is_dir()) {
            picker = picker.set_directory(folder);
        }
        let Some(mut path) = picker.save_file() else {
            return;
        };
        if path.extension().is_none_or(|ext| ext != trip_package::EXTENSION) {
            path.set_extension(trip_package::EXTENSION);
        }
        if let Some(folder) = path.parent() {
            self.config.set("trip_package_folder", folder.display().to_string());
        }
        let days = self.db.trip_days(trip_id);
        let members = self.db.trip_members(trip_id, &MemberQuery::default(), 0, usize::MAX >> 1);
        let mut photos = Vec::new();
        let mut sources = HashMap::new();
        for member in &members {
            let day = member.day_id.and_then(|day_id| days.iter().position(|day| day.id == day_id));
            photos.push(PackPhoto { key: member.media_id, name: String::new(), day, size: None });
            let photo = member.photo_id.and_then(|id| self.db.photo(id));
            sources.insert(member.media_id, photo.map(|photo| (PathBuf::from(&photo.path), self.edit_settings(photo.id), photo.rotation)));
        }
        let stories = self.db.stories(trip_id).into_iter().map(|story| PackStory { blocks: self.db.story_blocks(story.id), story }).collect();
        let package = Package {
            trip: PackTrip {
                title: trip.title.clone(),
                description: trip.description.clone(),
                destination: trip.destination.clone(),
                first_day: trip.first_day,
                last_day: trip.last_day,
                zone: trip.zone.clone(),
                trip_time: trip.trip_time,
            },
            days: days.iter().map(|day| PackDay { day: day.day, title: day.title.clone(), notes: day.notes.clone() }).collect(),
            stories,
            photos,
        };
        self.activity = trf("Saving the trip package of “{}”…", &[&trip.title]);
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-trip-package".into()).spawn(move || {
            let outcome = trip_package::write_package(&path, &package, &sources, &story_html::Progress::default()).map(|missing| (path, missing));
            let _ = events.send(UiEvent::TripPackaged(outcome));
            ctx.request_repaint();
        });
    }

    pub(super) fn on_trip_packaged(&mut self, outcome: Result<(PathBuf, usize), String>) {
        self.activity = match outcome {
            Ok((path, 0)) => trf("Saved the trip package {}", &[&path.display()]),
            Ok((path, missing)) => trf("Saved the trip package {}. {} unavailable photos stay unavailable in it.", &[&path.display(), &missing]),
            Err(err) => trf("Couldn't save the trip package: {}", &[&err]),
        };
    }

    /// Asks for a Trip package and a folder for its photos, then writes the
    /// photos there on its own thread; the import itself follows in
    /// [`Self::on_trip_unpacked`].
    pub(super) fn import_trip_package(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new().set_title(tr("Import a trip package")).add_filter(tr("Trip package"), &[trip_package::EXTENSION]).pick_file()
        else {
            return;
        };
        let (package, start) = match trip_package::read_package(&path) {
            Ok(read) => read,
            Err(err) => {
                self.activity = err;
                return;
            }
        };
        let mut picker = rfd::FileDialog::new().set_title(tr("Choose where to put the trip's photos"));
        if let Some(folder) = self.config.text("trip_import_folder").filter(|folder| Path::new(folder).is_dir()) {
            picker = picker.set_directory(folder);
        } else if let Some(pictures) = directories::UserDirs::new().and_then(|dirs| dirs.picture_dir().map(Path::to_path_buf)) {
            picker = picker.set_directory(pictures);
        }
        let Some(parent) = picker.pick_folder() else {
            return;
        };
        self.config.set("trip_import_folder", parent.display().to_string());
        // The photos go into a new folder named after the Trip, never among
        // files already there.
        let base = story_html::folder_name(&package.trip.title);
        let folder = (1..)
            .map(|n| if n == 1 { parent.join(&base) } else { parent.join(format!("{base} {n}")) })
            .find(|folder| !folder.exists())
            .unwrap_or_else(|| parent.join(&base));
        self.activity = trf("Importing “{}”…", &[&package.trip.title]);
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-trip-import".into()).spawn(move || {
            let outcome = std::fs::create_dir_all(&folder).and_then(|()| folder.canonicalize()).map_err(|err| err.to_string()).and_then(|folder| {
                let files = trip_package::extract_photos(&path, &package, start, &folder, &story_html::Progress::default())?;
                Ok((package, files, folder))
            });
            let _ = events.send(UiEvent::TripUnpacked(Box::new(outcome)));
            ctx.request_repaint();
        });
    }

    /// Adds an unpacked Trip package to the library, makes sure its photos'
    /// folder is scanned so they link, and opens the Trip.
    pub(super) fn on_trip_unpacked(&mut self, outcome: Result<Unpacked, String>) {
        let (package, files, folder) = match outcome {
            Ok(unpacked) => unpacked,
            Err(err) => {
                self.activity = trf("Couldn't import the trip package: {}", &[&err]);
                return;
            }
        };
        let trip_id = match self.db.import_trip(&package, &files) {
            Ok(id) => id,
            Err(err) => {
                self.activity = trf("Couldn't import the trip package: {}", &[&err]);
                return;
            }
        };
        let scan = match self.folder_for(&folder) {
            Some(id) => Ok(id),
            None => self.db.add_folder(&folder),
        };
        match scan {
            Ok(id) => {
                let _ = self.cmd_tx.send(super::Command::Scan(id));
                self.folders = self.db.folders();
                self.sync_watches();
            }
            Err(err) => {
                self.activity = err;
                return;
            }
        }
        self.trips = self.db.trips(&crate::trip::TripQuery::default());
        self.set_nav(Nav::Trip(trip_id));
        self.activity = trf("Imported “{}”. Its photos are in {}.", &[&package.trip.title, &folder.display()]);
    }
}
