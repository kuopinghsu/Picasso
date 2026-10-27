//! Map tiles and Web Mercator conversion for Places and the Map panel.
//!
//! Tiles are cached on disk, one folder per map source. OpenFreeMap sends
//! vector data, which `liberty` draws; the other sources send pictures. The
//! User-Agent identifies this local viewer, as the tile policies ask.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};

use crate::liberty::{self, MapLabel};

pub const TILE_PX: f64 = 256.0;

/// Where map tiles come from, picked in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MapSource {
    #[default]
    OpenFreeMap,
    OpenStreetMap,
    CartoVoyager,
    CartoPositron,
    CartoDarkMatter,
    EsriImagery,
}

impl MapSource {
    pub const ALL: [MapSource; 6] = [
        MapSource::OpenFreeMap,
        MapSource::OpenStreetMap,
        MapSource::CartoVoyager,
        MapSource::CartoPositron,
        MapSource::CartoDarkMatter,
        MapSource::EsriImagery,
    ];

    /// The CARTO styles, which Settings offers as one source.
    pub const CARTO_STYLES: [MapSource; 3] =
        [MapSource::CartoVoyager, MapSource::CartoPositron, MapSource::CartoDarkMatter];

    pub fn code(self) -> &'static str {
        match self {
            MapSource::OpenFreeMap => "openfreemap",
            MapSource::OpenStreetMap => "openstreetmap",
            MapSource::CartoVoyager => "carto-voyager",
            MapSource::CartoPositron => "carto-positron",
            MapSource::CartoDarkMatter => "carto-dark-matter",
            MapSource::EsriImagery => "esri-imagery",
        }
    }

    pub fn from_code(code: &str) -> Option<MapSource> {
        MapSource::ALL.into_iter().find(|source| source.code() == code)
    }

    /// Product name, the same in every language.
    pub fn name(self) -> &'static str {
        match self {
            MapSource::OpenFreeMap => "OpenFreeMap Liberty",
            MapSource::OpenStreetMap => "OpenStreetMap",
            MapSource::CartoVoyager => "CARTO Voyager",
            MapSource::CartoPositron => "CARTO Positron",
            MapSource::CartoDarkMatter => "CARTO Dark Matter",
            MapSource::EsriImagery => "Esri World Imagery",
        }
    }

    /// Name of a CARTO style without the provider.
    pub fn style_name(self) -> &'static str {
        self.name().trim_start_matches("CARTO ")
    }

    /// Credit the provider asks to show on the map.
    pub fn attribution(self) -> &'static str {
        match self {
            MapSource::OpenFreeMap => "OpenFreeMap © OpenMapTiles · Data © OpenStreetMap contributors",
            MapSource::OpenStreetMap => "© OpenStreetMap contributors",
            MapSource::CartoVoyager | MapSource::CartoPositron | MapSource::CartoDarkMatter => {
                "© CARTO · © OpenStreetMap contributors"
            }
            MapSource::EsriImagery => "Tiles © Esri · Esri, Maxar, Earthstar Geographics, and the GIS User Community",
        }
    }

    /// CARTO serves its tiles only with a (free) API key.
    pub fn needs_key(self) -> bool {
        matches!(self, MapSource::CartoVoyager | MapSource::CartoPositron | MapSource::CartoDarkMatter)
    }

    /// Folder of this source's tiles under the tile cache. OpenStreetMap
    /// keeps the folder earlier versions filled.
    pub fn cache_dir(self, tiles: &Path) -> PathBuf {
        match self {
            MapSource::OpenStreetMap => tiles.to_path_buf(),
            other => tiles.join(other.code()),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            MapSource::OpenFreeMap => "pbf",
            MapSource::EsriImagery => "jpg",
            MapSource::OpenStreetMap | MapSource::CartoVoyager | MapSource::CartoPositron | MapSource::CartoDarkMatter => "png",
        }
    }

    fn picture_url(self, z: u8, x: u32, y: u32, key: &str) -> String {
        match self {
            MapSource::OpenStreetMap | MapSource::OpenFreeMap => format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png"),
            MapSource::CartoVoyager | MapSource::CartoPositron | MapSource::CartoDarkMatter => {
                let style = match self {
                    MapSource::CartoVoyager => "voyager",
                    MapSource::CartoDarkMatter => "dark_all",
                    _ => "light_all",
                };
                format!("https://basemaps.cartocdn.com/rastertiles/{style}/{z}/{x}/{y}@2x.png?key={}", key.trim())
            }
            // Without `blankTile=false`, places without imagery this close
            // come back as a "Map data not yet available" picture.
            MapSource::EsriImagery => format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}?blankTile=false"
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TileJob {
    pub source: MapSource,
    /// API key for sources that need one.
    pub key: String,
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

/// A loaded tile: straight RGBA pixels, and the names to write over it.
#[derive(Debug)]
pub struct TileImage {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub labels: Vec<MapLabel>,
}

#[derive(Debug)]
pub struct TileDone {
    pub source: MapSource,
    pub z: u8,
    pub x: u32,
    pub y: u32,
    pub image: Option<TileImage>,
}

pub fn latlon_to_world(lat: f64, lon: f64, zoom: u8) -> (f64, f64) {
    let lat = lat.clamp(-85.05112878, 85.05112878);
    let n = f64::from(1u32 << zoom);
    let x = (lon + 180.0) / 360.0 * n * TILE_PX;
    let lat_rad = lat.to_radians();
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n * TILE_PX;
    (x, y)
}

/// Ground distance one point of a 256 px tile covers at `lat` and `zoom`.
pub fn meters_per_point(lat: f64, zoom: u8) -> f64 {
    const EQUATOR: f64 = 40_075_016.686;
    EQUATOR * lat.to_radians().cos() / (f64::from(1u32 << zoom) * TILE_PX)
}

pub fn world_to_latlon(x: f64, y: f64, zoom: u8) -> (f64, f64) {
    let n = f64::from(1u32 << zoom) * TILE_PX;
    let lon = (x / n) * 360.0 - 180.0;
    let merc = std::f64::consts::PI * (1.0 - 2.0 * y / n);
    let lat = merc.sinh().atan().to_degrees();
    (lat.clamp(-85.05112878, 85.05112878), lon.clamp(-180.0, 180.0))
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .user_agent(concat!("Picasso/", env!("CARGO_PKG_VERSION"), " (local photo library; geotag map)"))
        .build();
    ureq::Agent::new_with_config(config)
}

pub fn tile_loop(jobs: Receiver<TileJob>, done: Sender<TileDone>, cache: PathBuf) {
    let agent = agent();
    let tiles = cache.join("tiles");
    while let Ok(job) = jobs.recv() {
        let image = match job.source {
            MapSource::OpenFreeMap => vector_tile(&agent, &tiles, &job),
            _ => picture_tile(&agent, &tiles, &job),
        };
        let reply = TileDone { source: job.source, z: job.z, x: job.x, y: job.y, image };
        if done.send(reply).is_err() {
            break;
        }
    }
}

fn picture_tile(agent: &ureq::Agent, tiles: &Path, job: &TileJob) -> Option<TileImage> {
    let path = job
        .source
        .cache_dir(tiles)
        .join(job.z.to_string())
        .join(job.x.to_string())
        .join(format!("{}.{}", job.y, job.source.extension()));
    let bytes = match std::fs::read(&path) {
        Ok(cached) if cached.len() > 32 => cached,
        _ => {
            let bytes = fetch(agent, &job.source.picture_url(job.z, job.x, job.y, &job.key))?;
            if bytes.len() < 32 {
                return None;
            }
            store(&path, &bytes);
            bytes
        }
    };
    let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
    Some(TileImage {
        w: rgba.width() as usize,
        h: rgba.height() as usize,
        rgba: rgba.into_raw(),
        labels: Vec::new(),
    })
}

fn vector_tile(agent: &ureq::Agent, tiles: &Path, job: &TileJob) -> Option<TileImage> {
    let ((z, x, y), part) = liberty::Part::of(job.z, job.x, job.y);
    let path = MapSource::OpenFreeMap.cache_dir(tiles).join(z.to_string()).join(x.to_string()).join(format!("{y}.pbf"));
    let data = match std::fs::read(&path) {
        Ok(cached) => cached,
        Err(_) => {
            let data = unzip(fetch(agent, &vector_url(agent).replace("{z}", &z.to_string()).replace("{x}", &x.to_string()).replace("{y}", &y.to_string()))?)?;
            store(&path, &data);
            data
        }
    };
    let drawn = liberty::draw(&data, job.z, part, liberty::name_keys(crate::i18n::current()))?;
    let side = liberty::TILE_SIZE as usize;
    Some(TileImage { w: side, h: side, rgba: drawn.rgba, labels: drawn.labels })
}

/// Tile address of the current OpenFreeMap data, read once from its
/// TileJSON. The service also takes any name in place of the version, which
/// is used until the TileJSON can be read.
fn vector_url(agent: &ureq::Agent) -> String {
    static TEMPLATE: Mutex<Option<String>> = Mutex::new(None);
    if let Some(template) = TEMPLATE.lock().ok().and_then(|template| template.clone()) {
        return template;
    }
    let read = fetch(agent, "https://tiles.openfreemap.org/planet")
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|json| json["tiles"][0].as_str().map(str::to_string))
        .filter(|template| template.contains("{z}") && template.starts_with("https://"));
    match read {
        Some(template) => {
            if let Ok(mut stored) = TEMPLATE.lock() {
                *stored = Some(template.clone());
            }
            template
        }
        None => "https://tiles.openfreemap.org/planet/latest/{z}/{x}/{y}.pbf".into(),
    }
}

/// Body of a successful response. An empty tile comes back as no bytes.
fn fetch(agent: &ureq::Agent, url: &str) -> Option<Vec<u8>> {
    let mut response = agent.get(url).call().ok()?;
    match response.status().as_u16() {
        200 => response.body_mut().with_config().limit(16 << 20).read_to_vec().ok(),
        204 => Some(Vec::new()),
        _ => None,
    }
}

/// Vector tiles may arrive still gzipped.
fn unzip(data: Vec<u8>) -> Option<Vec<u8>> {
    if !data.starts_with(&[0x1F, 0x8B]) {
        return Some(data);
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data.as_slice()).read_to_end(&mut out).ok()?;
    Some(out)
}

fn store(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, bytes);
}

