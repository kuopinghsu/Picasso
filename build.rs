//! Draws the application icon and stamps the build date. Windows builds also
//! get the icon and version details embedded in the executable.

#[path = "build/icon.rs"]
mod icon;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    std::fs::write(out.join("icon-256.rgba"), icon::render(256)).expect("write icon pixels");
    println!("cargo:rustc-env=PICASSO_BUILD_DATE={}", build_date());

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let ico = out.join("picasso.ico");
        std::fs::write(&ico, ico_file(&ICO_SIZES)).expect("write icon file");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico.to_string_lossy());
        resource.set("ProductName", "Picasso");
        resource.set("FileDescription", "Picasso");
        if let Err(err) = resource.compile() {
            println!("cargo:warning=The Windows icon was not embedded: {err}");
        }
    }
}

/// UTC date as `YYYY.MM.DD`, from `SOURCE_DATE_EPOCH` for reproducible builds.
fn build_date() -> String {
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs()));
    let (year, month, day) = civil_date(seconds / 86_400);
    format!("{year}.{month:02}.{day:02}")
}

/// Days since 1970-01-01 to a Gregorian date (Howard Hinnant's algorithm).
fn civil_date(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// Windows `.ico` with one 32-bit bitmap per size.
fn ico_file(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| bitmap(size, &icon::render(size))).collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, image) in sizes.iter().zip(&images) {
        let edge = if size >= 256 { 0 } else { size as u8 };
        out.extend_from_slice(&[edge, edge, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in images {
        out.extend(image);
    }
    out
}

/// Icon entry: bottom-up BGRA rows after a header that counts the AND mask
/// in the height, then an all-clear AND mask.
fn bitmap(size: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in [40, size, size * 2] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..size).rev() {
        let row = &rgba[(y * size * 4) as usize..((y + 1) * size * 4) as usize];
        for pixel in row.chunks_exact(4) {
            out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    let mask_row = size.div_ceil(32) as usize * 4;
    out.resize(out.len() + mask_row * size as usize, 0);
    out
}
