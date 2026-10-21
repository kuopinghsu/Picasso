//! Picasso — a local photo library and viewer.

// Release builds on Windows open no console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod db;
mod edit;
mod geo;
mod i18n;
mod image_io;
mod photo_info;
mod picasa_ini;
mod types;
mod video;
mod worker;

/// Edge of the application icon unpacked by `build.rs`.
pub const ICON_SIZE: usize = 256;
/// Straight-alpha RGBA pixels of the application icon.
pub static ICON_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon-256.rgba"));

fn main() -> eframe::Result {
    let args: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(std::path::PathBuf::from).collect();
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
            .with_icon(eframe::egui::IconData {
                rgba: ICON_RGBA.to_vec(),
                width: ICON_SIZE as u32,
                height: ICON_SIZE as u32,
            })
            .with_drag_and_drop(true),
        renderer,
        ..Default::default()
    };
    eframe::run_native(
        "Picasso",
        options,
        Box::new(move |cc| Ok(Box::new(app::PicassoApp::new(&cc.egui_ctx, args)))),
    )
}
