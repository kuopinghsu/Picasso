//! Non-destructive photo editing in the style of Lightcraft's Develop
//! module. An edit is a list of steps saved in the library; they are
//! rendered over a decoded copy for previews, thumbnails, and the full-size
//! view, and over the original only when exporting to a new file.

mod auto;
mod color;
mod curve;
mod filters;
mod geometry;
mod hdr;
pub mod pipeline;
pub mod presets;
mod settings;

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use image::RgbaImage;

pub use auto::{auto_tone, auto_white_balance, sample, white_balance_for};
pub use color::hue_rgb;
pub use curve::{parametric_points, sample as sample_curve};
pub use geometry::{canvas_size, constrain, inside, is_convex, with_aspect, Geometry, FULL, SQUARE};
pub use pipeline::{Cache, Options};
pub use settings::{
    Control, Curve, History, Kind, Mask, Shape, Settings, Step, Unit, BANDS, CURVE_CHANNELS, HDR_METHODS, LOCALS, MIX_PARTS, PROFILES,
    ZONES,
};

use crate::i18n::tr;
use crate::photo_info::Histogram;
use crate::worker::UiEvent;

/// Long edge of the image the editor previews on.
pub const PREVIEW_EDGE: u32 = 2048;
/// Long edge of the quick renders drawn while a slider moves.
pub const DRAFT_EDGE: u32 = 1024;

/// Replaces `rgba` with the edited photo.
pub fn apply(rgba: &mut RgbaImage, settings: &Settings) {
    if settings.is_identity() {
        return;
    }
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| pipeline::render(rgba, settings, &Options::default(), None)));
    if let Ok(result) = result {
        *rgba = result;
    }
}

/// Which picture of the stage a render is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Preview,
    /// The unedited photo with the same straighten and crop, for comparing.
    Before,
}

pub enum EditJob {
    /// Decode the photo at preview size.
    Load { id: i64, path: String },
    Render {
        id: i64,
        seq: u64,
        base: Arc<RgbaImage>,
        settings: Settings,
        options: Options,
        purpose: Purpose,
        /// Render at [`DRAFT_EDGE`] for speed while the user drags.
        draft: bool,
    },
}

