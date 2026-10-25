//! Countries and the states or provinces of the largest ones, for the
//! Travel Map: names, outlines, fill triangles, which one a place is in, and
//! the two map projections. The shapes are Natural Earth's public-domain
//! 1:50m data, packed by `scripts/world-map.py`.

use std::f32::consts::{FRAC_PI_2, PI};
use std::io::Read;
use std::sync::OnceLock;

use crate::i18n::{self, Lang};

static PACKED: &[u8] = include_bytes!("../build/world.bin");

/// How far off a coastline, in degrees, a photo still counts for the
/// nearest land: the 1:50m outlines cut off small islands and piers.
const NEAR: f32 = 0.3;

/// The groups the Travel Map counts continents in. Antarctica goes with
/// Oceania.
pub const CONTINENTS: [&str; 5] = ["Africa", "Americas", "Asia", "Europe", "Oceania & Antarctica"];

pub struct Area {
    /// ISO 3166 code: "TW" for a country, "US_CA" for a state.
    pub id: String,
    /// The country of a state or province.
    pub parent: Option<usize>,
    /// Index into `CONTINENTS`.
    pub continent: usize,
    /// One per language, in `Lang::ALL` order.
    names: Vec<String>,
    /// Longitude and latitude to center the map on for it.
    pub label: [f32; 2],
    /// Longitude and latitude of each point, in degrees.
    pub points: Vec<[f32; 2]>,
    /// Start and length in `points` of each outline.
    pub rings: Vec<(u32, u32)>,
    ring_bounds: Vec<Bounds>,
    /// Fill triangles, as three indices into `points` each.
    pub triangles: Vec<u32>,
    pub bounds: Bounds,
    /// `points` in the flat projection.
    pub flat: Vec<[f32; 2]>,
    /// Smallest and largest x and y of `flat`.
    pub flat_bounds: [f32; 4],
    /// `points` on the unit sphere, for the globe.
    pub sphere: Vec<[f32; 3]>,
}

/// Longitude and latitude extent, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub west: f32,
    pub south: f32,
    pub east: f32,
    pub north: f32,
}

impl Bounds {
    const EMPTY: Bounds = Bounds {
        west: f32::INFINITY,
        south: f32::INFINITY,
        east: f32::NEG_INFINITY,
        north: f32::NEG_INFINITY,
    };

    fn add(&mut self, [lon, lat]: [f32; 2]) {
        self.west = self.west.min(lon);
        self.east = self.east.max(lon);
        self.south = self.south.min(lat);
        self.north = self.north.max(lat);
    }

    fn contains(&self, lon: f32, lat: f32, margin: f32) -> bool {
        lon >= self.west - margin && lon <= self.east + margin && lat >= self.south - margin && lat <= self.north + margin
    }
}

impl Area {
    /// Name in the interface language.
    pub fn name(&self) -> &str {
        let lang = Lang::ALL.iter().position(|lang| *lang == i18n::current()).unwrap_or(0);
        self.names.get(lang).unwrap_or(&self.names[0])
    }

    /// Whether the point is inside, holes excepted.
    pub fn contains(&self, lon: f32, lat: f32) -> bool {
        if !self.bounds.contains(lon, lat, 0.0) {
            return false;
        }
        let mut inside = false;
        for (ring, bounds) in self.rings.iter().zip(&self.ring_bounds) {
            if !bounds.contains(lon, lat, 0.0) {
                continue;
            }
            let points = &self.points[ring.0 as usize..(ring.0 + ring.1) as usize];
            let mut previous = points[points.len() - 1];
            for &point in points {
                let ([x1, y1], [x2, y2]) = (point, previous);
                if (y1 > lat) != (y2 > lat) && lon < (x2 - x1) * (lat - y1) / (y2 - y1) + x1 {
                    inside = !inside;
                }
                previous = point;
            }
        }
        inside
    }

    /// Squared distance in degrees, with longitude shrunk by latitude, from
    /// the point to the nearest outline point within `NEAR`.
    fn distance(&self, lon: f32, lat: f32) -> Option<f32> {
        if !self.bounds.contains(lon, lat, NEAR) {
            return None;
        }
        let squeeze = lat.to_radians().cos();
        let mut best = NEAR * NEAR;
        let mut found = false;
        for (ring, bounds) in self.rings.iter().zip(&self.ring_bounds) {
            if !bounds.contains(lon, lat, NEAR) {
                continue;
            }
            for [x, y] in &self.points[ring.0 as usize..(ring.0 + ring.1) as usize] {
                let (dx, dy) = ((x - lon) * squeeze, y - lat);
                let distance = dx * dx + dy * dy;
                if distance < best {
                    best = distance;
                    found = true;
                }
            }
        }
        found.then_some(best)
    }

