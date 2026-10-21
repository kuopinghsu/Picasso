//! OpenStreetMap tiles and Web Mercator conversion for the Places view.
//!
//! Tiles are cached on disk and requested one at a time. The User-Agent
//! identifies this local viewer, per the OpenStreetMap tile policy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};

pub const TILE_PX: f64 = 256.0;

#[derive(Clone, Copy, Debug)]
pub struct TileJob {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

#[derive(Debug)]
pub struct TileDone {
    pub z: u8,
    pub x: u32,
    pub y: u32,
    pub image: Option<(usize, usize, Vec<u8>)>,
}

pub fn latlon_to_world(lat: f64, lon: f64, zoom: u8) -> (f64, f64) {
    let lat = lat.clamp(-85.05112878, 85.05112878);
    let n = f64::from(1u32 << zoom);
    let x = (lon + 180.0) / 360.0 * n * TILE_PX;
    let lat_rad = lat.to_radians();
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n * TILE_PX;
    (x, y)
}

pub fn world_to_latlon(x: f64, y: f64, zoom: u8) -> (f64, f64) {
    let n = f64::from(1u32 << zoom) * TILE_PX;
    let lon = (x / n) * 360.0 - 180.0;
    let merc = std::f64::consts::PI * (1.0 - 2.0 * y / n);
    let lat = merc.sinh().atan().to_degrees();
    (lat.clamp(-85.05112878, 85.05112878), lon.clamp(-180.0, 180.0))
}

pub fn tile_loop(jobs: Receiver<TileJob>, done: Sender<TileDone>, cache: PathBuf) {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .user_agent("Picasso/0.1 (local photo library; geotag map)")
        .build();
    let agent = ureq::Agent::new_with_config(config);
    while let Ok(job) = jobs.recv() {
        let image = load_tile(&agent, &cache, job.z, job.x, job.y);
        if done.send(TileDone { z: job.z, x: job.x, y: job.y, image }).is_err() {
            break;
        }
    }
}

fn load_tile(agent: &ureq::Agent, cache: &Path, z: u8, x: u32, y: u32) -> Option<(usize, usize, Vec<u8>)> {
    let path = cache.join("tiles").join(z.to_string()).join(x.to_string()).join(format!("{y}.png"));
    let bytes = if let Ok(cached) = std::fs::read(&path) {
        if cached.len() > 32 {
            cached
        } else {
            fetch_tile(agent, &path, z, x, y)?
        }
    } else {
        fetch_tile(agent, &path, z, x, y)?
    };
    let image = image::load_from_memory(&bytes).ok()?;
    let rgba = image.to_rgba8();
    Some((rgba.width() as usize, rgba.height() as usize, rgba.into_raw()))
}

fn fetch_tile(agent: &ureq::Agent, path: &Path, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
    let url = format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png");
    let mut response = agent.get(&url).call().ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let bytes = response.body_mut().read_to_vec().ok()?;
    if bytes.len() < 32 {
        return None;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, &bytes);
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_is_the_middle_of_zoom_zero() {
        let (x, y) = latlon_to_world(0.0, 0.0, 0);
        assert!((x - 128.0).abs() < 0.01);
        assert!((y - 128.0).abs() < 0.01);
        let (lat, lon) = world_to_latlon(x, y, 0);
        assert!(lat.abs() < 1e-6);
        assert!(lon.abs() < 1e-6);
    }
}
