//! Picasso — a local photo library and viewer.

// Release builds on Windows open no console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod backup;
#[cfg(test)]
mod bench;
mod config;
#[cfg(test)]
mod corpus;
mod db;
mod edit;
mod geo;
mod geotag;
mod i18n;
mod image_io;
mod liberty;
mod mvt;
mod photo_info;
mod picasa_ini;
mod raw_tiff;
mod rename;
mod secret;
mod shell;
mod thumbs;
mod track;
mod track_edit;
mod trip;
mod types;
mod update;
mod video;
mod worker;
mod world;

/// Edge of the application icon unpacked by `build.rs`.
pub const ICON_SIZE: usize = 256;
/// Straight-alpha RGBA pixels of the application icon.
pub static ICON_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon-256.rgba"));

fn main() -> eframe::Result {
    let args: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(std::path::PathBuf::from).collect();
    if args.first().is_some_and(|arg| arg.as_os_str() == "geotag") {
        std::process::exit(geotag::cli(&args[1..]));
    }
    if args.first().is_some_and(|arg| arg.as_os_str() == "--version") {
        attach_console();
        let build = [env!("PICASSO_BUILD_DATE"), env!("PICASSO_COMMIT")].join(" ");
        println!("Picasso {} ({})", env!("CARGO_PKG_VERSION"), build.trim_end());
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
            cc.egui_ctx.add_font_provider(std::sync::Arc::new(SystemFonts::default()));
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
    fn fonts_for_family(&self, family: &eframe::egui::FontFamily) -> Vec<eframe::egui::epaint::text::FontInsert> {
        self.guarded(Vec::new(), || self.inner.fonts_for_family(family))
    }

    fn font_for(&self, request: &eframe::egui::epaint::text::FallbackRequest<'_>) -> Option<eframe::egui::epaint::text::FontInsert> {
        self.guarded(None, || self.inner.font_for(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_that_cannot_be_listed_leave_the_built_in_ones() {
        let fonts = SystemFonts::default();
        assert_eq!(fonts.guarded(1, || 2), 2);
        assert_eq!(fonts.guarded(0, || -> i32 { panic!("no fontconfig") }), 0);
        assert_eq!(fonts.guarded(0, || 2), 0, "a failed listing is not tried again");
    }
}
