//! How a photo's camera details and location read in the side panel and the
//! viewer's info panel, each in a card: the camera and lens as the camera
//! card's title, the four values that say how the photo was taken as chips,
//! then the rest grouped; and the location in a card of its own.

use eframe::egui::{self, RichText, Vec2};

use super::theme;
use crate::i18n::tr;

/// The values a photographer reads first, as chips, in this order.
const SHOT: [&str; 4] = ["Aperture", "Exposure time", "ISO", "Focal length"];

/// The camera card's title and the line under it.
const TITLE: [&str; 2] = ["Camera", "Lens"];

/// The other rows, in groups apart by a thin line, in this order. Rows not
/// named in any group go last.
const GROUPS: [(&str, &[&str]); 3] = [
    (
        "Exposure",
        &[
            "Program",
            "Exposure bias",
            "Metering",
            "Flash",
            "White balance",
            "35 mm equiv.",
        ],
    ),
    ("File", &["Color space", "Software"]),
    ("Copyright", &["Artist", "Copyright"]),
];

/// Rows shown elsewhere: the date with the photo's own details, and the
/// altitude with its location.
const ELSEWHERE: [&str; 2] = ["Taken", "Altitude"];

/// Draws `rows`, the photo's EXIF as read, `id` keeping each grid apart,
/// and returns how many groups of rows it drew. A video's rows, which are
/// about its file and streams, are one plain list.
pub(super) fn exif(
    ui: &mut egui::Ui,
    id: (&str, i64),
    rows: &[(&'static str, String)],
    video: bool,
) -> usize {
    if video {
        grid(ui, (id.0, id.1, "video"), rows);
        return 1;
    }
    let value = |label: &str| {
        rows.iter()
            .find(|(name, _)| *name == label)
            .map(|(_, value)| value.as_str())
    };
    // The camera and lens as the card's title, as a photo app names a shot.
    if let Some(camera) = value("Camera") {
        let response = ui.add(
            egui::Label::new(
                RichText::new(camera)
                    .size(14.0)
                    .strong()
                    .color(theme::text()),
            )
            .wrap()
            .sense(egui::Sense::click()),
        );
        copy_menu(&response, camera);
    }
    if let Some(lens) = value("Lens") {
        let response = ui.add(
            egui::Label::new(RichText::new(lens).size(12.0).color(theme::muted()))
                .wrap()
                .sense(egui::Sense::click()),
        );
        copy_menu(&response, lens);
    }
    let chips: Vec<String> = SHOT
        .iter()
        .filter_map(|label| {
            value(label).map(|text| {
                if *label == "ISO" {
                    format!("ISO {text}")
                } else {
                    text.to_string()
                }
            })
        })
        .collect();
    if !chips.is_empty() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for chip in &chips {
                // Laid out whole and placed as one piece, so a chip that
                // doesn't fit goes to the next row rather than wrapping its
                // words a letter at a time.
                let galley = ui.painter().layout_no_wrap(
                    chip.clone(),
                    egui::FontId::proportional(12.5),
                    theme::text(),
                );
                let size = galley.size() + Vec2::new(16.0, 6.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                ui.painter().rect_filled(rect, 10, theme::well());
                ui.painter()
                    .galley(rect.center() - galley.size() / 2.0, galley, theme::text());
                let response = response.on_hover_cursor(egui::CursorIcon::ContextMenu);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, chip));
                copy_menu(&response, chip);
            }
        });
    }
    // Each group under a thin line rather than a heading: the card says
    // what they are about and each row's label says the rest.
    let mut shown: Vec<&str> = SHOT
        .iter()
        .chain(TITLE.iter())
        .chain(ELSEWHERE.iter())
        .copied()
        .collect();
    let mut drawn = 0;
    let mut before = value("Camera").is_some() || value("Lens").is_some() || !chips.is_empty();
    let mut groups: Vec<(&str, Vec<(&'static str, String)>)> = Vec::new();
    for (title, labels) in GROUPS {
        groups.push((
            title,
            rows.iter()
                .filter(|(label, _)| labels.contains(label))
                .cloned()
                .collect(),
        ));
        shown.extend(labels.iter().copied());
    }
    groups.push((
        "other",
        rows.iter()
            .filter(|(label, _)| !shown.contains(label))
            .cloned()
            .collect(),
    ));
    for (title, group) in groups.into_iter().filter(|(_, group)| !group.is_empty()) {
        if before {
            divider(ui);
        }
        grid(ui, (id.0, id.1, title), &group);
        before = true;
        drawn += 1;
    }
    drawn
}

/// A thin line across the card between groups of rows.
fn divider(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    let (line, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0, theme::hairline());
    ui.add_space(6.0);
}

/// A card: a rounded panel a shade apart from the panel around it, with
/// `title` in small capitals over what `add` draws.
pub(super) fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(10.0);
    egui::Frame::new()
        .fill(theme::bg())
        .stroke(egui::Stroke::new(1.0, theme::hairline()))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(title.to_uppercase())
                    .size(10.5)
                    .strong()
                    .color(theme::muted()),
            );
            ui.add_space(4.0);
            add(ui)
        })
        .inner
}