/// The Terrain Tiles on AWS, first made by Mapzen: free and without a key,
/// with the height of each pixel coded in its color.
const TERRAIN_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png";
/// Zoom of the terrain tiles heights are read from, about 30 m a pixel in
/// Europe.
const TERRAIN_ZOOM: u8 = 12;
/// Zoom a long track's heights may be read from, about 500 m a pixel.
const TERRAIN_MIN_ZOOM: u8 = 8;
/// Most terrain tiles one lookup downloads, about 35 MB of land. A longer
/// track is read from coarser tiles, each level a quarter as many.
const TERRAIN_TILES: usize = 300;
/// Terrain tiles downloaded at once.
const TERRAIN_FETCHES: usize = 6;

/// Tiles done and tiles needed while [`terrain_heights`] runs.
#[derive(Default)]
pub struct TerrainProgress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
}

/// The terrain tiles at `zoom` that `places` fall in, each with the places
/// in it, as their index and their pixel in the tile.
type TerrainTiles = BTreeMap<(u32, u32), Vec<(usize, f64, f64)>>;

fn terrain_tiles(places: &[(f64, f64)], zoom: u8) -> TerrainTiles {
    let last = (1u32 << zoom) - 1;
    let mut tiles = TerrainTiles::new();
    for (index, &(lat, lon)) in places.iter().enumerate() {
        let (x, y) = latlon_to_world(lat, lon, zoom);
        let tile = (((x / TILE_PX) as u32).min(last), ((y / TILE_PX) as u32).min(last));
        tiles.entry(tile).or_default().push((index, x - f64::from(tile.0) * TILE_PX, y - f64::from(tile.1) * TILE_PX));
    }
    tiles
}

