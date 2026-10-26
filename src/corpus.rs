//! Test photos made on the fly, one for each format and EXIF orientation,
//! with known pixels, and the checks every photo must pass: its size is read
//! upright, it decodes upright, its thumbnail keeps its shape, and an export
//! keeps the orientation, rotation, crop, and size asked for.
//!
//! HEIC and camera RAW files can't be made here. To put real photos of any
//! format through the same reading checks, point `PICASSO_CORPUS` at a folder
//! of them: `PICASSO_CORPUS=~/corpus cargo test --release corpus -- --nocapture`.

use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

use crate::edit::{self, Control, ExportOptions, Settings};
use crate::image_io;

const RED: [u8; 3] = [220, 30, 30];
const GREEN: [u8; 3] = [30, 200, 60];
const BLUE: [u8; 3] = [30, 60, 220];
const WHITE: [u8; 3] = [245, 245, 245];
/// JPEG and resizing blur the colors a little.
const TOLERANCE: i32 = 40;

/// Red, green, blue, and white quarters, clockwise from the top left, as
/// the photo should look upright.
fn scene(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let [r, g, b] = match (x < w / 2, y < h / 2) {
            (true, true) => RED,
            (false, true) => GREEN,
            (true, false) => WHITE,
            (false, false) => BLUE,
        };
        Rgba([r, g, b, 255])
    })
}

/// The quarters an upright image shows, clockwise from the top left.
fn quarters(image: &RgbaImage) -> [[u8; 3]; 4] {
    let (w, h) = image.dimensions();
    let at = |fx: f32, fy: f32| {
        let pixel = image.get_pixel(((w as f32 * fx) as u32).min(w - 1), ((h as f32 * fy) as u32).min(h - 1));
        [pixel[0], pixel[1], pixel[2]]
    };
    [at(0.25, 0.25), at(0.75, 0.25), at(0.75, 0.75), at(0.25, 0.75)]
}

fn assert_quarters(image: &RgbaImage, expected: [[u8; 3]; 4], what: &str) {
    for (found, wanted) in quarters(image).iter().zip(expected) {
        let close = found.iter().zip(wanted).all(|(a, b)| (*a as i32 - b as i32).abs() <= TOLERANCE);
        assert!(close, "{what}: quarters {:?}, expected {expected:?}", quarters(image));
    }
}

const UPRIGHT: [[u8; 3]; 4] = [RED, GREEN, BLUE, WHITE];

/// The stored pixels of a photo whose EXIF orientation turns them into `upright`.
fn stored_for(upright: &RgbaImage, orientation: u16) -> RgbaImage {
    use image::imageops::{flip_horizontal, flip_vertical, rotate180, rotate270, rotate90};
    match orientation {
        2 => flip_horizontal(upright),
        3 => rotate180(upright),
        4 => flip_vertical(upright),
        // Transposed and transversed: each is its own inverse.
        5 => rotate270(&flip_horizontal(upright)),
        6 => rotate270(upright),
        7 => rotate90(&flip_horizontal(upright)),
        8 => rotate90(upright),
        _ => upright.clone(),
    }
}