/// Editor thread: decodes preview images and renders edits onto them. Of
/// the render requests that queue up while a slider moves, only the newest
/// of each purpose is drawn.
pub fn edit_loop(jobs: Receiver<EditJob>, events: Sender<UiEvent>, ctx: eframe::egui::Context) {
    let mut full = Cache::default();
    let mut quick = Cache::default();
    let mut draft_base: Option<(Arc<RgbaImage>, Arc<RgbaImage>)> = None;
    while let Ok(first) = jobs.recv() {
        let mut pending = vec![first];
        pending.extend(jobs.try_iter());
        let newest = |purpose: Purpose| {
            pending
                .iter()
                .rposition(|job| matches!(job, EditJob::Render { purpose: p, .. } if *p == purpose))
        };
        let keep = [newest(Purpose::Preview), newest(Purpose::Before)];
        for (index, job) in pending.into_iter().enumerate() {
            if matches!(job, EditJob::Render { .. }) && !keep.contains(&Some(index)) {
                continue;
            }
            let event = match job {
                EditJob::Load { id, path } => {
                    full.clear();
                    quick.clear();
                    draft_base = None;
                    load_preview(id, &path)
                }
                EditJob::Render {
                    id,
                    seq,
                    base,
                    settings,
                    options,
                    purpose,
                    draft,
                } => {
                    let small = draft && base.width().max(base.height()) > DRAFT_EDGE;
                    let (source, cache) = if small {
                        if draft_base.as_ref().is_none_or(|(of, _)| !Arc::ptr_eq(of, &base)) {
                            let image = image::DynamicImage::ImageRgba8((*base).clone());
                            let shrunk = Arc::new(crate::image_io::limit_rgba(image, DRAFT_EDGE));
                            quick.clear();
                            draft_base = Some((base.clone(), shrunk));
                        }
                        (draft_base.as_ref().map(|(_, small)| small.clone()).unwrap_or(base), &mut quick)
                    } else {
                        (base, &mut full)
                    };
                    let rendered = std::panic::catch_unwind(AssertUnwindSafe(|| {
                        let mut image = pipeline::render(&source, &settings, &options, Some(cache));
                        let histogram = (purpose == Purpose::Preview).then(|| Box::new(Histogram::from_rgba(image.as_raw())));
                        let canvas = [source.width() as f32, source.height() as f32];
                        pipeline::decorate(&mut image, &settings, &options, canvas, pipeline::view(&settings, &options));
                        (image, histogram)
                    }));
                    match rendered {
                        Ok((image, histogram)) => UiEvent::EditPreview {
                            id,
                            seq,
                            purpose,
                            w: image.width() as usize,
                            h: image.height() as usize,
                            rgba: image.into_raw(),
                            histogram,
                        },
                        Err(_) => {
                            full.clear();
                            quick.clear();
                            UiEvent::EditFailed {
                                id,
                                error: tr("The edit couldn't be rendered").into(),
                            }
                        }
                    }
                }
            };
            if events.send(event).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    }
}

fn load_preview(id: i64, path: &str) -> UiEvent {
    let path = Path::new(path);
    let decoded = std::panic::catch_unwind(|| crate::image_io::load_at_least(path, PREVIEW_EDGE)).unwrap_or_else(|_| Err("decoder fault".into()));
    match decoded {
        Ok(image) => UiEvent::EditBase {
            id,
            image: Arc::new(crate::image_io::limit_rgba(image, PREVIEW_EDGE)),
        },
        Err(error) => UiEvent::EditFailed { id, error },
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportOptions {
    /// JPEG quality, 1…100.
    pub quality: u8,
    /// Longest side of the file in pixels; `None` keeps the full size.
    pub long_edge: Option<u32>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            quality: 92,
            long_edge: None,
        }
    }
}

/// Writes the edited photo to `target`: JPEG, PNG, or TIFF by extension.
/// `rotation` is the clockwise turn stored in the catalog.
pub fn export(source: &Path, target: &Path, settings: &Settings, rotation: i32, options: ExportOptions) -> Result<(), String> {
    if same_file(source, target) {
        return Err(tr("Choose a different name than the original").into());
    }
    let image = std::panic::catch_unwind(|| crate::image_io::load_oriented(source)).unwrap_or_else(|_| Err("decoder fault".into()))?;
    let mut rgba = image.to_rgba8();
    drop(image);
    if !settings.is_identity() {
        rgba = std::panic::catch_unwind(AssertUnwindSafe(|| pipeline::render(&rgba, settings, &Options::default(), None)))
            .map_err(|_| tr("The edit couldn't be rendered").to_string())?;
    }
    if let Some(edge) = options.long_edge.filter(|edge| *edge < rgba.width().max(rgba.height())) {
        let scale = edge as f32 / rgba.width().max(rgba.height()) as f32;
        let (w, h) = (
            ((rgba.width() as f32 * scale).round() as u32).max(1),
            ((rgba.height() as f32 * scale).round() as u32).max(1),
        );
        rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Lanczos3);
    }
    let rgba = match rotation.rem_euclid(360) {
        90 => image::imageops::rotate90(&rgba),
        180 => image::imageops::rotate180(&rgba),
        270 => image::imageops::rotate270(&rgba),
        _ => rgba,
    };
    let extension = target
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => crate::image_io::save_lossless(target, &rgba, image::ImageFormat::Png),
        "tif" | "tiff" => crate::image_io::save_lossless(target, &rgba, image::ImageFormat::Tiff),
        _ => crate::image_io::save_jpeg(target, &rgba, options.quality.clamp(1, 100)),
    }
}

/// True when `a` and `b` name one file, also through a link or another
/// letter case on a disk that ignores case.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return a.dev() == b.dev() && a.ino() == b.ino();
        }
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b || (cfg!(windows) && a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()),
        _ => false,
    }
}

/// `photo.jpg` → `photo-edited.jpg` beside the original; a RAW or HEIC
/// source exports as JPEG.
pub fn export_name(source: &Path) -> String {
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "photo".into());
    let extension = source
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .filter(|ext| matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "tif" | "tiff"))
        .unwrap_or_else(|| "jpg".into());
    format!("{stem}-edited.{extension}")
}

/// Size of the edited photo for an uncropped `w`×`h` photo.
pub fn edited_size(settings: &Settings, w: u32, h: u32) -> (u32, u32) {
    Geometry::from_settings(settings).output_size(w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untouched_codes_round_trip() {
        let decode = color::decode_table();
        for code in 0..=255usize {
            assert_eq!(color::encode(decode[code]) as usize, code, "code {code}");
        }
        let mut image = RgbaImage::from_fn(16, 16, |x, y| image::Rgba([(x * 16) as u8, (y * 16) as u8, 77, 255]));
        let original = image.clone();
        apply(&mut image, &Settings::default());
        assert_eq!(image, original);
    }

    #[test]
    fn export_names_sit_beside_the_original() {
        assert_eq!(export_name(Path::new("a/b/photo.JPG")), "photo-edited.jpg");
        assert_eq!(export_name(Path::new("raw.cr2")), "raw-edited.jpg");
    }

    #[test]
    fn cropped_edits_report_their_size() {
        let mut settings = Settings::default();
        settings.set_crop([0.0, 0.0, 0.5, 0.5]);
        assert_eq!(edited_size(&settings, 400, 300), (200, 150));
    }
}
