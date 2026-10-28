//! Reads the library on a background thread with its own connection, so the
//! window keeps drawing while a large library loads or a scan streams in.

use std::path::PathBuf;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;

use crate::db::Database;
use crate::trip::{MemberQuery, Trip, TripQuery};
use crate::types::{Album, Folder, Nav, Photo, Sort};

use super::tree::{self, FolderRoot};

pub(super) struct LoadJob {
    pub seq: u64,
    /// Also read the folders, their tree, the albums, and the trips.
    pub full: bool,
    pub nav: Nav,
    pub search: String,
    pub sort: Sort,
    /// Which of a Trip's photos to show.
    pub trip_filter: MemberQuery,
}

pub(super) struct Lists {
    pub folders: Vec<Folder>,
    pub tree: Vec<FolderRoot>,
    pub albums: Vec<Album>,
    /// Active trips, then archived ones.
    pub trips: Vec<Trip>,
}

pub(super) struct Loaded {
    pub seq: u64,
    pub lists: Option<Lists>,
    pub photos: Vec<Photo>,
}

pub(super) fn spawn(db_path: PathBuf, ctx: egui::Context) -> (Sender<LoadJob>, Receiver<Loaded>) {
    let (job_tx, job_rx) = crossbeam_channel::unbounded();
    let (done_tx, done_rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("picasso-load".into())
        .spawn(move || match Database::open_reader(&db_path) {
            Ok(db) => run(&db, &job_rx, &done_tx, &ctx),
            Err(err) => eprintln!("picasso load: {err}"),
        })
        .expect("load thread");
    (job_tx, done_rx)
}

fn run(db: &Database, jobs: &Receiver<LoadJob>, done: &Sender<Loaded>, ctx: &egui::Context) {
    while let Ok(mut job) = jobs.recv() {
        // Only the newest request is answered; one it replaces may still
        // have asked for the side bar lists.
        while let Ok(next) = jobs.try_recv() {
            let full = job.full || next.full;
            job = next;
            job.full = full;
        }
        let lists = job.full.then(|| {
            let folders = db.folders();
            let tree = tree::build(&folders, &db.photo_dirs());
            let mut trips = db.trips(&TripQuery::default());
            trips.extend(db.trips(&TripQuery { archived: true, ..TripQuery::default() }));
            Lists { folders, tree, albums: db.albums(), trips }
        });
        let photos = match job.nav {
            Nav::Trip(id) if job.search.trim().is_empty() => db.trip_photos(id, &job.trip_filter, job.sort),
            _ => db.photos(&job.nav, &job.search, job.sort),
        };
        if done.send(Loaded { seq: job.seq, lists, photos }).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}