/// What a click in the location section asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LocationAction {
    /// Show the photo in Places, where a location is set or changed.
    Places,
    /// Open the site at this zoom.
    OpenStreetMap,
}

/// Draws where photo `id` was taken: `place` from its GPS or a pin
/// (`pinned`), and `altitude` from its EXIF. Without a place, says so and
/// offers to set one. `in_places` leaves out the way to Places, as the
/// person is there.
pub(super) fn location(
    ui: &mut egui::Ui,
    id: i64,
    place: Option<(f64, f64)>,
    pinned: bool,
    altitude: Option<&str>,
    in_places: bool,
) -> Option<LocationAction> {
    let mut action = None;
    match place {
        Some((lat, lon)) => {
            let response = ui.add(
                egui::Label::new(RichText::new(dms(lat, lon)).size(13.0).color(theme::text()))
                    .wrap(),
            );
            copy_menu(&response, &decimal(lat, lon));
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(decimal(lat, lon))
                        .size(11.5)
                        .color(theme::muted()),
                );
                ui.label(RichText::new("·").size(11.5).color(theme::muted()));
                ui.label(
                    RichText::new(tr(if pinned {
                        "Pinned in Picasso"
                    } else {
                        "From camera GPS"
                    }))
                    .size(11.5)
                    .color(theme::muted()),
                );
            });
            if let Some(altitude) = altitude {
                grid(
                    ui,
                    ("altitude", id, ""),
                    &[("Altitude", altitude.to_string())],
                );
            }
            ui.add_space(8.0);
            // One column of buttons the same width and height, which line up
            // whatever the language makes of their words.
            ui.spacing_mut().item_spacing.y = 6.0;
            if !in_places && action_button(ui, tr("Show in Places"), true) {
                action = Some(LocationAction::Places);
            }
            if action_button(ui, tr("Copy coordinates"), false) {
                ui.ctx().copy_text(decimal(lat, lon));
            }
            if action_button(ui, tr("Open in OpenStreetMap"), false) {
                action = Some(LocationAction::OpenStreetMap);
            }
        }
        None => {
            ui.label(
                RichText::new(tr("No GPS location"))
                    .size(12.0)
                    .color(theme::muted()),
            );
            ui.add_space(6.0);
            if !in_places && action_button(ui, tr("Geotag it in Places"), true) {
                action = Some(LocationAction::Places);
            }
        }
    }
    action
}

/// A button as wide as the card and [`ACTION_HEIGHT`] tall, its words
/// centred: filled with the accent when `primary`, the card's first action.
fn action_button(ui: &mut egui::Ui, text: &str, primary: bool) -> bool {
    let size = Vec2::new(ui.available_width(), ACTION_HEIGHT);
    let button = if primary {
        theme::primary_button(text)
    } else {
        egui::Button::new(RichText::new(text).size(13.0))
    };
    ui.add_sized(size, button.corner_radius(8)).clicked()
}

/// Height of the Location card's buttons.
const ACTION_HEIGHT: f32 = 28.0;

/// Labels in a muted column beside their values; a right-click on a value
/// copies it. The labels are English keys for `tr`.
pub(super) fn grid(ui: &mut egui::Ui, id: (&str, i64, &str), rows: &[(&'static str, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(Vec2::new(12.0, 5.0))
        .min_col_width(70.0)
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(RichText::new(tr(label)).size(12.0).color(theme::muted()));
                let response = ui.add(
                    egui::Label::new(RichText::new(value).size(12.5).color(theme::text()))
                        .wrap()
                        .sense(egui::Sense::click()),
                );
                copy_menu(&response, value);
                ui.end_row();
            }
        });
}

/// A right-click menu on `response` that copies `text`.
fn copy_menu(response: &egui::Response, text: &str) {
    response.context_menu(|ui| {
        if ui.button(tr("Copy")).clicked() {
            ui.ctx().copy_text(text.to_string());
            ui.close();
        }
    });
}

/// Latitude and longitude as decimal degrees, as maps and other apps take them.
pub(super) fn decimal(lat: f64, lon: f64) -> String {
    format!("{lat:.5}, {lon:.5}")
}

