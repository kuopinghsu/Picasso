//! Picasso — a local photo library and viewer.

// Release builds on Windows open no console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod backup;
#[cfg(test)]
mod bench;
mod burst;
mod config;
#[cfg(test)]
mod corpus;
mod db;
mod edit;
mod elevation;
mod geo;
mod geocode;
mod geotag;
mod gutenberg;
mod i18n;
mod image_io;
mod inline;
mod keychain;
mod liberty;
mod mvt;
mod photo_info;
mod picasa_ini;
mod publish;
mod raw_tiff;
mod rename;
mod secret;
mod shell;
mod story_html;
mod thumbs;
mod track;
mod track_edit;
mod trip;
mod trip_package;
mod types;
mod update;
mod upload;
mod video;
mod wordpress;
mod worker;
mod world;

/// Edge of the application icon unpacked by `build.rs`.
pub const ICON_SIZE: usize = 256;
/// Straight-alpha RGBA pixels of the application icon.
pub static ICON_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon-256.rgba"));

fn main() -> eframe::Result {
    raise_open_file_limit();
    let args: Vec<std::path::PathBuf> = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect();
    if args.first().is_some_and(|arg| arg.as_os_str() == "geotag") {
        std::process::exit(geotag::cli(&args[1..]));
    }
    if args
        .first()
        .is_some_and(|arg| arg.as_os_str() == "--version")
    {
        attach_console();
        let build = [env!("PICASSO_BUILD_DATE"), env!("PICASSO_COMMIT")].join(" ");
        println!(
            "Picasso {} ({})",
            env!("CARGO_PKG_VERSION"),
            build.trim_end()
        );
        return Ok(());
    }
    prefer_wsl_gl();
    let renderer = match std::env::var("PICASSO_RENDERER").ok().as_deref() {
        Some("wgpu") => eframe::Renderer::Wgpu,
        Some("glow") | None => eframe::Renderer::Glow,
        Some(other) => {
            eprintln!("Picasso: unknown PICASSO_RENDERER={other}, using OpenGL.");
            eframe::Renderer::Glow
        }
    };
    run(&args, renderer)
}

/// A release build on Windows has no console of its own; the command line
/// output goes to the console it was started from.
#[cfg(windows)]
fn attach_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: AttachConsole takes a plain process id and fails harmlessly
    // when there is no console or one is attached already.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

/// Lets the program have as many files open at once as the system allows,
/// up to 10,240, once. macOS starts an app from Finder or the Dock, and some
/// shells start commands, with a limit of 256. The test suite, which opens
/// many libraries and photos at once in one process, ran out of it, and
/// SQLite then reported "unable to open database file". Called before
/// anything is opened, and again by [`db::Database::open`], which the tests
/// reach without `main`.
pub fn raise_open_file_limit() {
    #[cfg(unix)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: getrlimit only writes `limit`, which lives for the call.
            if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
                return;
            }
            let reached = raised_file_limit(limit.rlim_cur, limit.rlim_max, |soft| {
                let raised = libc::rlimit {
                    rlim_cur: soft,
                    rlim_max: limit.rlim_max,
                };
                // SAFETY: setrlimit only reads `raised`, which lives for the
                // call, and leaves the limit as it was when it refuses.
                unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) == 0 }
            });
            if reached < 1_024 {
                eprintln!(
                    "Picasso can have only {reached} files open at once; opening photos may fail."
                );
            }
        });
    }
}

/// Raises a soft limit on open files of `current`, under a hard limit of
/// `hard`, through `try_set`, which sets the soft limit it is given or
/// refuses and leaves it. Returns the limit it ends at: the most `try_set`
/// accepts, up to 10,240, or `current` if it accepts nothing higher.
#[cfg(unix)]
fn raised_file_limit(
    current: libc::rlim_t,
    hard: libc::rlim_t,
    mut try_set: impl FnMut(libc::rlim_t) -> bool,
) -> libc::rlim_t {
    // macOS refuses more than OPEN_MAX, 10,240, even when the hard limit is
    // unlimited.
    const MOST: libc::rlim_t = 10_240;
    let wanted = hard.min(MOST);
    if current >= wanted || try_set(wanted) {
        return current.max(wanted);
    }
    // macOS also refuses more than kern.maxfilesperproc, which can be set
    // lower. The most it accepts lies between the current limit, accepted,
    // and `wanted`, refused; each attempt halves the gap, and the limit set
    // is the last accepted.
    let (mut accepted, mut refused) = (current, wanted);
    while refused - accepted > 1 {
        let soft = accepted + (refused - accepted) / 2;
        if try_set(soft) {
            accepted = soft;
        } else {
            refused = soft;
        }
    }
    accepted
}

