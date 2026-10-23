//! Changing the date taken of the selected photos, for a camera whose clock
//! was off or a scanned print. The new date lives in the catalog; the files
//! stay as they are.

use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};
use eframe::egui::{self, Align, Layout, Margin, RichText};

use crate::i18n::{tr, trf};

use super::{about, theme, Modal, PicassoApp};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum DateMode {
    Shift,
    Set,
    File,
}

pub(super) struct DateDialog {
    ids: Vec<i64>,
    /// Earliest date taken among them, the one shown in the preview.
    first: i64,
    mode: DateMode,
    days: i64,
    hours: i64,
    minutes: i64,
    text: String,
}

impl PicassoApp {
    pub(super) fn open_adjust_date(&mut self, ids: Vec<i64>) {
        let first = ids
            .iter()
            .filter_map(|id| match self.index_of(*id) {
                Some(index) => self.photos.get(index).map(|photo| photo.taken_at),
                None => self.db.photo(*id).map(|photo| photo.taken_at),
            })
            .min();
        let Some(first) = first else { return };
        self.modal = Modal::AdjustDate(DateDialog {
            ids,
            first,
            mode: DateMode::Shift,
            days: 0,
            hours: 0,
            minutes: 0,
            text: format_time(first),
        });
    }

    pub(super) fn adjust_date_modal(&mut self, ctx: &egui::Context) {
        let Modal::AdjustDate(dialog) = &mut self.modal else { return };
        let mut close = false;
        let mut apply = false;
        let target = match dialog.mode {
            DateMode::Shift => Some(dialog.first + ((dialog.days * 24 + dialog.hours) * 60 + dialog.minutes) * 60),
            DateMode::Set => parse_time(&dialog.text),
            DateMode::File => None,
        };
        let response = egui::Modal::new(egui::Id::unique("adjust-date")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(460.0);
            about::title_bar(ui, &self.logo, tr("Adjust date taken"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let count = dialog.ids.len();
                let intro = if count == 1 {
                    tr("Picasso keeps the new date in the library; the file stays as it is.").to_string()
                } else {
                    trf("{} photos. Picasso keeps the new dates in the library; the files stay as they are.", &[&count])
                };
                ui.label(RichText::new(intro).color(theme::muted()));
                ui.radio_value(&mut dialog.mode, DateMode::Shift, tr("Shift by"));
                ui.add_enabled_ui(dialog.mode == DateMode::Shift, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.add(egui::DragValue::new(&mut dialog.days).range(-36_500..=36_500));
                        ui.label(tr("days"));
                        ui.add(egui::DragValue::new(&mut dialog.hours).range(-23..=23));
                        ui.label(tr("hours"));
                        ui.add(egui::DragValue::new(&mut dialog.minutes).range(-59..=59));
                        ui.label(tr("minutes"));
                    });
                });
                let set = if count == 1 { tr("Set to") } else { tr("Set the earliest photo to, and move the others with it") };
                ui.radio_value(&mut dialog.mode, DateMode::Set, set);
                ui.add_enabled_ui(dialog.mode == DateMode::Set, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.add(egui::TextEdit::singleline(&mut dialog.text).desired_width(160.0).hint_text("2019-05-17 14:30"));
                    });
                });
                ui.radio_value(&mut dialog.mode, DateMode::File, tr("Use the date in the file"));
                ui.add_space(4.0);
                let preview = match (dialog.mode, target) {
                    (DateMode::File, _) => tr("The date Picasso read from each file comes back.").to_string(),
                    (_, Some(target)) => format!("{}  →  {}", format_time(dialog.first), format_time(target)),
                    (_, None) => tr("Type a date such as 2019-05-17 14:30").to_string(),
                };
                let color = if dialog.mode != DateMode::File && target.is_none() { theme::DANGER } else { theme::text() };
                ui.label(RichText::new(preview).color(color));
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = dialog.mode == DateMode::File || target.is_some_and(|target| target != dialog.first);
                    if ui.add_enabled(ready, theme::primary_button(tr("Apply"))).clicked() {
                        apply = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if apply {
            let Modal::AdjustDate(dialog) = std::mem::replace(&mut self.modal, Modal::None) else { return };
            match (dialog.mode, target) {
                (DateMode::File, _) => self.db.reset_taken(&dialog.ids),
                (_, Some(target)) => self.db.shift_taken(&dialog.ids, target - dialog.first),
                (_, None) => return,
            }
            self.activity = trf("Changed the date of {} photos", &[&dialog.ids.len()]);
            self.reload();
            return;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
        }
    }
}

fn format_time(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|time| time.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// A local date and time typed as `2019-05-17 14:30`, with `/` or `.`
/// allowed between the parts and the time optional.
fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim().replace(['/', '.'], "-");
    let naive = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(&text, format).ok())
        .or_else(|| NaiveDate::parse_from_str(&text, "%Y-%m-%d").ok()?.and_hms_opt(12, 0, 0))?;
    let secs = Local.from_local_datetime(&naive).earliest()?.timestamp();
    (secs >= crate::types::EARLIEST_PHOTO).then_some(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_dates_are_read_in_local_time() {
        let noon = Local.with_ymd_and_hms(2019, 5, 17, 12, 0, 0).unwrap().timestamp();
        assert_eq!(parse_time("2019-05-17"), Some(noon));
        assert_eq!(parse_time(" 2019/05/17 14:30 "), Some(noon + 9000));
        assert_eq!(parse_time("2019.05.17 14:30:15"), Some(noon + 9015));
        assert_eq!(format_time(noon + 9000), "2019-05-17 14:30");
        assert_eq!(parse_time("17 May"), None);
        assert_eq!(parse_time("1700-01-01"), None);
    }
}