/// The finest zoom at which `places` need no more than [`TERRAIN_TILES`]
/// tiles, with those tiles.
fn terrain_zoom(places: &[(f64, f64)]) -> (u8, TerrainTiles) {
    let mut zoom = TERRAIN_ZOOM;
    let mut tiles = terrain_tiles(places, zoom);
    while tiles.len() > TERRAIN_TILES && zoom > TERRAIN_MIN_ZOOM {
        zoom -= 1;
        tiles = terrain_tiles(places, zoom);
    }
    (zoom, tiles)
}

/// The ground height in meters at each of `places`, as (latitude,
/// longitude), from terrain tiles kept in the tile cache. Only the numbers
/// of the tiles are sent, as when showing the map. A tile that fails is
/// tried once more; a place whose tile still can't be had gets `None`.
pub fn terrain_heights(places: &[(f64, f64)], cache: &Path, progress: &TerrainProgress) -> Vec<Option<f64>> {
    let (zoom, tiles) = terrain_zoom(places);
    let tiles: Vec<_> = tiles.into_iter().collect();
    progress.total.store(tiles.len(), Ordering::Relaxed);
    let folder = cache.join("tiles").join("terrarium").join(zoom.to_string());
    let agent = agent();
    let heights = Mutex::new(vec![None; places.len()]);
    let mut left: Vec<usize> = (0..tiles.len()).collect();
    for last_try in [false, true] {
        let next = AtomicUsize::new(0);
        let failed = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..TERRAIN_FETCHES.min(left.len()) {
                scope.spawn(|| {
                    while let Some(&job) = left.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let ((x, y), inside) = &tiles[job];
                        // Each tile is read as it comes, so only the heights are kept.
                        let got = terrain_tile(&agent, &folder, zoom, *x, *y);
                        if let Some(tile) = &got {
                            let mut heights = heights.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                            for &(index, px, py) in inside {
                                heights[index] = Some(terrain_height(tile, px, py));
                            }
                        } else {
                            failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(job);
                        }
                        if got.is_some() || last_try {
                            progress.done.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });
        left = failed.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());
        if left.is_empty() {
            break;
        }
    }
    heights.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A terrain tile from the cache, or downloaded when it isn't there or
/// doesn't decode, as a tile cut short by a crash wouldn't.
fn terrain_tile(agent: &ureq::Agent, folder: &Path, zoom: u8, x: u32, y: u32) -> Option<image::RgbaImage> {
    let path = folder.join(x.to_string()).join(format!("{y}.png"));
    let decode = |bytes: &[u8]| image::load_from_memory(bytes).ok().map(|tile| tile.to_rgba8());
    if let Some(tile) = std::fs::read(&path).ok().and_then(|bytes| decode(&bytes)) {
        return Some(tile);
    }
    let url = TERRAIN_URL.replace("{z}", &zoom.to_string()).replace("{x}", &x.to_string()).replace("{y}", &y.to_string());
    let bytes = fetch(agent, &url)?;
    let tile = decode(&bytes)?;
    let _ = crate::image_io::write_whole(&path, &bytes);
    Some(tile)
}

/// The height at (`x`, `y`) in a terrain tile, between its four nearest
/// pixels. A place on the water reads as the sea floor there, so heights
/// under sea level count as 0 m: a ferry or flight goes over the sea.
fn terrain_height(tile: &image::RgbaImage, x: f64, y: f64) -> f64 {
    let (w, h) = (tile.width() as f64, tile.height() as f64);
    // Pixel centers are half a pixel in.
    let (x, y) = ((x * w / TILE_PX - 0.5).clamp(0.0, w - 1.0), (y * h / TILE_PX - 0.5).clamp(0.0, h - 1.0));
    let (x0, y0) = (x.floor(), y.floor());
    let (x1, y1) = ((x0 + 1.0).min(w - 1.0), (y0 + 1.0).min(h - 1.0));
    let at = |px: f64, py: f64| {
        let [r, g, b, _] = tile.get_pixel(px as u32, py as u32).0;
        f64::from(r) * 256.0 + f64::from(g) + f64::from(b) / 256.0 - 32768.0
    };
    let (fx, fy) = (x - x0, y - y0);
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    (top * (1.0 - fy) + bottom * fy).max(0.0)
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

    #[test]
    fn sources_round_trip_and_keep_their_own_caches() {
        for source in MapSource::ALL {
            assert_eq!(MapSource::from_code(source.code()), Some(source));
        }
        assert_eq!(MapSource::from_code("nope"), None);
        assert_eq!(MapSource::default(), MapSource::OpenFreeMap);
        let tiles = Path::new("tiles");
        assert_eq!(MapSource::OpenStreetMap.cache_dir(tiles), tiles);
        assert_eq!(MapSource::EsriImagery.cache_dir(tiles), tiles.join("esri-imagery"));
        assert!(MapSource::EsriImagery.picture_url(3, 5, 2, "").ends_with("/tile/3/2/5?blankTile=false"), "Esri puts the row first");
        assert!(MapSource::CartoPositron.picture_url(3, 5, 2, " abc ").ends_with("/light_all/3/5/2@2x.png?key=abc"));
        assert!(MapSource::CartoVoyager.picture_url(3, 5, 2, "abc").contains("/voyager/3/5/2@2x.png"));
        assert!(MapSource::CartoDarkMatter.picture_url(3, 5, 2, "abc").contains("/dark_all/3/5/2@2x.png"));
        assert!(MapSource::ALL.iter().all(|source| MapSource::from_code(source.code()) == Some(*source)));
        assert!(MapSource::CARTO_STYLES.iter().all(|style| style.needs_key()));
        assert_eq!(MapSource::CartoDarkMatter.style_name(), "Dark Matter");
    }

    #[test]
    fn terrain_colors_are_heights() {
        // 1000 m is 33768 = 131 * 256 + 232; 1100 m is 33868 = 132 * 256 + 76.
        let mut tile = image::RgbaImage::from_pixel(256, 256, image::Rgba([131, 232, 0, 255]));
        for y in 0..256 {
            tile.put_pixel(129, y, image::Rgba([132, 76, 0, 255]));
        }
        assert_eq!(terrain_height(&tile, 10.5, 10.5), 1000.0);
        // Halfway between the centers of pixels 128 and 129.
        assert!((terrain_height(&tile, 129.0, 50.0) - 1050.0).abs() < 1e-9);
        assert_eq!(terrain_height(&tile, 129.5, 255.9), 1100.0);
        // The sea floor counts as sea level.
        let sea = image::RgbaImage::from_pixel(256, 256, image::Rgba([120, 0, 0, 255]));
        assert_eq!(terrain_height(&sea, 100.0, 100.0), 0.0);
    }

    #[test]
    fn terrain_comes_from_the_cache_without_a_network() {
        let cache = std::env::temp_dir().join(format!("picasso-terrain-{}", std::process::id()));
        let (x, y) = latlon_to_world(25.0336, 121.5645, TERRAIN_ZOOM);
        let (tx, ty) = ((x / TILE_PX) as u32, (y / TILE_PX) as u32);
        let path = cache.join("tiles/terrarium/12").join(tx.to_string()).join(format!("{ty}.png"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(256, 256, image::Rgba([128, 10, 0, 255])).save(&path).unwrap();
        let progress = TerrainProgress::default();
        assert_eq!(terrain_heights(&[(25.0336, 121.5645)], &cache, &progress), vec![Some(10.0)]);
        assert_eq!((progress.done.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed)), (1, 1));
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn long_tracks_read_coarser_terrain() {
        // A walk round a city stays at the finest tiles.
        let city: Vec<(f64, f64)> = (0..1000).map(|step| (41.38 + step as f64 * 1e-5, 2.17)).collect();
        assert_eq!(terrain_zoom(&city).0, TERRAIN_ZOOM);
        // A drive from Lisbon to Moscow needs too many of them.
        let drive: Vec<(f64, f64)> =
            (0..=4000).map(|step| (38.7 + (55.75 - 38.7) * step as f64 / 4000.0, -9.14 + (37.62 + 9.14) * step as f64 / 4000.0)).collect();
        let (zoom, tiles) = terrain_zoom(&drive);
        assert!(zoom < TERRAIN_ZOOM && tiles.len() <= TERRAIN_TILES, "{zoom} {}", tiles.len());
        assert_eq!(tiles.values().map(Vec::len).sum::<usize>(), drive.len());
    }

    #[test]
    fn gzipped_tiles_are_unpacked() {
        use std::io::Write;
        let mut packed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        packed.write_all(b"vector tile").unwrap();
        assert_eq!(unzip(packed.finish().unwrap()).unwrap(), b"vector tile");
        assert_eq!(unzip(b"plain".to_vec()).unwrap(), b"plain");
    }
}
