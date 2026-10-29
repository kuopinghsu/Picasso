//! Exporting a Trip as a Trip package and importing one
//! (`docs/TRAVEL.md`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, RichText};

use crate::i18n::{tr, trf};
use crate::story_html;
use crate::trip::MemberQuery;
use crate::trip_package::{self, PackDay, PackPhoto, PackStory, PackTrack, PackTrip, Package};
use crate::types::Nav;
use crate::worker::Unpacked;

use super::{theme, PicassoApp, UiEvent};

/// A Trip package being written or read, with its progress and Cancel.
pub(super) struct TripJob {
    importing: bool,
    progress: Arc<story_html::Progress>,
}

impl PicassoApp {
    /// Asks where to save a Trip package of Trip `trip_id`, then writes it
    /// on its own thread: every photo of the Trip in its Day, and every
    /// Story.
    pub(super) fn export_trip_package(&mut self, ctx: &egui::Context, trip_id: i64) {
        if self.trip_job.is_some() {
            return;
        }
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
            photos.push(PackPhoto { key: member.media_id, name: String::new(), day, size: None, member: true, hash: None });
            let photo = member.photo_id.and_then(|id| self.db.photo(id));
            sources.insert(member.media_id, photo.map(|photo| (PathBuf::from(&photo.path), self.edit_settings(photo.id), photo.rotation)));
        }
        let stories: Vec<PackStory> =
            self.db.stories(trip_id).into_iter().map(|story| PackStory { blocks: self.db.story_blocks(story.id), story }).collect();
        // Stories keep photos taken out of the Trip; those travel too.
        for item in &stories {
            let used = item.story.cover.into_iter().chain(item.blocks.iter().flat_map(|block| block.block.media().iter().map(|media| media.media_id)));
            for media_id in used {
                if sources.contains_key(&media_id) {
                    continue;
                }
                photos.push(PackPhoto { key: media_id, name: String::new(), day: None, size: None, member: false, hash: None });
                let photo = self.db.media_photo(media_id).and_then(|id| self.db.photo(id));
                sources.insert(media_id, photo.map(|photo| (PathBuf::from(&photo.path), self.edit_settings(photo.id), photo.rotation)));
            }
        }
        // The tracks linked to the Trip, those whose files are here.
        let tracks = self
            .db
            .trip_tracks(trip_id)
            .into_iter()
            .filter(|link| link.track_id.is_some())
            .map(|link| PackTrack {
                file: Path::new(&link.path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
                day: link.day_id.and_then(|day_id| days.iter().position(|day| day.id == day_id)),
                path: Some(PathBuf::from(&link.path)),
                name: link.name,
                ..PackTrack::default()
            })
            .collect();
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
            tracks,
        };
        let progress = self.start_trip_job(false);
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-trip-package".into()).spawn(move || {
            let outcome = trip_package::write_package(&path, &package, &sources, &progress).map(|missing| (path, missing));
            let _ = events.send(UiEvent::TripPackaged(outcome));
            ctx.request_repaint();
        });
    }

    pub(super) fn on_trip_packaged(&mut self, outcome: Result<(PathBuf, usize), String>) {
        self.trip_job = None;
        self.activity = match outcome {
            Err(err) if err == story_html::CANCELLED => tr("Export cancelled. Nothing was written.").into(),
            Ok((path, 0)) => trf("Saved the trip package {}", &[&path.display()]),
            Ok((path, missing)) => trf("Saved the trip package {}. {} unavailable photos stay unavailable in it.", &[&path.display(), &missing]),
            Err(err) => trf("Couldn't save the trip package: {}", &[&err]),
        };
    }

    /// Asks for a Trip package and a folder for its photos, then writes the
    /// photos there on its own thread; the import itself follows in
    /// [`Self::on_trip_unpacked`].
    pub(super) fn import_trip_package(&mut self, ctx: &egui::Context) {
        if self.trip_job.is_some() {
            return;
        }
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
        let progress = self.start_trip_job(true);
        let db_path = self.data_dir.join("picasso.db");
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-trip-import".into()).spawn(move || {
            // The folder is new, so a cancelled or failed import takes it away
            // again with everything written into it.
            let mut package = package;
            let existing = match in_library(&db_path, &mut package, &progress) {
                Ok(existing) => existing,
                Err(err) => {
                    let _ = events.send(UiEvent::TripUnpacked(Box::new(Err(err))));
                    ctx.request_repaint();
                    return;
                }
            };
            let skip = existing.keys().copied().collect();
            let outcome = std::fs::create_dir_all(&folder).and_then(|()| folder.canonicalize()).map_err(|err| err.to_string()).and_then(|folder| {
                match trip_package::extract_photos(&path, &mut package, start, &folder, &skip, &progress) {
                    Ok(files) => Ok((package, files, existing, folder)),
                    Err(err) => {
                        let _ = std::fs::remove_dir_all(&folder);
                        Err(err)
                    }
                }
            });
            let _ = events.send(UiEvent::TripUnpacked(Box::new(outcome)));
            ctx.request_repaint();
        });
    }

    /// Adds an unpacked Trip package to the library, makes sure its photos'
    /// folder is scanned so they link, and opens the Trip.
    pub(super) fn on_trip_unpacked(&mut self, outcome: Result<Unpacked, String>) {
        self.trip_job = None;
        let (package, files, existing, folder) = match outcome {
            Ok(unpacked) => unpacked,
            Err(err) if err == story_html::CANCELLED => {
                self.activity = tr("Import cancelled. Nothing was added.").into();
                return;
            }
            Err(err) => {
                self.activity = trf("Couldn't import the trip package: {}", &[&err]);
                return;
            }
        };
        let trip_id = match self.db.import_trip(&package, &files, &existing) {
            Ok(id) => id,
            Err(err) => {
                // Nothing was added, so the new folder of photos goes too.
                let _ = std::fs::remove_dir_all(&folder);
                self.activity = trf("Couldn't import the trip package: {}", &[&err]);
                return;
            }
        };
        self.trips = self.db.trips(&crate::trip::TripQuery::default());
        self.set_nav(Nav::Trip(trip_id));
        let new_tracks = package.tracks.iter().any(|track| track.path.as_ref().is_some_and(|path| path.starts_with(&folder)));
        if files.is_empty() && !new_tracks {
            // Every photo and track was in the library already; the folder stayed empty.
            let _ = std::fs::remove_dir(&folder);
            self.activity = trf("Imported “{}”. All its photos were in the library already.", &[&package.trip.title]);
            return;
        }
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
        self.activity = if existing.is_empty() {
            trf("Imported “{}”. Its photos are in {}.", &[&package.trip.title, &folder.display()])
        } else {
            trf("Imported “{}”. {} photos were in the library already; the others are in {}.", &[&package.trip.title, &existing.len(), &folder.display()])
        };
    }

    fn start_trip_job(&mut self, importing: bool) -> Arc<story_html::Progress> {
        let progress = Arc::new(story_html::Progress::default());
        self.trip_job = Some(TripJob { importing, progress: Arc::clone(&progress) });
        progress
    }

    /// The running Trip package job in the status bar: how far it is, a
    /// progress bar, and Cancel.
    pub(super) fn trip_job_status(&mut self, ui: &mut egui::Ui) {
        let Some(job) = &self.trip_job else { return };
        let small = |text: String| RichText::new(text).size(12.0);
        let (done, total) = (job.progress.done.load(Ordering::Relaxed), job.progress.total.load(Ordering::Relaxed));
        ui.label(small("·".into()).color(theme::muted()));
        let note = if job.importing { trf("Importing photos: {} of {}", &[&done, &total]) } else { trf("Packing photos: {} of {}", &[&done, &total]) };
        ui.label(small(note).color(theme::muted()));
        let fraction = if total == 0 { 0.0 } else { done as f32 / total as f32 };
        ui.add(egui::ProgressBar::new(fraction).desired_width(120.0).desired_height(8.0));
        if job.progress.cancel.load(Ordering::Relaxed) {
            ui.label(small(tr("Cancelling…").into()).color(theme::muted()));
        } else if ui.link(small(tr("Cancel").into())).clicked() {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}

/// Photos of the package that are in the library already, as the library
/// photo standing for each by key: a file of the same size and SHA-256.
/// Only files of a carried photo's exact size are read. Tracks found the
/// same way get the path of the library's file.
fn in_library(db_path: &Path, package: &mut Package, progress: &story_html::Progress) -> Result<HashMap<i64, i64>, String> {
    let db = crate::db::Database::open_reader(db_path)?;
    let library_tracks = db.track_paths();
    for track in &mut package.tracks {
        let same = library_tracks.iter().find(|path| {
            std::fs::metadata(path).is_ok_and(|meta| meta.len() == track.size) && trip_package::file_hash(Path::new(path)).as_deref() == Some(track.hash.as_str())
        });
        track.path = same.map(PathBuf::from);
    }
    let mut hashes: HashMap<String, Option<String>> = HashMap::new();
    let mut found = HashMap::new();
    let carried: Vec<_> = package.photos.iter().filter_map(|photo| Some((photo.key, photo.size?, photo.hash.as_deref()?))).collect();
    progress.total.store(carried.len(), Ordering::Relaxed);
    for (key, size, hash) in carried {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(story_html::CANCELLED.into());
        }
        progress.done.fetch_add(1, Ordering::Relaxed);
        let same = db.photos_with_bytes(size).into_iter().find(|(_, path)| {
            hashes.entry(path.clone()).or_insert_with(|| trip_package::file_hash(Path::new(path))).as_deref() == Some(hash)
        });
        if let Some((photo_id, _)) = same {
            found.insert(key, photo_id);
        }
    }
    progress.done.store(0, Ordering::Relaxed);
    Ok(found)
}