/// Latitude and longitude in degrees, minutes and seconds, as people read them.
pub(super) fn dms(lat: f64, lon: f64) -> String {
    let part = |value: f64, positive: char, negative: char| {
        // Rounded to whole seconds first, so 59.6″ carries into the minute.
        let total = (value.abs() * 3600.0).round() as u64;
        let (degrees, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
        format!(
            "{degrees}°{minutes:02}′{seconds:02}″{}",
            if value < 0.0 { negative } else { positive }
        )
    };
    format!("{} {}", part(lat, 'N', 'S'), part(lon, 'E', 'W'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_read_in_degrees_and_as_decimals() {
        assert_eq!(dms(12.582194, -65.7225), "12°34′56″N 65°43′21″W");
        assert_eq!(dms(-0.0001, 0.0), "0°00′00″S 0°00′00″E");
        assert_eq!(
            dms(10.999999, 20.0),
            "11°00′00″N 20°00′00″E",
            "a second rounded up carries"
        );
        assert_eq!(decimal(12.582194, -65.7225), "12.58219, -65.72250");
    }

    #[test]
    fn both_sections_fit_the_narrowest_panel() {
        let rows: Vec<(&'static str, String)> = vec![
            ("Camera", "Nikon Z 8".into()),
            ("Lens", "NIKKOR Z 180-600mm f/5.6-6.3 VR".into()),
            ("Exposure time", "1/2000 s".into()),
            ("Aperture", "f/6.3".into()),
            ("ISO", "1600".into()),
            ("Focal length", "600 mm".into()),
            ("Flash", "not fired, compulsory flash mode".into()),
            ("Software", "Ver.03.10".into()),
            ("Altitude", "1234.5 m".into()),
        ];
        // The viewer's info panel: 256 points wide, less its margins.
        let width = 232.0;
        let ctx = egui::Context::default();
        let mut used = (0.0, 0.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let drawn = ui.allocate_ui(Vec2::new(width, 2000.0), |ui| {
                        ui.set_max_width(width);
                        card(ui, "Camera", |ui| exif(ui, ("exif", 1), &rows, false));
                        card(ui, "Location", |ui| {
                            location(
                                ui,
                                1,
                                Some((12.582194, -65.7225)),
                                false,
                                Some("1234.5 m"),
                                false,
                            )
                        });
                    });
                    used = (drawn.response.rect.width(), drawn.response.rect.height());
                });
            },
        );
        output.textures_delta.clear();
        assert!(used.0 <= width + 0.5, "wider than the panel: {used:?}");
    }

    #[test]
    fn chips_that_dont_fit_go_to_the_next_row_whole() {
        let rows: Vec<(&'static str, String)> = vec![
            ("Aperture", "f/1.8".into()),
            ("Exposure time", "1/4608 s".into()),
            ("ISO", "64".into()),
            ("Focal length", "6.8 mm".into()),
        ];
        let ctx = egui::Context::default();
        let mut height = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    // Narrower than all four chips side by side.
                    let drawn = ui.allocate_ui(Vec2::new(180.0, 2000.0), |ui| {
                        exif(ui, ("exif", 1), &rows, false)
                    });
                    height = drawn.response.rect.height();
                });
            },
        );
        output.textures_delta.clear();
        assert!(
            height > 30.0 && height < 70.0,
            "two rows of whole chips, not one chip a letter a line: {height}"
        );
    }

    #[test]
    fn a_videos_details_are_one_list_and_a_photos_are_grouped() {
        let rows: Vec<(&'static str, String)> = vec![
            ("Duration", "0:12".into()),
            ("Camera", "Phone".into()),
            ("Software", "18.1".into()),
            ("Bit rate", "12 Mbit/s".into()),
        ];
        let ctx = egui::Context::default();
        let (mut video, mut photo) = (0, 0);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                video = exif(ui, ("video", 1), &rows, true);
                photo = exif(ui, ("photo", 2), &rows, false);
            });
        });
        output.textures_delta.clear();
        assert_eq!(video, 1, "a video's rows are one list");
        assert!(photo > 1, "the same rows of a photo are grouped: {photo}");
    }

    #[test]
    fn the_location_buttons_line_up_the_same_size() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.allocate_ui(Vec2::new(232.0, 900.0), |ui| {
                        card(ui, "Location", |ui| {
                            location(ui, 1, Some((12.58, -65.72)), false, None, false)
                        })
                    });
                });
            },
        );
        output.textures_delta.clear();
        let update = output
            .platform_output
            .accesskit_update
            .take()
            .expect("accessibility update");
        let buttons: Vec<egui::accesskit::Rect> = update
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == egui::accesskit::Role::Button)
            .filter_map(|(_, node)| node.bounds())
            .collect();
        assert_eq!(
            buttons.len(),
            3,
            "Show in Places, Copy coordinates, Open in OpenStreetMap"
        );
        let size = |rect: &egui::accesskit::Rect| {
            ((rect.x1 - rect.x0).round(), (rect.y1 - rect.y0).round())
        };
        assert!(
            buttons.iter().all(
                |rect| size(rect) == size(&buttons[0]) && (rect.x0 - buttons[0].x0).abs() < 0.5
            ),
            "same size, one column: {buttons:?}"
        );
    }

    #[test]
    fn every_exif_label_has_a_place() {
        // Each label photo_info gives is a chip, in a group, or shown elsewhere.
        let named: Vec<&str> = SHOT
            .iter()
            .chain(TITLE.iter())
            .chain(ELSEWHERE.iter())
            .chain(GROUPS.iter().flat_map(|(_, labels)| labels.iter()))
            .copied()
            .collect();
        for label in [
            "Lens",
            "Taken",
            "Exposure time",
            "Aperture",
            "ISO",
            "Focal length",
            "35 mm equiv.",
            "Exposure bias",
            "Program",
            "Metering",
            "Flash",
            "White balance",
            "Color space",
            "Altitude",
            "Software",
            "Artist",
            "Copyright",
            "Camera",
        ] {
            assert!(named.contains(&label), "{label}");
        }
    }
}
