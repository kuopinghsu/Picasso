//! Handing photos to other programs: the picture or the path on the
//! clipboard, or the file in the program the system opens it with.

use eframe::egui;

use crate::i18n::tr;
use crate::worker::UiEvent;

use super::PicassoApp;

/// Longest side of a picture put on the clipboard. Bigger ones are slow to
/// paste and take a lot of memory in the program they go to.
const CLIPBOARD_EDGE: u32 = 4096;

impl PicassoApp {
    /// Puts photo `id`, with its edit, on the clipboard. It renders on a
    /// separate thread; [`UiEvent::Copied`] says when it's there. One copy
    /// runs at a time, since a large RAW file takes a lot of memory.
    pub(super) fn copy_picture(&mut self, ctx: &egui::Context, id: i64) {
        if self.copying {
            return;
        }
        let Some(photo) = self.index_of(id).and_then(|index| self.photos.get(index)) else { return };
        if photo.is_video() {
            self.activity = tr("Videos can't be copied as a picture").into();
            return;
        }
        let path = std::path::PathBuf::from(&photo.path);
        let rotation = photo.rotation;
        let settings = match self.edit.as_ref().filter(|session| session.id == id) {
            Some(session) => session.current(),
            None => self.edit_settings(id),
        };
        let (ctx, events) = (ctx.clone(), self.events_tx.clone());
        self.activity = tr("Copying the picture…").into();
        self.copying = true;
        let spawned = std::thread::Builder::new().name("picasso-copy".into()).spawn(move || {
            let result = crate::edit::render_edited(&path, &settings, rotation, Some(CLIPBOARD_EDGE)).map(|rgba| {
                let size = [rgba.width() as usize, rgba.height() as usize];
                ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()));
            });
            let _ = events.send(UiEvent::Copied(result));
            ctx.request_repaint();
        });
        if let Err(err) = spawned {
            self.copying = false;
            self.activity = err.to_string();
        }
    }

    /// Puts the full paths of `ids` on the clipboard, one per line.
    pub(super) fn copy_paths(&mut self, ctx: &egui::Context, ids: &[i64]) {
        let paths: Vec<&str> = ids
            .iter()
            .filter_map(|id| self.index_of(*id).and_then(|index| self.photos.get(index)))
            .map(|photo| photo.path.as_str())
            .collect();
        if paths.is_empty() {
            return;
        }
        ctx.copy_text(paths.join("\n"));
        self.activity = tr(if paths.len() == 1 { "Copied the path" } else { "Copied the paths" }).into();
    }

    /// Opens photo `id` in the program the system opens its type with.
    pub(super) fn open_externally(&mut self, id: i64) {
        let Some(path) = self.index_of(id).and_then(|index| self.photos.get(index)).map(|photo| photo.path.clone()) else {
            return;
        };
        if let Err(err) = open::that_detached(&path) {
            self.activity = err.to_string();
        }
    }

    /// True for a Ctrl+C (⌘C) this frame that isn't meant for selected
    /// text, which it then belongs to Picasso.
    pub(super) fn take_copy_command(ctx: &egui::Context) -> bool {
        let text_selected = ctx
            .with_plugin(|labels: &mut egui::text_selection::LabelSelectionState| labels.has_selection())
            .unwrap_or(false);
        if text_selected || ctx.egui_wants_keyboard_input() {
            return false;
        }
        ctx.input_mut(|input| {
            let copy = input.events.iter().any(|event| matches!(event, egui::Event::Copy));
            input.events.retain(|event| !matches!(event, egui::Event::Copy));
            copy
        })
    }
}
