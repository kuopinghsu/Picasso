//! Shown instead of the library when `picasso.db` can't be opened: damaged,
//! locked by another program, or unreadable. Nothing is changed until the
//! user picks a way out, and the library that failed is always kept.

use std::path::PathBuf;

use eframe::egui::{self, Margin, RichText, Vec2};

use crate::backup::{self, Backup};
use crate::i18n::{self, tr, trf, Lang};

use super::{about, settings::local_time, theme, theme::ThemeMode};

pub(super) struct Recovery {
    data_dir: PathBuf,
    error: String,
    backups: Vec<Backup>,
    logo: egui::TextureHandle,
    /// Why the latest choice didn't work.
    note: String,
}

impl Recovery {
    pub(super) fn new(ctx: &egui::Context, data_dir: PathBuf, error: String) -> Self {
        eprintln!("picasso library: {error}");
        let config = crate::db::config_dir();
        let stored = |key| crate::config::stored_text(&config, key);
        let mode = ThemeMode::from_code(&stored("theme").unwrap_or_default());
        theme::apply(ctx, mode.is_dark(super::system_dark(ctx)));
        i18n::set(stored("language").as_deref().and_then(Lang::from_code).unwrap_or_else(i18n::system));
        let backups = backup::list(&data_dir).into_iter().filter(|backup| backup::check(&backup.path).is_ok()).collect();
        Self { data_dir, error, backups, logo: theme::load_logo(ctx), note: String::new() }
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
            Ok(_) => self.restart(ctx),
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
        let locked = ["locked", "busy"].iter().any(|word| self.error.to_lowercase().contains(word));
        if locked {
            ui.add_space(4.0);
            note(ui, tr("Another program may be using the library, such as a backup or sync tool. Close it, then try again."));
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
        let kept = tr("The library that can't be opened stays in the same folder, renamed picasso-damaged with today's date.");
        if let Some(newest) = self.backups.first() {
            let label = trf("Restore the backup of {}", &[&local_time(newest.made_at)]);
            let text = format!("{} {kept}", tr("Changes since that backup are lost."));
            if choice(ui, &label, &text) {
                let path = newest.path.clone();
                self.replace(&ctx, Some(path));
            }
        }
        let text = format!("{} {kept}", tr("The new library starts with your Pictures folder; add your other folders again. Stars, captions, albums, and edits start empty."));
        if choice(ui, tr("Start a new library"), &text) {
            self.replace(&ctx, None);
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