/// WSLg exposes the GPU through `/dev/dxg`. Its Wayland EGL path binds the
/// dummy `/dev/dri` device, then the compositor closes the socket
/// (`Broken pipe` / `WinitEventLoop`). Xwayland draws successfully.
fn prefer_wsl_gl() {
    if !std::path::Path::new("/dev/dxg").exists() {
        return;
    }
    let display = std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty());
    if display {
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("WAYLAND_SOCKET");
    }
    if std::env::var_os("GALLIUM_DRIVER").is_none() {
        std::env::set_var("GALLIUM_DRIVER", "d3d12");
    }
    if std::env::var_os("MESA_LOADER_DRIVER_OVERRIDE").is_none() {
        std::env::set_var("MESA_LOADER_DRIVER_OVERRIDE", "d3d12");
    }
}

fn run(args: &[std::path::PathBuf], renderer: eframe::Renderer) -> eframe::Result {
    let args = args.to_vec();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([880.0, 560.0])
            .with_title("Picasso")
            // Matches picasso.desktop, so Linux docks show the right icon.
            .with_app_id("picasso")
            .with_icon(eframe::egui::IconData {
                rgba: ICON_RGBA.to_vec(),
                width: ICON_SIZE as u32,
                height: ICON_SIZE as u32,
            })
            .with_drag_and_drop(true),
        renderer,
        system_font_fallback: false,
        ..Default::default()
    };
    eframe::run_native(
        "Picasso",
        options,
        Box::new(move |cc| {
            cc.egui_ctx
                .add_font_provider(std::sync::Arc::new(SystemFonts::default()));
            Ok(app::start(&cc.egui_ctx, args))
        }),
    )
}

/// egui's fallback to the system's fonts, which draws Chinese, Japanese,
/// and other scripts the built-in fonts lack. Where fontconfig has no setup
/// or no fonts, such as a bare container or a minimal server, it panics
/// while listing them; text then sticks to the built-in fonts instead.
#[derive(Default)]
struct SystemFonts {
    inner: egui_system_font_provider::SystemFontProvider,
    failed: std::sync::atomic::AtomicBool,
}

impl SystemFonts {
    fn guarded<T>(&self, empty: T, find: impl FnOnce() -> T) -> T {
        use std::sync::atomic::Ordering;
        if self.failed.load(Ordering::Relaxed) {
            return empty;
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(find)).unwrap_or_else(|_| {
            self.failed.store(true, Ordering::Relaxed);
            eprintln!("Picasso: the system's fonts could not be listed, so only the built-in fonts are used.");
            empty
        })
    }
}

impl eframe::egui::epaint::text::FontProvider for SystemFonts {
    fn fonts_for_family(
        &self,
        family: &eframe::egui::FontFamily,
    ) -> Vec<eframe::egui::epaint::text::FontInsert> {
        self.guarded(Vec::new(), || self.inner.fonts_for_family(family))
    }

    fn font_for(
        &self,
        request: &eframe::egui::epaint::text::FallbackRequest<'_>,
    ) -> Option<eframe::egui::epaint::text::FontInsert> {
        self.guarded(None, || self.inner.font_for(request))
            .or_else(|| symbol_font(request))
    }
}

