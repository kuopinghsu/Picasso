//! Shown instead of the library when `picasso.db` can't be opened: damaged,
//! locked by another program, or unreadable. Nothing is changed until the
//! user picks a way out, and the library that failed is always kept. Only a
//! library that fails its integrity check is offered a replacement: one that
//! is merely busy may be open in another Picasso, which would go on writing
//! to it under its new name.
//!
//! A thread runs the check, so the window shows at once.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui::{self, Margin, RichText, Vec2};

use crate::backup::{self, Backup};
use crate::i18n::{self, tr, trf, Lang};

use super::{about, settings::local_time, theme, theme::ThemeMode};

pub(super) struct Recovery {
    data_dir: PathBuf,
    error: String,
    /// Arrives from a thread, as checking a large library takes a while.
    findings: Option<Findings>,
    findings_rx: Receiver<Findings>,
    /// A backup or a new library took its place; only a restart is left.
    replaced: bool,
    logo: egui::TextureHandle,
    /// Why the latest choice didn't work.
    note: String,
}

struct Findings {
    /// The library file fails its integrity check, so it may be replaced.
    damaged: bool,
    /// Why the library file can't be read, so it can't be checked: no
    /// permission, or on Windows another program holding it.
    unreadable: Option<std::io::ErrorKind>,
    /// The newest backup that passes the check.
    backup: Option<Backup>,
}

impl Findings {
    fn of(data_dir: &Path) -> Self {
        let library = data_dir.join("picasso.db");
        let unreadable = if library.is_file() { std::fs::File::open(&library).err().map(|err| err.kind()) } else { None };
        // A check that waited in vain for a lock, or couldn't read the file,
        // says nothing about damage.
        let damaged = library.is_file() && unreadable.is_none() && backup::check(&library).is_err_and(|err| !busy(&err));
        let backup = if damaged { backup::list(data_dir).into_iter().find(|backup| backup::check(&backup.path).is_ok()) } else { None };
        Findings { damaged, unreadable, backup }
    }
}

/// True for SQLite's errors about another connection holding the file.
fn busy(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("locked") || error.contains("busy")
}

impl Recovery {
    pub(super) fn new(ctx: &egui::Context, data_dir: PathBuf, error: String) -> Self {
        eprintln!("picasso library: {error}");
        let config = crate::db::config_dir();
        let stored = |key| crate::config::stored_text(&config, key);
        let mode = ThemeMode::from_code(&stored("theme").unwrap_or_default());
        theme::apply(ctx, mode.is_dark(super::system_dark(ctx)));
        i18n::set(stored("language").as_deref().and_then(Lang::from_code).unwrap_or_else(i18n::system));
        let (findings_tx, findings_rx) = mpsc::channel();
        let (dir, repaint) = (data_dir.clone(), ctx.clone());
        let _ = std::thread::Builder::new().name("picasso-recovery".into()).spawn(move || {
            let _ = findings_tx.send(Findings::of(&dir));
            repaint.request_repaint();
        });
        Self { data_dir, error, findings: None, findings_rx, replaced: false, logo: theme::load_logo(ctx), note: String::new() }
    }

    /// The findings once the thread has them. A thread that died found
    /// nothing, so nothing is offered for replacement.
    fn findings(&mut self) -> Option<&Findings> {
        if self.findings.is_none() {
            match self.findings_rx.try_recv() {
                Ok(findings) => self.findings = Some(findings),
                Err(TryRecvError::Disconnected) => self.findings = Some(Findings { damaged: false, unreadable: None, backup: None }),
                Err(TryRecvError::Empty) => {}
            }
        }
        self.findings.as_ref()
    }

    /// Restarts Picasso, which opens the library again.
    fn restart(&mut self, ctx: &egui::Context) {
        match backup::relaunch() {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(_) => self.note = tr("Start Picasso again to finish.").to_string(),
        }
    }