    /// Whether the outline edge from `a` to `b` is only where Natural Earth
    /// cut a shape at the date line or closed Antarctica at the pole, not a
    /// border or coast.
    pub fn is_seam(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0].abs() == 180.0 && a[0] == b[0]) || (a[1] == -90.0 && b[1] == -90.0)
    }
}

pub struct World {
    pub areas: Vec<Area>,
    /// Indices of the countries, the areas without a parent.
    pub countries: Vec<usize>,
    /// States or provinces of each area, for the few countries that have them.
    pub children: Vec<Vec<usize>>,
}

/// The world map, unpacked the first time it is needed.
pub fn world() -> &'static World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD.get_or_init(|| unpack(PACKED).expect("build/world.bin is a world map from scripts/world-map.py"))
}

impl World {
    /// The country, or its state or province where the map has those, the
    /// place is in or just off the coast of.
    pub fn locate(&self, lat: f64, lon: f64) -> Option<usize> {
        let (lon, lat) = (lon as f32, lat as f32);
        let country = self.find(&self.countries, lon, lat, true)?;
        Some(self.find(&self.children[country], lon, lat, true).unwrap_or(country))
    }

    /// Like `locate`, but only on land: for what the pointer is over.
    pub fn area_at(&self, lat: f32, lon: f32) -> Option<usize> {
        let country = self.find(&self.countries, lon, lat, false)?;
        Some(self.find(&self.children[country], lon, lat, false).unwrap_or(country))
    }