/// Fonts with symbols such as ★ and ☆ that the system lookup misses: it
/// tries only the generic families for a character of no script, and on
/// Windows none of those has the star, so the star drew as an empty box
/// unless a Chinese font happened to be loaded for other text.
fn symbol_font_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        files.push(
            std::path::Path::new(&windir)
                .join("Fonts")
                .join("seguisym.ttf"),
        );
    } else if cfg!(target_os = "macos") {
        files.push("/System/Library/Fonts/Apple Symbols.ttf".into());
    } else {
        for dir in [
            "/usr/share/fonts/truetype/dejavu",
            "/usr/share/fonts/TTF",
            "/usr/share/fonts/dejavu",
        ] {
            files.push(std::path::Path::new(dir).join("DejaVuSans.ttf"));
        }
        for dir in [
            "/usr/share/fonts/truetype/noto",
            "/usr/share/fonts/noto",
            "/usr/share/fonts/google-noto",
        ] {
            files.push(std::path::Path::new(dir).join("NotoSansSymbols2-Regular.ttf"));
        }
    }
    files
}

/// The first of [`symbol_font_files`] with a glyph for the request's first
/// character. egui asks once per character and family, so a file is read
/// only for the few characters no other font has.
fn symbol_font(
    request: &eframe::egui::epaint::text::FallbackRequest<'_>,
) -> Option<eframe::egui::epaint::text::FontInsert> {
    use ab_glyph::Font as _;
    use eframe::egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
    let c = request.cluster.chars().next()?;
    symbol_font_files().into_iter().find_map(|path| {
        let bytes = std::fs::read(&path).ok()?;
        let has =
            ab_glyph::FontRef::try_from_slice(&bytes).is_ok_and(|font| font.glyph_id(c).0 != 0);
        has.then(|| FontInsert {
            name: format!("symbols:{}", path.display()),
            data: eframe::egui::FontData::from_owned(bytes),
            families: vec![InsertFontFamily {
                family: request.family.clone(),
                priority: FontPriority::Lowest,
            }],
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The soft limits `raised_file_limit` reaches from 256 under `hard`
    /// on a system that accepts up to `system`, and the limit it leaves set.
    #[cfg(unix)]
    fn raise_from_256(hard: libc::rlim_t, system: libc::rlim_t) -> (libc::rlim_t, libc::rlim_t) {
        let mut set = 256;
        let reached = raised_file_limit(256, hard, |soft| {
            let accepted = soft <= system && soft <= hard;
            if accepted {
                set = soft;
            }
            accepted
        });
        (reached, set)
    }

    #[cfg(unix)]
    #[test]
    fn the_open_file_limit_rises_to_the_most_the_system_accepts() {
        // macOS's unlimited hard limit, as far as OPEN_MAX.
        assert_eq!(
            raise_from_256(libc::RLIM_INFINITY, 1 << 20),
            (10_240, 10_240)
        );
        // A lower hard limit, as on Linux.
        assert_eq!(raise_from_256(4_096, 1 << 20), (4_096, 4_096));
        // kern.maxfilesperproc set below 10,240: all it allows, not half.
        assert_eq!(raise_from_256(libc::RLIM_INFINITY, 1_200), (1_200, 1_200));
        // Nothing higher allowed: the limit stays.
        assert_eq!(raise_from_256(libc::RLIM_INFINITY, 256), (256, 256));
        assert_eq!(raise_from_256(256, 1 << 20), (256, 256));
        // Already higher: never lowered.
        assert_eq!(
            raised_file_limit(65_536, libc::RLIM_INFINITY, |_| panic!("set")),
            65_536
        );
    }

    #[test]
    fn the_star_has_a_font_where_the_system_has_a_symbol_font() {
        if !symbol_font_files().iter().any(|path| path.exists()) {
            return;
        }
        let family = eframe::egui::FontFamily::Proportional;
        for star in ["★", "☆"] {
            let request = eframe::egui::epaint::text::FallbackRequest {
                cluster: star,
                family: &family,
            };
            assert!(symbol_font(&request).is_some(), "no font for {star}");
        }
    }

    #[test]
    fn fonts_that_cannot_be_listed_leave_the_built_in_ones() {
        let fonts = SystemFonts::default();
        assert_eq!(fonts.guarded(1, || 2), 2);
        assert_eq!(fonts.guarded(0, || -> i32 { panic!("no fontconfig") }), 0);
        assert_eq!(
            fonts.guarded(0, || 2),
            0,
            "a failed listing is not tried again"
        );
    }
}