    fn replace(&mut self, ctx: &egui::Context, backup: Option<PathBuf>) {
        match backup::replace_damaged(&self.data_dir, backup.as_deref()) {
            Ok(_) => {
                self.replaced = true;
                self.restart(ctx);
            }
            Err(err) => self.note = trf("That didn't work: {}", &[&err]),
        }
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.label(RichText::new(tr("Picasso can't open its library")).size(20.0).strong().color(theme::text()));
        ui.add_space(6.0);
        note(ui, tr("Your photos are safe in their folders. The library holds what Picasso adds to them: stars, captions, albums, places, and edits."));
        ui.add_space(10.0);
        let library = self.data_dir.join("picasso.db");
        ui.label(RichText::new(library.display().to_string()).size(12.5).color(theme::text()));
        ui.label(RichText::new(&self.error).monospace().size(12.0).color(theme::DANGER));
        let unreadable = self.findings().and_then(|found| found.unreadable);
        let findings = self.findings().map(|found| (found.damaged, found.backup.as_ref().map(|backup| (backup.path.clone(), backup.made_at))));
        let damaged = findings.as_ref().is_some_and(|(damaged, _)| *damaged);
        ui.add_space(4.0);
        if unreadable == Some(std::io::ErrorKind::PermissionDenied) {
            note(ui, tr("Picasso isn't allowed to read the library file. Check who owns it and its permissions, then try again."));
        } else if busy(&self.error) || unreadable.is_some() {
            note(ui, tr("Another program may be using the library, such as a backup or sync tool. Close it, then try again."));
        } else if findings.is_none() {
            note(ui, tr("Checking the library…"));
        } else if !damaged {
            note(ui, tr("The library file itself is intact, so Picasso won't replace it. Close other programs that may be using it, or free some disk space, then try again."));
        }

        ui.add_space(16.0);
        let width = ui.available_width();
        let choice = |ui: &mut egui::Ui, label: &str, text: &str| {
            let button = ui.add(egui::Button::new(RichText::new(label).strong()).min_size(Vec2::new(width, 30.0)));
            ui.add_space(2.0);
            note(ui, text);
            ui.add_space(10.0);
            button.clicked()
        };
        if choice(ui, tr("Try again"), tr("Opens Picasso again, for when the library was only busy.")) {
            self.restart(&ctx);
        }
        if damaged && !self.replaced {
            let kept = tr("The library that can't be opened stays in the same folder, renamed picasso-damaged with today's date.");
            if let Some((path, made_at)) = findings.and_then(|(_, backup)| backup) {
                let label = trf("Restore the backup of {}", &[&local_time(made_at)]);
                let text = format!("{} {kept}", tr("Changes since that backup are lost."));
                if choice(ui, &label, &text) {
                    self.replace(&ctx, Some(path));
                }
            }
            let text = format!("{} {kept}", tr("The new library starts with your Pictures folder; add your other folders again. Stars, captions, albums, and edits start empty."));
            if choice(ui, tr("Start a new library"), &text) {
                self.replace(&ctx, None);
            }
        }

        ui.horizontal(|ui| {
            if ui.button(tr("Open library folder")).clicked() {
                let _ = open::that(&self.data_dir);
            }
            if ui.button(tr("Quit")).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        if !self.note.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(&self.note).color(theme::DANGER));
        }
    }
}

impl eframe::App for Recovery {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::sidebar())).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() - 520.0).max(16.0) / 2.0);
                about::dialog_frame().show(ui, |ui| {
                    ui.set_width(560.0);
                    let mut close = false;
                    about::title_bar(ui, &self.logo, "Picasso", &mut close);
                    if close {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    egui::Frame::new().inner_margin(Margin::symmetric(28, 22)).show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| self.body(ui));
                    });
                });
            });
        });
    }
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(theme::muted())).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso-recovery-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::db::Database::open(&dir.join("picasso.db")).unwrap().create_album("Trip").unwrap();
        dir
    }

    #[test]
    fn only_a_damaged_library_is_offered_a_replacement() {
        let dir = data_dir("healthy");
        backup::create(&dir).unwrap();
        let healthy = Findings::of(&dir);
        assert!(!healthy.damaged && healthy.backup.is_none());

        std::fs::write(dir.join("picasso.db"), b"not a database".repeat(500)).unwrap();
        let damaged = Findings::of(&dir);
        assert!(damaged.damaged);
        assert!(damaged.backup.is_some(), "the newest good backup is offered");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_library_is_not_called_damaged() {
        let dir = data_dir("missing");
        std::fs::remove_file(dir.join("picasso.db")).unwrap();
        assert!(!Findings::of(&dir).damaged);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_library_picasso_may_not_read_is_not_called_damaged() {
        use std::os::unix::fs::PermissionsExt;
        let dir = data_dir("unreadable");
        let library = dir.join("picasso.db");
        std::fs::set_permissions(&library, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads any file, so there is nothing to test.
        if std::fs::File::open(&library).is_err() {
            let found = Findings::of(&dir);
            assert_eq!(found.unreadable, Some(std::io::ErrorKind::PermissionDenied));
            assert!(!found.damaged && found.backup.is_none());
        }
        std::fs::set_permissions(&library, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(Findings::of(&dir).unreadable, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_errors_count_as_busy() {
        assert!(busy("database is locked"));
        assert!(busy("Database Busy"));
        assert!(!busy("file is not a database"));
    }
}
