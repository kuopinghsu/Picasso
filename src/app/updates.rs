//! The once-a-day check for a newer Picasso release, the note in the header
//! when there is one, and its switch in Settings.

use crossbeam_channel::Receiver;
use eframe::egui::{self, RichText};

use crate::i18n::{tr, trf};
use crate::types::unix_now;
use crate::update::{self, Release};

use super::{theme, PicassoApp};

type Outcome = Result<Option<Release>, String>;

#[derive(Default)]
pub(super) struct UpdateState {
    /// The latest release, when it is newer than this build.
    available: Option<Release>,
    checking: Option<Receiver<Outcome>>,
    /// Whether the running check was asked for in Settings, which reports
    /// every outcome; the daily check stays quiet unless there is news.
    manual: bool,
    /// What the latest check from Settings found.
    note: String,
    /// The daily check has been looked at this session.
    started: bool,
}

impl PicassoApp {
    /// Picks up the release found by an earlier check.
    pub(super) fn load_update(&mut self) {
        let version = self.config.text("update_version").unwrap_or_default().to_string();
        let url = self.config.text("update_url").unwrap_or_default().to_string();
        if !version.is_empty() && update::is_newer(&version) {
            self.updates.available = Some(Release { version, url });
        }
    }

    fn checks_updates(&self) -> bool {
        self.config.flag("check_updates").unwrap_or(true)
    }

    fn check_for_update(&mut self, ctx: &egui::Context, manual: bool) {
        if self.updates.checking.is_some() {
            return;
        }
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("picasso-update".into()).spawn(move || {
            let _ = sender.send(update::latest());
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.updates.checking = Some(receiver);
            self.updates.manual = manual;
        }
    }

    /// Starts the daily check when it is due, and takes in its outcome.
    pub(super) fn pump_update(&mut self, ctx: &egui::Context) {
        if !self.updates.started {
            self.updates.started = true;
            let checked = self.config.text("update_checked_at").and_then(|at| at.parse::<i64>().ok()).unwrap_or(0);
            if self.checks_updates() && unix_now() - checked >= update::EVERY {
                self.check_for_update(ctx, false);
            }
        }
        let Some(outcome) = self.updates.checking.as_ref().and_then(|checking| checking.try_recv().ok()) else {
            return;
        };
        self.updates.checking = None;
        let release = match outcome {
            Ok(release) => release.filter(|release| update::is_newer(&release.version)),
            Err(err) => {
                if self.updates.manual {
                    self.updates.note = trf("Couldn't check for updates: {}", &[&err]);
                }
                return;
            }
        };
        self.config.set("update_checked_at", unix_now().to_string());
        let (version, url) = release.as_ref().map_or_else(Default::default, |release| (release.version.clone(), release.url.clone()));
        self.config.set("update_version", version);
        self.config.set("update_url", url);
        if self.updates.manual {
            self.updates.note = match &release {
                Some(release) => trf("Picasso {} is available", &[&release.version]),
                None => trf("Picasso {} is up to date", &[&env!("CARGO_PKG_VERSION")]),
            };
        }
        self.updates.available = release;
    }

    /// A link to the release page in the header, while there is a newer one.
    pub(super) fn update_link(&self, ui: &mut egui::Ui) {
        let Some(release) = &self.updates.available else { return };
        let text = RichText::new(trf("Picasso {} is available", &[&release.version])).color(theme::ACCENT);
        if ui.add(egui::Button::new(text).frame(false)).on_hover_text(tr("Open the download page")).clicked() {
            let _ = open::that(&release.url);
        }
    }

    pub(super) fn update_settings(&mut self, ui: &mut egui::Ui) {
        let mut on = self.checks_updates();
        if ui.checkbox(&mut on, tr("Check for a newer version once a day")).changed() {
            self.config.set("check_updates", on);
        }
        ui.horizontal(|ui| {
            let checking = self.updates.checking.is_some();
            if ui.add_enabled(!checking, egui::Button::new(tr("Check now"))).clicked() {
                self.updates.note.clear();
                self.check_for_update(ui.ctx(), true);
            }
            if checking {
                ui.spinner();
            } else if let Some(release) = self.updates.available.clone().filter(|_| !self.updates.note.is_empty()) {
                if ui.link(&self.updates.note).clicked() {
                    let _ = open::that(&release.url);
                }
            } else if !self.updates.note.is_empty() {
                ui.label(RichText::new(&self.updates.note).color(theme::muted()));
            }
        });
        ui.label(
            RichText::new(tr("Picasso asks GitHub for the latest version number. Nothing is downloaded or sent."))
                .size(12.0)
                .color(theme::muted()),
        );
    }
}