    fn find(&self, candidates: &[usize], lon: f32, lat: f32, near: bool) -> Option<usize> {
        if let Some(&inside) = candidates.iter().find(|&&index| self.areas[index].contains(lon, lat)) {
            return Some(inside);
        }
        if !near {
            return None;
        }
        candidates
            .iter()
            .filter_map(|&index| Some((index, self.areas[index].distance(lon, lat)?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// The country an area belongs to: itself, or its parent.
    pub fn country_of(&self, area: usize) -> usize {
        self.areas[area].parent.unwrap_or(area)
    }

    #[cfg(test)]
    pub fn find_id(&self, id: &str) -> Option<usize> {
        self.areas.iter().position(|area| area.id == id)
    }
}

/// Reads the packed map. See `scripts/world-map.py` for the layout.
fn unpack(packed: &[u8]) -> Option<World> {
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(packed).read_to_end(&mut bytes).ok()?;
    let mut reader = Reader { bytes: &bytes, at: 0 };
    if reader.take(4)? != b"PWM1" {
        return None;
    }
    let count = reader.u16()? as usize;
    let mut areas = Vec::with_capacity(count);
    for _ in 0..count {
        let id_len = reader.u8()? as usize;
        let id = String::from_utf8(reader.take(id_len)?.to_vec()).ok()?;
        let parent = match reader.u16()? {
            0xFFFF => None,
            parent => Some(parent as usize),
        };
        let continent = (reader.u8()? as usize).min(CONTINENTS.len() - 1);
        let mut names = Vec::with_capacity(Lang::ALL.len());
        for _ in 0..Lang::ALL.len() {
            let len = reader.u16()? as usize;
            names.push(String::from_utf8(reader.take(len)?.to_vec()).ok()?);
        }
        let label = unquantize(reader.u16()?, reader.u16()?);
        let vertices = reader.u32()? as usize;
        let mut points = Vec::with_capacity(vertices);
        let (mut x, mut y) = (0u16, 0u16);
        for _ in 0..vertices {
            x = x.wrapping_add(reader.u16()?);
            y = y.wrapping_add(reader.u16()?);
            points.push(unquantize(x, y));
        }
        let ring_count = reader.u16()? as usize;
        let mut rings = Vec::with_capacity(ring_count);
        for _ in 0..ring_count {
            let (start, len) = (reader.u32()?, reader.u32()?);
            if len < 3 || (start + len) as usize > points.len() {
                return None;
            }
            rings.push((start, len));
        }
        let index_count = reader.u32()? as usize;
        let mut triangles = Vec::with_capacity(index_count);
        let mut index = 0i64;
        for _ in 0..index_count {
            index += i64::from(reader.i32()?);
            if index < 0 || index as usize >= points.len() {
                return None;
            }
            triangles.push(index as u32);
        }
        let ring_bounds: Vec<Bounds> = rings
            .iter()
            .map(|(start, len)| {
                let mut bounds = Bounds::EMPTY;
                points[*start as usize..(*start + *len) as usize].iter().for_each(|point| bounds.add(*point));
                bounds
            })
            .collect();
        let mut bounds = Bounds::EMPTY;
        points.iter().for_each(|point| bounds.add(*point));
        let flat: Vec<[f32; 2]> = points.iter().map(|[lon, lat]| natural_earth(*lon, *lat)).collect();
        let flat_bounds = flat.iter().fold([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY], |b, [x, y]| {
            [b[0].min(*x), b[1].min(*y), b[2].max(*x), b[3].max(*y)]
        });
        let sphere = points.iter().map(|[lon, lat]| unit_vector(*lon, *lat)).collect();
        areas.push(Area {
            id,
            parent,
            continent,
            names,
            label,
            points,
            rings,
            ring_bounds,
            triangles,
            bounds,
            flat,
            flat_bounds,
            sphere,
        });
    }
    let mut children = vec![Vec::new(); areas.len()];
    let mut countries = Vec::new();
    for (index, area) in areas.iter().enumerate() {
        match area.parent {
            Some(parent) if parent < areas.len() => children[parent].push(index),
            Some(_) => return None,
            None => countries.push(index),
        }
    }
    Some(World { areas, countries, children })
}

fn unquantize(x: u16, y: u16) -> [f32; 2] {
    [f32::from(x) * 360.0 / 65535.0 - 180.0, f32::from(y) * 180.0 / 65535.0 - 90.0]
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(len)?)?;
        self.at += len;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

/// Widest x and y the flat projection reaches, at the equator's ends and at
/// the poles.
pub const FLAT_EXTENT: [f32; 2] = [2.7354, 1.4224];

/// Natural Earth projection (Šavrič, Jenny, Patterson, Petrovič, Hurni) of a
/// longitude and latitude in degrees; y grows northward.
pub fn natural_earth(lon: f32, lat: f32) -> [f32; 2] {
    let (lambda, phi) = (lon.to_radians(), lat.to_radians());
    let p2 = phi * phi;
    let p4 = p2 * p2;
    let x = lambda * (0.870700 - 0.131979 * p2 + p4 * (-0.013791 + p4 * p2 * (0.003971 - 0.001529 * p2)));
    let y = phi * (1.007226 + p2 * (0.015085 + p4 * (-0.044475 + 0.028874 * p2 - 0.005916 * p4)));
    [x, y]
}

/// Longitude and latitude at a point of the flat projection, if it is on
/// the map.
pub fn natural_earth_inverse(x: f32, y: f32) -> Option<(f32, f32)> {
    let mut phi = y;
    for _ in 0..25 {
        let p2 = phi * phi;
        let p4 = p2 * p2;
        let f = phi * (1.007226 + p2 * (0.015085 + p4 * (-0.044475 + 0.028874 * p2 - 0.005916 * p4))) - y;
        let df = 1.007226 + p2 * (3.0 * 0.015085 + p4 * (-7.0 * 0.044475 + 9.0 * 0.028874 * p2 - 11.0 * 0.005916 * p4));
        let step = f / df;
        phi -= step;
        if step.abs() < 1e-6 {
            break;
        }
    }
    if !phi.is_finite() || phi.abs() > FRAC_PI_2 + 1e-4 {
        return None;
    }
    let p2 = phi * phi;
    let p4 = p2 * p2;
    let lambda = x / (0.870700 - 0.131979 * p2 + p4 * (-0.013791 + p4 * p2 * (0.003971 - 0.001529 * p2)));
    if !lambda.is_finite() || lambda.abs() > PI + 1e-4 {
        return None;
    }
    Some((lambda.to_degrees(), phi.to_degrees().clamp(-90.0, 90.0)))
}

pub fn unit_vector(lon: f32, lat: f32) -> [f32; 3] {
    let (lambda, phi) = (lon.to_radians(), lat.to_radians());
    [phi.cos() * lambda.cos(), phi.cos() * lambda.sin(), phi.sin()]
}

/// The globe turned so `lon`, `lat` faces the viewer: an orthographic
/// projection.
#[derive(Clone, Copy)]
pub struct Globe {
    cos_lon: f32,
    sin_lon: f32,
    cos_lat: f32,
    sin_lat: f32,
}

impl Globe {
    pub fn facing(lon: f32, lat: f32) -> Globe {
        let (lambda, phi) = (lon.to_radians(), lat.to_radians());
        Globe {
            cos_lon: lambda.cos(),
            sin_lon: lambda.sin(),
            cos_lat: phi.cos(),
            sin_lat: phi.sin(),
        }
    }

    /// Where a point of the unit sphere shows: x right and y up on the unit
    /// disc, and its depth, which is negative on the far side.
    pub fn project(&self, [x, y, z]: [f32; 3]) -> [f32; 3] {
        let toward = x * self.cos_lon + y * self.sin_lon;
        let across = y * self.cos_lon - x * self.sin_lon;
        [across, self.cos_lat * z - self.sin_lat * toward, self.sin_lat * z + self.cos_lat * toward]
    }

    /// Longitude and latitude shown at a point of the unit disc.
    pub fn unproject(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let depth = 1.0 - x * x - y * y;
        if depth < 0.0 {
            return None;
        }
        let depth = depth.sqrt();
        let toward = self.cos_lat * depth - self.sin_lat * y;
        let z = self.sin_lat * depth + self.cos_lat * y;
        let east = toward * self.cos_lon - x * self.sin_lon;
        let north = toward * self.sin_lon + x * self.cos_lon;
        Some((north.atan2(east).to_degrees(), z.clamp(-1.0, 1.0).asin().to_degrees()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_countries_and_regions() {
        let world = world();
        assert!(world.countries.len() > 200);
        let us = world.find_id("US").unwrap();
        assert_eq!(world.children[us].len(), 51);
        let california = world.find_id("US_CA").unwrap();
        assert_eq!(world.areas[california].parent, Some(us));
        assert_eq!(world.areas[california].names[0], "California");
        let taiwan = world.find_id("TW").unwrap();
        assert_eq!(world.areas[taiwan].names[1], "臺灣");
        assert_eq!(CONTINENTS[world.areas[taiwan].continent], "Asia");
        for area in &world.areas {
            assert_eq!(area.triangles.len() % 3, 0, "{}", area.id);
            assert!(!area.rings.is_empty(), "{}", area.id);
        }
    }

    #[test]
    fn locates_places() {
        let world = world();
        let id = |place: Option<usize>| place.map(|index| world.areas[index].id.as_str());
        assert_eq!(id(world.locate(25.03, 121.56)), Some("TW"));
        assert_eq!(id(world.locate(48.85, 2.35)), Some("FR"));
        assert_eq!(id(world.locate(34.05, -118.24)), Some("US_CA"));
        assert_eq!(id(world.locate(-33.87, 151.21)), Some("AU_NSW"));
        assert_eq!(id(world.locate(35.68, 139.69)), Some("JP"));
        assert_eq!(id(world.locate(0.0, -30.0)), None, "mid-Atlantic");
        // On the waterfront, just outside the 1:50m coastline.
        assert_eq!(id(world.locate(25.15, 121.92)), Some("TW"));
        assert_eq!(id(world.area_at(48.85, 2.35)), Some("FR"));
    }

    #[test]
    fn projections_round_trip() {
        for (lon, lat) in [(0.0, 0.0), (121.5, 25.0), (-179.0, -60.0), (45.0, 89.0)] {
            let [x, y] = natural_earth(lon, lat);
            let (back_lon, back_lat) = natural_earth_inverse(x, y).unwrap();
            assert!((back_lon - lon).abs() < 0.01 && (back_lat - lat).abs() < 0.01, "{lon},{lat}");
            let globe = Globe::facing(100.0, 20.0);
            let [sx, sy, depth] = globe.project(unit_vector(lon, lat));
            if depth >= 0.0 {
                let (back_lon, back_lat) = globe.unproject(sx, sy).unwrap();
                assert!((back_lon - lon).abs() < 0.01 && (back_lat - lat).abs() < 0.01, "{lon},{lat}");
            }
        }
        let [x, _] = natural_earth(180.0, 0.0);
        assert!((x - FLAT_EXTENT[0]).abs() < 0.01);
        let [_, y] = natural_earth(0.0, 90.0);
        assert!((y - FLAT_EXTENT[1]).abs() < 0.01);
        assert!(natural_earth_inverse(3.0, 0.0).is_none());
        let globe = Globe::facing(121.0, 23.5);
        let [x, y, depth] = globe.project(unit_vector(121.0, 23.5));
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5 && (depth - 1.0).abs() < 1e-5);
    }
}