/// A JPEG with an EXIF block holding only `orientation`.
fn jpeg_with_orientation(rgba: &RgbaImage, orientation: u16) -> Vec<u8> {
    let jpeg = image_io::encode_jpeg(rgba, 95).unwrap();
    let mut tiff = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend(app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("picasso-corpus-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every format Picasso decodes itself, plus JPEGs in all eight EXIF
/// orientations, each showing [`scene`] upright at 96×64.
fn make_corpus(dir: &Path) -> Vec<PathBuf> {
    let upright = scene(96, 64);
    let mut files = Vec::new();
    for orientation in 1..=8 {
        let path = dir.join(format!("orientation-{orientation}.jpg"));
        std::fs::write(&path, jpeg_with_orientation(&stored_for(&upright, orientation), orientation)).unwrap();
        files.push(path);
    }
    let plain = dir.join("no-exif.jpg");
    std::fs::write(&plain, image_io::encode_jpeg(&upright, 95).unwrap()).unwrap();
    files.push(plain);
    for (name, format) in [
        ("photo.png", ImageFormat::Png),
        ("photo.tif", ImageFormat::Tiff),
        ("photo.bmp", ImageFormat::Bmp),
        ("photo.gif", ImageFormat::Gif),
        ("photo.webp", ImageFormat::WebP),
    ] {
        let path = dir.join(name);
        let image = if format == ImageFormat::Bmp || format == ImageFormat::Gif {
            DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(upright.clone()).into_rgb8())
        } else {
            DynamicImage::ImageRgba8(upright.clone())
        };
        image.save_with_format(&path, format).unwrap();
        files.push(path);
    }
    files
}

#[test]
fn every_format_and_orientation_reads_upright() {
    let dir = folder("read");
    for path in make_corpus(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let probe = image_io::probe(&path, 42).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!((probe.width, probe.height), (96, 64), "{name}: size");
        assert_eq!(probe.taken_at, 42, "{name}: without a date the file's time is used");

        let full = image_io::load_oriented(&path).unwrap().to_rgba8();
        assert_eq!(full.dimensions(), (96, 64), "{name}");
        assert_quarters(&full, UPRIGHT, &name);

        let tile = image_io::thumbnail_rgba(&image_io::load_for_tile(&path, 32).unwrap(), 32);
        assert_eq!(tile.dimensions(), (48, 32), "{name}: thumbnail");
        assert_quarters(&tile, UPRIGHT, &format!("{name} thumbnail"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transparency_survives_a_png() {
    let dir = folder("alpha");
    let path = dir.join("clear.png");
    let mut image = scene(40, 40);
    image.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
    image.save(&path).unwrap();
    assert_eq!(image_io::load_oriented(&path).unwrap().to_rgba8().get_pixel(0, 0)[3], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn damaged_and_odd_files_fail_without_panicking() {
    let dir = folder("odd");
    let whole = jpeg_with_orientation(&scene(96, 64), 6);
    let cases: [(&str, Vec<u8>); 4] = [
        ("empty.jpg", Vec::new()),
        ("text.jpg", b"this is not a photo".to_vec()),
        ("cut.jpg", whole[..whole.len() / 3].to_vec()),
        ("wrong-extension.png", whole.clone()),
    ];
    for (name, bytes) in cases {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        // Any answer will do, as long as reading returns.
        let _ = image_io::probe(&path, 0);
        let _ = std::panic::catch_unwind(|| image_io::load_oriented(&path)).expect(name);
        let _ = std::panic::catch_unwind(|| image_io::load_for_tile(&path, 32)).expect(name);
    }
    let tiny = dir.join("tiny.png");
    RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])).save(&tiny).unwrap();
    assert_eq!(image_io::load_oriented(&tiny).unwrap().width(), 1);
    let wide = dir.join("panorama.png");
    scene(3000, 50).save(&wide).unwrap();
    assert_eq!(image_io::thumbnail_rgba(&image_io::load_for_tile(&wide, 32).unwrap(), 32).dimensions(), (96, 2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_keep_orientation_rotation_crop_and_size() {
    let dir = folder("export");
    let source = dir.join("photo.jpg");
    std::fs::write(&source, jpeg_with_orientation(&stored_for(&scene(96, 64), 6), 6)).unwrap();
    let decode = |path: &Path| image::open(path).unwrap().to_rgba8();
    for extension in ["jpg", "png", "tif"] {
        let target = dir.join(format!("plain.{extension}"));
        edit::export(&source, &target, &Settings::default(), 0, ExportOptions::default()).unwrap();
        let out = decode(&target);
        assert_eq!(out.dimensions(), (96, 64), "{extension}");
        assert_quarters(&out, UPRIGHT, extension);

        // A quarter turn clockwise and a long edge of 48.
        let target = dir.join(format!("turned.{extension}"));
        let options = ExportOptions { quality: 90, long_edge: Some(48) };
        edit::export(&source, &target, &Settings::default(), 90, options).unwrap();
        let out = decode(&target);
        assert_eq!(out.dimensions(), (32, 48), "{extension} turned");
        assert_quarters(&out, [WHITE, RED, GREEN, BLUE], &format!("{extension} turned"));
    }

    // The left half, brightened: the same picture the viewer shows.
    let mut settings = Settings::default();
    settings.set_crop([0.0, 0.0, 0.5, 1.0]);
    settings.set(Control::from_key("light.exposure").unwrap(), 0.5);
    let target = dir.join("edited.png");
    edit::export(&source, &target, &settings, 0, ExportOptions::default()).unwrap();
    let exported = decode(&target);
    let mut shown = image_io::load_oriented(&source).unwrap().to_rgba8();
    edit::apply(&mut shown, &settings);
    assert_eq!(exported.dimensions(), shown.dimensions());
    assert_eq!(exported.dimensions(), (48, 64));
    let difference = exported.pixels().zip(shown.pixels()).map(|(a, b)| (a[0] as i32 - b[0] as i32).abs()).max().unwrap();
    assert!(difference <= 2, "the export and the viewer differ by {difference}");
    let red = quarters(&exported)[0];
    assert!(red[0] > RED[0] - 5 && red[1] > RED[1], "exposure brightened the red quarter: {red:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn combined_transforms_export_at_the_size_the_editor_reports() {
    let dir = folder("transforms");
    let source = dir.join("photo.png");
    scene(120, 80).save(&source).unwrap();
    let combos: [&[(Control, f32)]; 5] = [
        &[(Control::STRAIGHTEN, 12.0)],
        &[(Control::STRAIGHTEN, -30.0), (Control::FLIP_H, 1.0)],
        &[(Control::PERSPECTIVE_V, 20.0), (Control::FLIP_V, 1.0)],
        &[(Control::STRAIGHTEN, 90.0), (Control::PERSPECTIVE_H, -15.0)],
        &[(Control::STRAIGHTEN, 5.0), (Control::PERSPECTIVE_V, 10.0), (Control::PERSPECTIVE_H, 10.0), (Control::FLIP_H, 1.0)],
    ];
    for (index, combo) in combos.iter().enumerate() {
        let mut settings = Settings::default();
        for (control, value) in combo.iter() {
            settings.set(*control, *value);
        }
        settings.set_crop([0.1, 0.05, 0.8, 0.9]);
        let target = dir.join(format!("combo-{index}.png"));
        edit::export(&source, &target, &settings, 0, ExportOptions::default()).unwrap();
        let (w, h) = image::image_dimensions(&target).unwrap();
        let (ew, eh) = edit::edited_size(&settings, 120, 80);
        assert!(w.abs_diff(ew) <= 1 && h.abs_diff(eh) <= 1, "combo {index}: exported {w}×{h}, editor says {ew}×{eh}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edit_and_its_undo_point_survive_a_restart() {
    use crate::edit::{History, Kind};
    let dir = folder("history");
    let library = dir.join("picasso.db");
    let mut history = History::default();
    let mut settings = Settings::default();
    settings.set(Control::from_key("light.exposure").unwrap(), 0.7);
    history.commit(Kind::Adjust, &settings);
    settings.set_crop([0.0, 0.0, 0.5, 0.5]);
    history.commit(Kind::Crop, &settings);
    history.undo();
    {
        let db = crate::db::Database::open(&library).unwrap();
        let folder = dir.join("photos");
        std::fs::create_dir_all(&folder).unwrap();
        db.add_folder(&folder).unwrap();
        db.conn_for_tests().execute("INSERT INTO photos(id, folder_id, path, filename) VALUES (5, 1, 'a', 'a.jpg')", []).unwrap();
        db.save_edit_history(5, &history.steps_json(), history.steps().len(), history.applied());
    }
    let db = crate::db::Database::open(&library).unwrap();
    let (steps, applied) = db.edit_history(5).unwrap();
    let back = History::from_json(&steps, applied);
    assert_eq!(back, history);
    assert!(back.can_redo(), "the undone crop can still be redone");
    assert_eq!(back.settings().crop(), [0.0, 0.0, 1.0, 1.0]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_export_never_replaces_its_original() {
    let dir = folder("original");
    let source = dir.join("photo.jpg");
    let bytes = jpeg_with_orientation(&scene(96, 64), 1);
    std::fs::write(&source, &bytes).unwrap();
    let options = ExportOptions::default();
    assert!(edit::export(&source, &source, &Settings::default(), 0, options).is_err());
    let other_case = dir.join("PHOTO.jpg");
    if other_case.exists() {
        assert!(edit::export(&source, &other_case, &Settings::default(), 0, options).is_err(), "a disk that ignores case");
    }
    #[cfg(unix)]
    {
        let link = dir.join("link.jpg");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(edit::export(&source, &link, &Settings::default(), 0, options).is_err(), "a link to the original");
    }
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp")).collect();
    assert!(leftovers.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Real photos from `PICASSO_CORPUS`, when set: each must be read, decode
/// upright to the size its probe reported, and make a thumbnail.
#[test]
fn real_photos_in_picasso_corpus_read_and_decode() {
    let Some(root) = std::env::var_os("PICASSO_CORPUS") else { return };
    let mut checked = 0;
    let mut failed = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap().flatten() {
        let path = entry.path();
        if !path.is_file() || !crate::types::is_media_path(&path) || crate::types::is_video_path(&path) {
            continue;
        }
        let name = path.display().to_string();
        let started = std::time::Instant::now();
        let result = (|| -> Result<String, String> {
            let probe = image_io::probe(&path, 0)?;
            let image = image_io::load_oriented(&path)?;
            if (image.width(), image.height()) != (probe.width, probe.height) {
                return Err(format!("probe says {}×{}, decode gives {}×{}", probe.width, probe.height, image.width(), image.height()));
            }
            image_io::load_for_tile(&path, 256)?;
            Ok(format!("{}×{} {}", probe.width, probe.height, probe.camera))
        })();
        match result {
            Ok(summary) => println!("ok   {name}: {summary} in {:?}", started.elapsed()),
            Err(err) => {
                println!("FAIL {name}: {err}");
                failed.push(name);
            }
        }
        checked += 1;
    }
    println!("{checked} files checked");
    assert!(failed.is_empty(), "{} of {checked} files failed", failed.len());
}
