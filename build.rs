//! Unpacks the application icon from `build/picasso.ico` and stamps the build
//! date and commit. Windows builds also get the icon and version details embedded in the
//! executable.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const ICON_SIZE: u32 = 256;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"));
    let ico = manifest.join("build").join("picasso.ico");
    let bytes = std::fs::read(&ico).unwrap_or_else(|err| panic!("read {}: {err}", ico.display()));
    std::fs::write(out.join("icon-256.rgba"), ico_rgba(&bytes, ICON_SIZE)).expect("write icon pixels");
    println!("cargo:rustc-env=PICASSO_BUILD_DATE={}", build_date());
    println!("cargo:rustc-env=PICASSO_COMMIT={}", commit(&manifest));

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico.to_string_lossy());
        resource.set("ProductName", "Picasso");
        resource.set("FileDescription", "Picasso");
        if let Err(err) = resource.compile() {
            println!("cargo:warning=The Windows icon was not embedded: {err}");
        }
    }
}

/// Straight-alpha RGBA pixels of the `size` × `size` entry in an `.ico`,
/// which must be stored as a 32-bit bitmap (bottom-up BGRA rows).
fn ico_rgba(ico: &[u8], size: u32) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([ico[at], ico[at + 1], ico[at + 2], ico[at + 3]]);
    let edge = |byte: u8| if byte == 0 { 256 } else { u32::from(byte) };
    let entry = (0..usize::from(u16_at(4)))
        .map(|index| 6 + 16 * index)
        .find(|&at| edge(ico[at]) == size && edge(ico[at + 1]) == size)
        .unwrap_or_else(|| panic!("picasso.ico has no {size}x{size} image"));
    let start = u32_at(entry + 12) as usize;
    let image = &ico[start..];
    assert!(
        u32_at(start) == 40 && u16_at(start + 14) == 32,
        "the {size}x{size} image in picasso.ico must be a 32-bit bitmap, not PNG"
    );
    let header = u32_at(start) as usize;
    let row = (size * 4) as usize;
    let mut out = Vec::with_capacity(row * size as usize);
    for y in (0..size as usize).rev() {
        let line = &image[header + y * row..header + (y + 1) * row];
        for pixel in line.chunks_exact(4) {
            out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    out
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

/// Short hash of the commit being built, so a package can be traced to its
/// source: `SOURCE_COMMIT` or `GITHUB_SHA` in CI, else asked of git with
/// `-dirty` added when tracked files were changed, else empty (a source
/// archive). The build script runs again when a tracked file, the branch,
/// or its latest commit changes, so `-dirty` comes and goes with the edits.
fn commit(manifest: &std::path::Path) -> String {
    for path in ["src", "build", "build.rs", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={path}");
    }
    for name in ["SOURCE_COMMIT", "GITHUB_SHA", "SOURCE_DATE_EPOCH"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(manifest)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        let dir = PathBuf::from(dir);
        for path in [dir.join("HEAD"), dir.join("refs").join("heads"), dir.join("packed-refs")] {
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
        // A tracked file that is gone is never current, which is right:
        // the build differs from the commit until it is back.
        // -z gives names as they are, where plain output quotes those with
        // letters outside ASCII, which would name no file.
        for path in git(&["ls-files", "-z"]).unwrap_or_default().split('\0').filter(|path| !path.is_empty()) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    // A pull request's GITHUB_SHA is the merge GitHub tests, which is in no
    // branch; CI passes the pull request's own commit as SOURCE_COMMIT.
    for name in ["SOURCE_COMMIT", "GITHUB_SHA"] {
        if let Some(sha) = std::env::var(name).ok().filter(|sha| !sha.is_empty()) {
            return sha.chars().take(7).collect();
        }
    }
    let Some(sha) = git(&["rev-parse", "HEAD"]) else { return String::new() };
    let short: String = sha.chars().take(7).collect();
    // Tracked files that differ from the commit make a build no commit names.
    let changed = std::process::Command::new("git")
        .args(["diff", "HEAD", "--quiet"])
        .current_dir(manifest)
        .status()
        .is_ok_and(|status| status.code() == Some(1));
    if changed {
        format!("{short}-dirty")
    } else {
        short
    }
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
