//! The track editor's model: every point of a GPX, KML, or Polarsteps file
//! with its height and time, the cleanup GPXViewer runs on them, the edits
//! made by hand, and the GPX and KML files they are saved as.

use std::collections::HashSet;
use std::path::Path;

use base64::Engine;
use serde_json::{json, Map, Value};

use crate::config::Config;
use crate::i18n::tr;
use crate::track::{self, LocalName};

const MS_PER_HOUR: f64 = 3_600_000.0;
/// Points closer than this in latitude and longitude, about a centimeter,
/// are at the same place.
const SAME_PLACE: f64 = 1e-7;
/// A long gap is bridged only where the track runs on within this many
/// degrees of the gap's own heading at both ends.
const MAX_TURN_DEG: f64 = 60.0;
/// Share of the cluster radius the corners of a run's bounds must be within
/// to settle it without measuring each point, which leaves room for the
/// earth not being flat.
const BOUNDS_MARGIN: f64 = 0.999;
/// The element GPXViewer keeps its settings in, which Picasso writes too so
/// either reads the other's files the same way.
const SETTINGS_TAG: &str = "gpxviewer-settings";
/// `creator` of the GPX files Picasso saves.
pub const CREATOR: &str = "Picasso";

/// The cleanup thresholds, as in GPXViewer's Settings dialog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Points reached faster than this from the one before are dropped,
    /// unless they are more than `long_gap_m` away.
    pub max_speed_kmh: f64,
    /// Runs of points that fit in a circle this wide are merged into one.
    pub cluster_radius_m: f64,
    pub long_gap_m: f64,
    /// A point is redundant when a curve through its neighbors passes
    /// closer than this.
    pub tolerance_m: f64,
    pub remove_redundant: bool,
    /// Bridges gaps longer than `long_gap_m` with a point on a curve.
    pub fill_gaps: bool,
    /// A merged run of points that lasted this long is a stay.
    pub dwell_hours: f64,
    /// A step this long or this fast is a flight.
    pub flight_km: f64,
    pub flight_kmh: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            max_speed_kmh: 30.0,
            cluster_radius_m: 150.0,
            long_gap_m: 100.0,
            tolerance_m: 8.0,
            remove_redundant: true,
            fill_gaps: false,
            dwell_hours: 1.0,
            flight_km: 600.0,
            flight_kmh: 250.0,
        }
    }
}

impl Settings {
    /// The settings saved in `config`, with the defaults for any missing.
    pub fn load(config: &Config) -> Settings {
        let base = Settings::default();
        let number = |key: &str, fallback: f64| config.float(key).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(fallback);
        Settings {
            max_speed_kmh: number("track_max_speed_kmh", base.max_speed_kmh),
            cluster_radius_m: number("track_cluster_radius_m", base.cluster_radius_m),
            long_gap_m: number("track_long_gap_m", base.long_gap_m),
            tolerance_m: number("track_tolerance_m", base.tolerance_m),
            remove_redundant: config.flag("track_remove_redundant").unwrap_or(base.remove_redundant),
            fill_gaps: config.flag("track_fill_gaps").unwrap_or(base.fill_gaps),
            dwell_hours: number("track_dwell_hours", base.dwell_hours),
            flight_km: number("track_flight_km", base.flight_km),
            flight_kmh: number("track_flight_kmh", base.flight_kmh),
        }
    }

    pub fn save(&self, config: &mut Config) {
        config.set("track_max_speed_kmh", self.max_speed_kmh);
        config.set("track_cluster_radius_m", self.cluster_radius_m);
        config.set("track_long_gap_m", self.long_gap_m);
        config.set("track_tolerance_m", self.tolerance_m);
        config.set("track_remove_redundant", self.remove_redundant);
        config.set("track_fill_gaps", self.fill_gaps);
        config.set("track_dwell_hours", self.dwell_hours);
        config.set("track_flight_km", self.flight_km);
        config.set("track_flight_kmh", self.flight_kmh);
    }

    fn dwell_ms(&self) -> f64 {
        self.dwell_hours * MS_PER_HOUR
    }

    /// The settings under GPXViewer's names.
    fn to_json(self) -> Value {
        json!({
            "maxAllowedSpeedKmh": self.max_speed_kmh,
            "stationaryRadiusMeters": self.cluster_radius_m,
            "longGapMeters": self.long_gap_m,
            "simplificationToleranceMeters": self.tolerance_m,
            "bezierFillGaps": self.fill_gaps,
            "removeRedundantPoints": self.remove_redundant,
            "dwellMinMs": self.dwell_ms().round(),
            "flightDistanceKm": self.flight_km,
            "flightSpeedKmh": self.flight_kmh,
        })
    }

    /// `base` with the settings in `map` under GPXViewer's names. Other
    /// keys, such as its map source, are ignored.
    fn from_json(map: &Map<String, Value>, base: Settings) -> Settings {
        let number = |key: &str, fallback: f64| {
            map.get(key).and_then(Value::as_f64).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(fallback)
        };
        let flag = |key: &str, fallback: bool| map.get(key).and_then(Value::as_bool).unwrap_or(fallback);
        Settings {
            max_speed_kmh: number("maxAllowedSpeedKmh", base.max_speed_kmh),
            cluster_radius_m: number("stationaryRadiusMeters", base.cluster_radius_m),
            long_gap_m: number("longGapMeters", base.long_gap_m),
            tolerance_m: number("simplificationToleranceMeters", base.tolerance_m),
            remove_redundant: flag("removeRedundantPoints", base.remove_redundant),
            fill_gaps: flag("bezierFillGaps", base.fill_gaps),
            dwell_hours: number("dwellMinMs", base.dwell_ms()) / MS_PER_HOUR,
            flight_km: number("flightDistanceKm", base.flight_km),
            flight_kmh: number("flightSpeedKmh", base.flight_kmh),
        }
    }

    /// Base64 of the JSON, as GPXViewer stores it in a file.
    fn encode(self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.to_json().to_string())
    }

    fn decode(text: &str, base: Settings) -> Option<Settings> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()?;
        let value: Value = serde_json::from_slice(&bytes).ok()?;
        Some(Settings::from_json(value.as_object()?, base))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditPoint {
    pub lat: f64,
    pub lon: f64,
    /// Height in meters.
    pub ele: Option<f64>,
    /// When the point was recorded, in Unix milliseconds.
    pub time: Option<i64>,
    /// How long the stay this point stands for lasted, in milliseconds, or
    /// 0 when it is no stay.
    pub dwell_ms: i64,
}

impl EditPoint {
    pub fn new(lat: f64, lon: f64, ele: Option<f64>, time: Option<i64>) -> EditPoint {
        EditPoint { lat, lon, ele, time, dwell_ms: 0 }
    }

    fn valid(self) -> Option<EditPoint> {
        let fine = self.lat.is_finite() && self.lon.is_finite() && self.lat.abs() <= 90.0 && self.lon.abs() <= 180.0;
        fine.then_some(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Gpx,
    Kml,
    Polarsteps,
}

/// A track file read for editing.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub name: String,
    pub format: Format,
    pub points: Vec<EditPoint>,
    /// GPXViewer or Picasso saved it, so it is cleaned up already.
    pub cleaned: bool,
    /// Settings saved in the file, over the `base` it was read with.
    pub settings: Option<Settings>,
    /// Points left out for having no time when others have one.
    pub untimed: usize,
}

/// Reads a GPX, KML, KMZ, or Polarsteps JSON file. Settings saved in it
/// are read over `base`.
pub fn load(path: &Path, base: Settings) -> Result<Loaded, String> {
    let text = track::read_text(path)?;
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_ascii_lowercase();
    let mut loaded = match ext.as_str() {
        "gpx" => parse_gpx(&text, base)?,
        "json" => Loaded {
            name: String::new(),
            format: Format::Polarsteps,
            points: parse_polarsteps(&text)?,
            cleaned: false,
            settings: None,
            untimed: 0,
        },
        _ => parse_kml(&text, base)?,
    };
    if loaded.name.is_empty() {
        loaded.name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    }
    Ok(loaded)
}

/// The track points of a GPX file, or its route points when it has no
/// track, in time order.
pub fn parse_gpx(text: &str, base: Settings) -> Result<Loaded, String> {
    let doc = track::parse_xml(text)?;
    let creator = doc.root_element().attribute("creator").unwrap_or_default();
    let mut points: Vec<EditPoint> = doc.descendants().filter(|node| node.has_tag_name_local("trkpt")).filter_map(gpx_point).collect();
    if points.is_empty() {
        points = doc.descendants().filter(|node| node.has_tag_name_local("rtept")).filter_map(gpx_point).collect();
    }
    let name = doc
        .descendants()
        .filter(|node| node.has_tag_name_local("trk") || node.has_tag_name_local("rte") || node.has_tag_name_local("metadata"))
        .map(|node| track::child_text(node, "name"))
        .find(|name| !name.is_empty())
        .unwrap_or_default();
    let settings = doc
        .descendants()
        .find(|node| node.has_tag_name_local(SETTINGS_TAG))
        .and_then(|node| node.text())
        .and_then(|text| Settings::decode(text, base));
    let (points, untimed) = in_time_order(points);
    Ok(Loaded {
        name,
        format: Format::Gpx,
        points: enough(points)?,
        cleaned: creator == "GPXViewer" || creator == CREATOR,
        settings,
        untimed,
    })
}

/// The lines and recorded tracks of a KML file, one after another, in time
/// order.
pub fn parse_kml(text: &str, base: Settings) -> Result<Loaded, String> {
    let doc = track::parse_xml(text)?;
    let mut points = Vec::new();
    let mut name = String::new();
    for node in doc.descendants().filter(|node| node.is_element()) {
        let found = match node.tag_name().name() {
            "LineString" => node
                .children()
                .find(|child| child.has_tag_name_local("coordinates"))
                .map(|coordinates| kml_coordinates(coordinates.text().unwrap_or_default()))
                .unwrap_or_default(),
            "Track" => kml_track(node),
            _ => continue,
        };
        if found.len() < 2 {
            continue;
        }
        if name.is_empty() {
            if let Some(placemark) = node.ancestors().find(|parent| parent.has_tag_name_local("Placemark")) {
                name = track::child_text(placemark, "name");
            }
        }
        points.extend(found);
    }
    if name.is_empty() {
        if let Some(document) = doc.descendants().find(|node| node.has_tag_name_local("Document")) {
            name = track::child_text(document, "name");
        }
    }
    let settings = doc
        .descendants()
        .find(|node| node.has_tag_name_local("Data") && node.attribute("name") == Some(SETTINGS_TAG))
        .and_then(|node| node.children().find(|child| child.has_tag_name_local("value")))
        .and_then(|node| node.text())
        .and_then(|text| Settings::decode(text, base));
    let (points, untimed) = in_time_order(points);
    Ok(Loaded {
        name,
        format: Format::Kml,
        points: enough(points)?,
        cleaned: settings.is_some(),
        settings,
        untimed,
    })
}

/// The locations of a Polarsteps export, or of any JSON with latitudes and
/// longitudes, in time order. A location without a time is taken a minute
/// after the one before. Without any times, the locations keep file order
/// and stay without times.
pub fn parse_polarsteps(text: &str) -> Result<Vec<EditPoint>, String> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|err| err.to_string())?;
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![&value];
    while let Some(current) = stack.pop() {
        match current {
            Value::Array(items) => stack.extend(items.iter().rev()),
            Value::Object(map) => {
                if let Some(record) = json_record(map) {
                    let key = ((record.lat * 1e7).round() as i64, (record.lon * 1e7).round() as i64, record.time);
                    if seen.insert(key) {
                        records.push(record);
                    }
                    continue;
                }
                stack.extend(map.values().filter(|value| value.is_object() || value.is_array()));
            }
            _ => {}
        }
    }
    let Some(first_time) = records.iter().find_map(|record| record.time) else {
        return enough(dedupe(records));
    };
    let mut points: Vec<EditPoint> = Vec::with_capacity(records.len());
    for mut record in records {
        if record.time.is_none() {
            let before = points.last().and_then(|point| point.time).unwrap_or(first_time);
            record.time = Some(before + 60_000);
        }
        points.push(record);
    }
    points.sort_by_key(|point| point.time);
    enough(dedupe(points))
}

fn enough(points: Vec<EditPoint>) -> Result<Vec<EditPoint>, String> {
    if points.len() < 2 {
        Err(tr("No track in the file").to_string())
    } else {
        Ok(points)
    }
}

/// When any point has a time, the points that have one, oldest first, and
/// how many were left out for having none.
fn in_time_order(points: Vec<EditPoint>) -> (Vec<EditPoint>, usize) {
    if !points.iter().any(|point| point.time.is_some()) {
        return (points, 0);
    }
    let count = points.len();
    let mut timed: Vec<EditPoint> = points.into_iter().filter(|point| point.time.is_some()).collect();
    timed.sort_by_key(|point| point.time);
    let untimed = count - timed.len();
    (timed, untimed)
}

fn gpx_point(node: roxmltree::Node<'_, '_>) -> Option<EditPoint> {
    let lat = node.attribute("lat")?.trim().parse().ok()?;
    let lon = node.attribute("lon")?.trim().parse().ok()?;
    let child = |name: &str| node.children().find(|child| child.has_tag_name_local(name)).and_then(|child| child.text());
    let ele = child("ele").and_then(|text| text.trim().parse::<f64>().ok()).filter(|ele| ele.is_finite());
    let time = child("time").and_then(track::parse_time_ms);
    EditPoint::new(lat, lon, ele, time).valid()
}

/// `lon,lat[,alt]` tuples separated by white space.
fn kml_coordinates(text: &str) -> Vec<EditPoint> {
    text.split_whitespace()
        .filter_map(|tuple| {
            let mut parts = tuple.split(',');
            let lon = parts.next()?.parse().ok()?;
            let lat = parts.next()?.parse().ok()?;
            let ele = parts.next().and_then(|alt| alt.parse::<f64>().ok()).filter(|ele| ele.is_finite());
            EditPoint::new(lat, lon, ele, None).valid()
        })
        .collect()
}

/// `when` times paired in order with `coord` positions, `lon lat [alt]`.
fn kml_track(node: roxmltree::Node<'_, '_>) -> Vec<EditPoint> {
    let whens: Vec<Option<i64>> = node
        .children()
        .filter(|child| child.has_tag_name_local("when"))
        .map(|child| child.text().and_then(track::parse_time_ms))
        .collect();
    node.children()
        .filter(|child| child.has_tag_name_local("coord"))
        .enumerate()
        .filter_map(|(index, child)| {
            let mut parts = child.text()?.split_whitespace();
            let lon = parts.next()?.parse().ok()?;
            let lat = parts.next()?.parse().ok()?;
            let ele = parts.next().and_then(|alt| alt.parse::<f64>().ok()).filter(|ele| ele.is_finite());
            EditPoint::new(lat, lon, ele, whens.get(index).copied().flatten()).valid()
        })
        .collect()
}

/// A location in a JSON object: `coordinates` as `[lon, lat, alt]`, its own
/// latitude and longitude, or those of its `location`.
fn json_record(map: &Map<String, Value>) -> Option<EditPoint> {
    if let Some(point) = json_coordinates(map) {
        return Some(point);
    }
    let location = map.get("location")?.as_object()?;
    let mut point = json_coordinates(location)?;
    point.time = json_time(map).or(point.time);
    Some(point)
}

fn json_coordinates(map: &Map<String, Value>) -> Option<EditPoint> {
    let time = json_time(map);
    if let Some(Value::Array(values)) = map.get("coordinates") {
        if values.len() >= 2 {
            let at = |index: usize| values.get(index).and_then(json_number);
            if let (Some(lon), Some(lat)) = (at(0), at(1)) {
                return EditPoint::new(lat, lon, at(2), time).valid();
            }
        }
    }
    let pick = |keys: &[&str]| keys.iter().find_map(|key| map.get(*key).and_then(json_number));
    let lat = pick(&["lat", "latitude"])?;
    let lon = pick(&["lon", "lng", "longitude"])?;
    EditPoint::new(lat, lon, pick(&["ele", "elevation", "alt", "altitude"]), time).valid()
}

fn json_number(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    };
    number.filter(|number| number.is_finite())
}

/// A time under one of the names Polarsteps and similar exports use: Unix
/// seconds or milliseconds, or ISO 8601 text.
fn json_time(map: &Map<String, Value>) -> Option<i64> {
    const KEYS: [&str; 8] = ["time", "timestamp", "datetime", "date", "created_at", "updated_at", "local_time", "recorded_at"];
    KEYS.iter().find_map(|key| {
        let value = map.get(*key)?;
        let number = match value {
            Value::String(text) if !text.trim().is_empty() => text.trim().parse::<f64>().ok(),
            other => json_number(other),
        };
        match number {
            Some(number) => Some(if number.abs() < 1e12 { (number * 1000.0).round() as i64 } else { number.round() as i64 }),
            None => value.as_str().and_then(track::parse_time_ms),
        }
    })
}

/// GPXViewer's cleanup: drop points reached too fast, drop redundant ones,
/// merge the runs that stay in one place, then bridge long gaps.
pub fn optimize(raw: &[EditPoint], settings: &Settings) -> Vec<EditPoint> {
    let cleaned = remove_speeding(raw, settings);
    let reduced = if settings.remove_redundant { remove_redundant(&cleaned, settings) } else { cleaned };
    let merged = merge_stays(&reduced, settings);
    if settings.fill_gaps { fill_gaps(&merged, settings) } else { merged }
}

/// Drops each point reached from the last one kept faster than the speed
/// limit, unless it is farther than a long gap, as a flight or a jump over
/// a stretch without signal is. A point no later than the one before is
/// dropped too. Points without a time are kept.
fn remove_speeding(points: &[EditPoint], settings: &Settings) -> Vec<EditPoint> {
    let mut kept: Vec<EditPoint> = Vec::with_capacity(points.len());
    for point in points {
        let Some(last) = kept.last() else {
            kept.push(*point);
            continue;
        };
        if let (Some(from), Some(to)) = (last.time, point.time) {
            if to <= from {
                continue;
            }
            let meters = distance_m(last, point);
            let kmh = meters / 1000.0 / ((to - from) as f64 / MS_PER_HOUR);
            if kmh > settings.max_speed_kmh && meters <= settings.long_gap_m {
                continue;
            }
        }
        kept.push(*point);
    }
    kept
}

/// Drops each point that lies within the tolerance of a curve drawn from
/// the last point kept to the next one.
fn remove_redundant(points: &[EditPoint], settings: &Settings) -> Vec<EditPoint> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut kept = vec![points[0]];
    for index in 1..points.len() - 1 {
        let prev = kept[kept.len() - 1];
        let (current, next) = (points[index], points[index + 1]);
        let before = if kept.len() >= 2 { kept[kept.len() - 2] } else { prev };
        let after = points.get(index + 2).copied().unwrap_or(next);
        let control1 = (prev.lat + (next.lat - before.lat) / 6.0, prev.lon + (next.lon - before.lon) / 6.0);
        let control2 = (next.lat - (after.lat - prev.lat) / 6.0, next.lon - (after.lon - prev.lon) / 6.0);
        let (lat, lon) = cubic_bezier((prev.lat, prev.lon), control1, control2, (next.lat, next.lon), 0.5);
        if distance_m(&current, &EditPoint::new(lat, lon, None, None)) > settings.tolerance_m {
            kept.push(current);
        }
    }
    kept.push(points[points.len() - 1]);
    dedupe(kept)
}

fn cubic_bezier(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let at = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
    (at(p0.0, p1.0, p2.0, p3.0), at(p0.1, p1.1, p2.1, p3.1))
}

/// Merges each run of two or more points that fits in a circle of the
/// cluster radius round its center into one point there, at the middle
/// time. A run that lasted the dwell time or more is a stay.
fn merge_stays(points: &[EditPoint], settings: &Settings) -> Vec<EditPoint> {
    let radius = settings.cluster_radius_m;
    let mut out = Vec::with_capacity(points.len());
    let mut start = 0;
    while start < points.len() {
        let (mut end, mut lat_sum, mut lon_sum) = (start, 0.0, 0.0);
        let (mut south, mut north, mut west, mut east) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        while end < points.len() {
            let point = points[end];
            let count = (end - start + 1) as f64;
            let center = EditPoint::new((lat_sum + point.lat) / count, (lon_sum + point.lon) / count, None, None);
            let (s, n, w, e) = (south.min(point.lat), north.max(point.lat), west.min(point.lon), east.max(point.lon));
            // No point of the run is farther from the center than the
            // farthest corner of its bounds, so most points of a long stay
            // are settled without measuring every point again.
            let farthest = [(s, w), (s, e), (n, w), (n, e)]
                .iter()
                .map(|&(lat, lon)| distance_m(&center, &EditPoint::new(lat, lon, None, None)))
                .fold(0.0, f64::max);
            let inside = farthest <= radius * BOUNDS_MARGIN || points[start..=end].iter().all(|point| distance_m(point, &center) <= radius);
            if !inside {
                break;
            }
            (south, north, west, east) = (s, n, w, e);
            lat_sum += point.lat;
            lon_sum += point.lon;
            end += 1;
        }
        if end - start >= 2 {
            let mut center = cluster_center(&points[start..end]);
            let lasted = match (points[start].time, points[end - 1].time) {
                (Some(first), Some(last)) => last - first,
                _ => 0,
            };
            if lasted as f64 >= settings.dwell_ms() {
                center.dwell_ms = lasted;
            }
            out.push(center);
            start = end;
        } else {
            out.push(EditPoint { dwell_ms: 0, ..points[start] });
            start += 1;
        }
    }
    dedupe(out)
}

fn cluster_center(cluster: &[EditPoint]) -> EditPoint {
    let count = cluster.len() as f64;
    let heights: Vec<f64> = cluster.iter().filter_map(|point| point.ele).collect();
    let ele = (!heights.is_empty()).then(|| heights.iter().sum::<f64>() / heights.len() as f64);
    let time = match (cluster[0].time, cluster[cluster.len() - 1].time) {
        (Some(first), Some(last)) => Some(first + (last - first) / 2),
        _ => None,
    };
    EditPoint::new(
        cluster.iter().map(|point| point.lat).sum::<f64>() / count,
        cluster.iter().map(|point| point.lon).sum::<f64>() / count,
        ele,
        time,
    )
}

/// Adds a point on a curve in the middle of each gap longer than the long
/// gap, where the track turns less than [`MAX_TURN_DEG`] into and out of it.
fn fill_gaps(points: &[EditPoint], settings: &Settings) -> Vec<EditPoint> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for index in 1..points.len() {
        let (start, end) = (points[index - 1], points[index]);
        if distance_m(&start, &end) > settings.long_gap_m {
            let prev = index.checked_sub(2).map(|at| points[at]);
            let next = points.get(index + 1).copied();
            let heading = bearing_deg(&start, &end);
            let straight_in = prev.is_none_or(|prev| distance_m(&prev, &start) <= 1.0 || turn_deg(bearing_deg(&prev, &start), heading) <= MAX_TURN_DEG);
            let straight_out = next.is_none_or(|next| distance_m(&end, &next) <= 1.0 || turn_deg(heading, bearing_deg(&end, &next)) <= MAX_TURN_DEG);
            if straight_in && straight_out {
                out.push(bezier_middle(prev.unwrap_or(start), start, end, next.unwrap_or(end)));
            }
        }
        out.push(end);
    }
    dedupe(out)
}

fn bezier_middle(prev: EditPoint, start: EditPoint, end: EditPoint, next: EditPoint) -> EditPoint {
    let control = (start.lat + 0.25 * (next.lat - prev.lat), start.lon + 0.25 * (next.lon - prev.lon));
    let lat = 0.25 * start.lat + 0.5 * control.0 + 0.25 * end.lat;
    let lon = 0.25 * start.lon + 0.5 * control.1 + 0.25 * end.lon;
    EditPoint::new(lat, lon, middle_ele(&start, &end), middle_time(&start, &end))
}

/// Heading on the flat map from `from` to `to`, in degrees.
fn bearing_deg(from: &EditPoint, to: &EditPoint) -> f64 {
    (to.lon - from.lon).atan2(to.lat - from.lat).to_degrees()
}

fn turn_deg(a: f64, b: f64) -> f64 {
    ((b - a + 540.0).rem_euclid(360.0) - 180.0).abs()
}

fn middle_ele(a: &EditPoint, b: &EditPoint) -> Option<f64> {
    match (a.ele, b.ele) {
        (Some(x), Some(y)) => Some((x + y) / 2.0),
        (one, other) => one.or(other),
    }
}

fn middle_time(a: &EditPoint, b: &EditPoint) -> Option<i64> {
    Some(a.time? + (b.time? - a.time?) / 2)
}

/// Drops each point at the same time and place as the one before.
fn dedupe(points: Vec<EditPoint>) -> Vec<EditPoint> {
    let mut out: Vec<EditPoint> = Vec::with_capacity(points.len());
    for point in points {
        let same = out.last().is_some_and(|last| {
            last.time == point.time && (last.lat - point.lat).abs() < SAME_PLACE && (last.lon - point.lon).abs() < SAME_PLACE
        });
        if !same {
            out.push(point);
        }
    }
    out
}

/// Great-circle distance in meters.
pub fn distance_m(a: &EditPoint, b: &EditPoint) -> f64 {
    let point = |p: &EditPoint| track::Point { lat: p.lat, lon: p.lon, time: None };
    track::distance_m(&point(a), &point(b))
}

/// Speed from `from` to `to` in km/h, or 0 without both times.
pub fn speed_kmh(from: &EditPoint, to: &EditPoint) -> f64 {
    match (from.time, to.time) {
        (Some(start), Some(end)) if end > start => distance_m(from, to) / 1000.0 / ((end - start) as f64 / MS_PER_HOUR),
        _ => 0.0,
    }
}

/// For each step from one point to the next, whether it is a flight: longer
/// or faster than the flight thresholds.
pub fn flights(points: &[EditPoint], settings: &Settings) -> Vec<bool> {
    points
        .windows(2)
        .map(|pair| distance_m(&pair[0], &pair[1]) / 1000.0 >= settings.flight_km || speed_kmh(&pair[0], &pair[1]) >= settings.flight_kmh)
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub meters: f64,
    /// From the first point to the last, in milliseconds.
    pub duration_ms: Option<i64>,
    pub average_kmh: Option<f64>,
    pub top_kmh: f64,
    pub stays: usize,
}

pub fn stats(points: &[EditPoint]) -> Stats {
    let meters: f64 = points.windows(2).map(|pair| distance_m(&pair[0], &pair[1])).sum();
    let duration_ms = match (points.first().and_then(|p| p.time), points.last().and_then(|p| p.time)) {
        (Some(first), Some(last)) => Some(last - first),
        _ => None,
    };
    Stats {
        meters,
        duration_ms,
        average_kmh: duration_ms.filter(|ms| *ms > 0).map(|ms| meters / 1000.0 / (ms as f64 / MS_PER_HOUR)),
        top_kmh: points.windows(2).map(|pair| speed_kmh(&pair[0], &pair[1])).fold(0.0, f64::max),
        stays: points.iter().filter(|point| point.dwell_ms > 0).count(),
    }
}

/// Puts a point at `lat`, `lon` between `points[segment]` and the next one,
/// at their middle height and time. Returns where it went.
pub fn insert_point(points: &mut Vec<EditPoint>, segment: usize, lat: f64, lon: f64) -> usize {
    let (start, end) = (points[segment], points[segment + 1]);
    points.insert(segment + 1, EditPoint::new(lat, lon, middle_ele(&start, &end), middle_time(&start, &end)));
    segment + 1
}

/// Removes the points at `indices`, unless fewer than two would be left.
/// Returns whether it did.
pub fn remove_points(points: &mut Vec<EditPoint>, indices: &HashSet<usize>) -> bool {
    let removed = indices.iter().filter(|index| **index < points.len()).count();
    if removed == 0 || points.len() - removed < 2 {
        return false;
    }
    let mut index = 0;
    points.retain(|_| {
        index += 1;
        !indices.contains(&(index - 1))
    });
    true
}

/// File name for the cleaned-up copy of `path`, as GPXViewer names it.
pub fn saved_name(path: &Path, extension: &str) -> String {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "track".into());
    let stem = stem.strip_suffix("-optimized").unwrap_or(&stem);
    format!("{stem}-optimized.{extension}")
}

/// A GPX 1.1 file of one track, with `settings` kept for GPXViewer and the
/// next edit.
pub fn to_gpx(points: &[EditPoint], name: &str, settings: &Settings) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!("<gpx version=\"1.1\" creator=\"{CREATOR}\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n"));
    out.push_str(&format!("  <metadata>\n    <extensions>\n      <{SETTINGS_TAG}>{}</{SETTINGS_TAG}>\n    </extensions>\n  </metadata>\n", settings.encode()));
    out.push_str(&format!("  <trk>\n    <name>{}</name>\n    <trkseg>\n", escape_xml(name)));
    for point in points {
        out.push_str(&format!("      <trkpt lat=\"{}\" lon=\"{}\">", point.lat, point.lon));
        if let Some(ele) = point.ele {
            out.push_str(&format!("<ele>{ele}</ele>"));
        }
        if let Some(time) = point.time.and_then(iso_time) {
            out.push_str(&format!("<time>{time}</time>"));
        }
        out.push_str("</trkpt>\n");
    }
    out.push_str("    </trkseg>\n  </trk>\n</gpx>\n");
    out
}

/// A KML file of one placemark: a `gx:Track` when every point has a time,
/// otherwise a `LineString`. A point without a height is written without
/// one rather than at 0 m.
pub fn to_kml(points: &[EditPoint], name: &str, settings: &Settings) -> String {
    let name = escape_xml(name);
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<kml xmlns=\"http://www.opengis.net/kml/2.2\" xmlns:gx=\"http://www.google.com/kml/ext/2.2\">\n  <Document>\n");
    out.push_str(&format!("    <name>{name}</name>\n"));
    out.push_str(&format!(
        "    <ExtendedData>\n      <Data name=\"{SETTINGS_TAG}\">\n        <value>{}</value>\n      </Data>\n    </ExtendedData>\n",
        settings.encode()
    ));
    out.push_str(&format!("    <Placemark>\n      <name>{name}</name>\n"));
    let times: Option<Vec<String>> = points.iter().map(|point| point.time.and_then(iso_time)).collect();
    match times {
        Some(times) => {
            out.push_str("      <gx:Track>\n");
            for time in times {
                out.push_str(&format!("        <when>{time}</when>\n"));
            }
            for point in points {
                let ele = point.ele.map(|ele| format!(" {ele}")).unwrap_or_default();
                out.push_str(&format!("        <gx:coord>{} {}{ele}</gx:coord>\n", point.lon, point.lat));
            }
            out.push_str("      </gx:Track>\n");
        }
        None => {
            out.push_str("      <LineString>\n        <coordinates>\n");
            for point in points {
                let ele = point.ele.map(|ele| format!(",{ele}")).unwrap_or_default();
                out.push_str(&format!("          {},{}{ele}\n", point.lon, point.lat));
            }
            out.push_str("        </coordinates>\n      </LineString>\n");
        }
    }
    out.push_str("    </Placemark>\n  </Document>\n</kml>\n");
    out
}

fn iso_time(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|time| time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A point `north` meters north and `east` meters east of 25°N 121°E,
    /// `minutes` after the start.
    fn at(north: f64, east: f64, minutes: f64) -> EditPoint {
        let lat = 25.0 + north / 111_195.0;
        let lon = 121.0 + east / (111_195.0 * 25_f64.to_radians().cos());
        EditPoint::new(lat, lon, None, Some((minutes * 60_000.0) as i64))
    }

    #[test]
    fn gpx_points_keep_heights_and_milliseconds() {
        let text = r#"<?xml version="1.0"?>
<gpx version="1.1" creator="phone" xmlns="http://www.topografix.com/GPX/1/1">
  <metadata><name>Walk</name></metadata>
  <trk><trkseg>
    <trkpt lat="25.1" lon="121.1"><ele>12.5</ele><time>2024-05-01T08:00:01.250Z</time></trkpt>
    <trkpt lat="25.0" lon="121.0"><ele>10</ele><time>2024-05-01T08:00:00Z</time></trkpt>
    <trkpt lat="25.2" lon="121.2"/>
  </trkseg></trk>
</gpx>"#;
        let loaded = parse_gpx(text, Settings::default()).unwrap();
        assert_eq!(loaded.name, "Walk");
        assert!(!loaded.cleaned && loaded.settings.is_none());
        // Sorted by time; the point without one is left out.
        assert_eq!(loaded.points.len(), 2);
        assert_eq!(loaded.untimed, 1);
        assert_eq!(loaded.points[0], EditPoint::new(25.0, 121.0, Some(10.0), Some(1_714_550_400_000)));
        assert_eq!(loaded.points[1].time, Some(1_714_550_401_250));
        assert_eq!(loaded.points[1].ele, Some(12.5));
        // Without any times, file order stays.
        let untimed = r#"<gpx><rte><rtept lat="1" lon="2"/><rtept lat="0" lon="1"/></rte></gpx>"#;
        let loaded = parse_gpx(untimed, Settings::default()).unwrap();
        assert_eq!(loaded.points[0].lat, 1.0);
        assert!(parse_gpx("<gpx><trk><trkseg><trkpt lat=\"1\" lon=\"2\"/></trkseg></trk></gpx>", Settings::default()).is_err());
    }

    #[test]
    fn kml_lines_and_tracks_are_joined() {
        let text = r#"<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2"><Document>
  <name>Holiday</name>
  <Placemark><gx:Track>
    <when>2024-05-01T08:00:00Z</when><when>2024-05-01T08:01:00Z</when>
    <gx:coord>121.0 25.0 5</gx:coord><gx:coord>121.1 25.1 6</gx:coord>
  </gx:Track></Placemark>
  <Placemark><LineString><coordinates>121.2,25.2,7 121.3,25.3</coordinates></LineString></Placemark>
</Document></kml>"#;
        let loaded = parse_kml(text, Settings::default()).unwrap();
        assert_eq!(loaded.name, "Holiday");
        assert_eq!(loaded.format, Format::Kml);
        // The line has no times, so only the timed points are kept.
        assert_eq!(loaded.points.len(), 2);
        assert_eq!(loaded.points[1], EditPoint::new(25.1, 121.1, Some(6.0), Some(1_714_550_460_000)));
        let line = r#"<kml><Placemark><name>Line</name><LineString><coordinates>1,2,3 4,5</coordinates></LineString></Placemark></kml>"#;
        let loaded = parse_kml(line, Settings::default()).unwrap();
        assert_eq!(loaded.name, "Line");
        assert_eq!(loaded.points, vec![EditPoint::new(2.0, 1.0, Some(3.0), None), EditPoint::new(5.0, 4.0, None, None)]);
    }

    #[test]
    fn polarsteps_locations_are_read_in_time_order() {
        let text = r#"{"locations": [
            {"lat": 40.5, "lon": -3.6, "time": 1751115143.5},
            {"lat": 40.4, "lon": -3.5, "time": 1751110826},
            {"lat": 40.4, "lon": -3.5, "time": 1751110826},
            {"lat": 40.6, "lon": -3.7},
            {"name": "step", "location": {"lat": 41.0, "lon": -3.0}, "time": "2025-06-28T12:00:00Z"}
        ]}"#;
        let points = parse_polarsteps(text).unwrap();
        // The repeated location is read once, and the one without a time
        // is taken a minute after the location before it in the file.
        let times: Vec<Option<i64>> = points.iter().map(|point| point.time).collect();
        assert_eq!(times, [Some(1_751_110_826_000), Some(1_751_110_886_000), Some(1_751_112_000_000), Some(1_751_115_143_500)]);
        assert_eq!(points[2], EditPoint::new(41.0, -3.0, None, Some(1_751_112_000_000)));
        // Without any times, none are made up and file order stays.
        let untimed = parse_polarsteps(r#"{"locations": [{"lat": 2, "lon": 1}, {"lat": 1, "lon": 2}]}"#).unwrap();
        assert_eq!(untimed, vec![EditPoint::new(2.0, 1.0, None, None), EditPoint::new(1.0, 2.0, None, None)]);
        assert!(parse_polarsteps(r#"{"name": "no places"}"#).is_err());
        assert!(parse_polarsteps("not json").is_err());
    }

    #[test]
    fn speeding_points_are_dropped_but_long_jumps_kept() {
        let settings = Settings::default();
        // 50 m in 1 s is 180 km/h and within a long gap: a glitch.
        let glitch = at(50.0, 0.0, 0.0 + 1.0 / 60.0);
        // 5 km in a minute is far over the limit but a long jump.
        let jump = at(5000.0, 0.0, 2.0);
        let points = [at(0.0, 0.0, 0.0), glitch, at(10.0, 0.0, 1.0), jump, at(5000.0, 0.0, 1.5)];
        let kept = remove_speeding(&points, &settings);
        // The last point is earlier than the jump before it.
        assert_eq!(kept, vec![points[0], points[2], points[3]]);
    }

    #[test]
    fn points_on_the_curve_are_redundant() {
        let settings = Settings::default();
        // Each curve runs from the last point kept, so after one point goes
        // the next spans a wider gap and stays.
        let line: Vec<EditPoint> = (0..6).map(|step| at(0.0, step as f64 * 100.0, step as f64)).collect();
        assert_eq!(remove_redundant(&line, &settings), vec![line[0], line[2], line[4], line[5]]);
        // Moved off the line, the fourth point is no longer on the curve.
        let mut bent = line.clone();
        bent[3] = at(80.0, 300.0, 3.0);
        assert!(remove_redundant(&bent, &settings).contains(&bent[3]));
    }

    #[test]
    fn a_long_stay_becomes_one_point() {
        let settings = Settings::default();
        let mut points = vec![at(-2000.0, 0.0, 0.0)];
        points.extend((0..5).map(|step| at(step as f64 * 10.0, 0.0, 10.0 + step as f64 * 20.0)));
        points.push(at(3000.0, 0.0, 200.0));
        let merged = merge_stays(&points, &settings);
        assert_eq!(merged.len(), 3);
        let stay = merged[1];
        assert!((distance_m(&stay, &at(20.0, 0.0, 0.0))) < 0.5);
        // From 10 to 90 minutes: over an hour, at the middle time.
        assert_eq!(stay.dwell_ms, 80 * 60_000);
        assert_eq!(stay.time, Some(50 * 60_000));
        assert_eq!(stats(&merged).stays, 1);
        let short = Settings { dwell_hours: 2.0, ..settings };
        assert_eq!(merge_stays(&points, &short)[1].dwell_ms, 0);
    }

    /// [`merge_stays`] measuring every point of a run each time it grows.
    fn merge_stays_slowly(points: &[EditPoint], settings: &Settings) -> Vec<EditPoint> {
        let mut out = Vec::new();
        let mut start = 0;
        while start < points.len() {
            let (mut end, mut lat_sum, mut lon_sum) = (start, 0.0, 0.0);
            while end < points.len() {
                let count = (end - start + 1) as f64;
                let center = EditPoint::new((lat_sum + points[end].lat) / count, (lon_sum + points[end].lon) / count, None, None);
                if points[start..=end].iter().any(|point| distance_m(point, &center) > settings.cluster_radius_m) {
                    break;
                }
                lat_sum += points[end].lat;
                lon_sum += points[end].lon;
                end += 1;
            }
            if end - start >= 2 {
                let mut center = cluster_center(&points[start..end]);
                let lasted = match (points[start].time, points[end - 1].time) {
                    (Some(first), Some(last)) => last - first,
                    _ => 0,
                };
                if lasted as f64 >= settings.dwell_ms() {
                    center.dwell_ms = lasted;
                }
                out.push(center);
                start = end;
            } else {
                out.push(EditPoint { dwell_ms: 0, ..points[start] });
                start += 1;
            }
        }
        dedupe(out)
    }

    #[test]
    fn long_stays_merge_as_when_every_point_is_measured() {
        let settings = Settings::default();
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        // Wandering slowly, then fast, in turn, makes runs of every length.
        let (mut north, mut east) = (0.0, 0.0);
        let mut walk = Vec::new();
        for step in 0..3000 {
            let reach = if (step / 300) % 2 == 0 { 60.0 } else { 600.0 };
            north += random() * reach;
            east += random() * reach;
            walk.push(at(north, east, step as f64));
        }
        assert_eq!(merge_stays(&walk, &settings), merge_stays_slowly(&walk, &settings));
        // A night of a point a second within 60 m is one stay, merged
        // without measuring 30,000 points for each one.
        let night: Vec<EditPoint> = (0..30_000).map(|second| at(random() * 60.0, random() * 60.0, second as f64 / 60.0)).collect();
        let merged = merge_stays(&night, &settings);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].dwell_ms > 8 * 3_600_000);
    }

    #[test]
    fn straight_long_gaps_get_a_middle_point() {
        let settings = Settings { fill_gaps: true, ..Settings::default() };
        let points = [at(0.0, 0.0, 0.0), at(50.0, 0.0, 1.0), at(550.0, 0.0, 6.0), at(600.0, 0.0, 7.0)];
        let filled = fill_gaps(&points, &settings);
        assert_eq!(filled.len(), 5);
        // The curve's control point is a quarter of the way along the
        // neighbors, which puts the middle at 250 m rather than 300 m.
        assert!((distance_m(&filled[2], &at(250.0, 0.0, 0.0))) < 1.0);
        assert_eq!(filled[2].time, Some(210_000));
        // A sharp turn into the gap leaves it as it is.
        let turned = [at(0.0, -50.0, 0.0), at(0.0, 0.0, 1.0), at(500.0, 0.0, 6.0)];
        assert_eq!(fill_gaps(&turned, &settings).len(), 3);
    }

    #[test]
    fn flights_and_stats() {
        let settings = Settings::default();
        let points = [at(0.0, 0.0, 0.0), at(1000.0, 0.0, 6.0), at(701_000.0, 0.0, 66.0)];
        assert_eq!(flights(&points, &settings), vec![false, true]);
        let stats = stats(&points);
        assert!((stats.meters - 701_000.0).abs() < 50.0);
        assert_eq!(stats.duration_ms, Some(66 * 60_000));
        assert!((stats.top_kmh - 700.0).abs() < 1.0);
        assert!((speed_kmh(&points[0], &points[1]) - 10.0).abs() < 0.01);
    }

    #[test]
    fn edits_insert_between_and_keep_two_points() {
        let mut points = vec![EditPoint::new(0.0, 0.0, Some(10.0), Some(0)), EditPoint::new(0.0, 1.0, Some(20.0), Some(60_000))];
        assert_eq!(insert_point(&mut points, 0, 0.1, 0.5), 1);
        assert_eq!(points[1], EditPoint::new(0.1, 0.5, Some(15.0), Some(30_000)));
        assert!(!remove_points(&mut points, &HashSet::from([0, 1])));
        assert!(remove_points(&mut points, &HashSet::from([1, 7])));
        assert_eq!(points.len(), 2);
        assert_eq!(points[1].lon, 1.0);
    }

    #[test]
    fn saved_files_read_back_with_their_settings() {
        let settings = Settings { max_speed_kmh: 12.0, fill_gaps: true, dwell_hours: 0.5, ..Settings::default() };
        let mut points = vec![
            EditPoint::new(25.5, 121.25, Some(3.5), Some(1_714_550_400_123)),
            EditPoint::new(25.6, 121.35, None, Some(1_714_550_460_000)),
        ];
        let gpx = parse_gpx(&to_gpx(&points, "Tom & Jerry", &settings), Settings::default()).unwrap();
        assert_eq!(gpx.name, "Tom & Jerry");
        assert!(gpx.cleaned);
        assert_eq!(gpx.settings, Some(settings));
        assert_eq!(gpx.points, points);
        let kml = parse_kml(&to_kml(&points, "Ride", &settings), Settings::default()).unwrap();
        assert!(kml.cleaned);
        assert_eq!(kml.settings, Some(settings));
        // The point without a height comes back without one, not at 0 m.
        assert_eq!(kml.points, points);
        // A point without a time makes a line.
        points[1].time = None;
        let text = to_kml(&points, "Ride", &settings);
        assert!(text.contains("<LineString>") && !text.contains("<when>"));
        assert_eq!(parse_kml(&text, Settings::default()).unwrap().points[1].ele, None);
        assert_eq!(saved_name(Path::new("/a/ride.gpx"), "kml"), "ride-optimized.kml");
        assert_eq!(saved_name(Path::new("/a/ride-optimized.gpx"), "gpx"), "ride-optimized.gpx");
    }

    #[test]
    fn gpxviewer_settings_are_read() {
        // Saved by GPXViewer, with its map settings beside the cleanup ones.
        let encoded = "eyJtYXhBbGxvd2VkU3BlZWRLbWgiOjMwLCJzdGF0aW9uYXJ5UmFkaXVzTWV0ZXJzIjoxNTAsImxvbmdHYXBNZXRlcnMiOjEwMCwic2ltcGxpZmljYXRpb25Ub2xlcmFuY2VNZXRlcnMiOjgsImJlemllckZpbGxHYXBzIjpmYWxzZSwicmVtb3ZlUmVkdW5kYW50UG9pbnRzIjp0cnVlLCJkd2VsbE1pbk1zIjozNjAwMDAwLCJmbGlnaHREaXN0YW5jZUttIjo2MDAsImZsaWdodFNwZWVkS21oIjoyNTAsIm1hcFNvdXJjZSI6ImNhcnRvLXZveWFnZXIiLCJ1c2VDYXJ0byI6dHJ1ZSwiYmV6aWVyU21vb3RoUGF0aCI6ZmFsc2UsImJlemllclJlZHVuZGFudFRvbGVyYW5jZU1ldGVycyI6OCwidXNlT3BlbkZyZWVNYXAiOmZhbHNlfQ==";
        let base = Settings { max_speed_kmh: 99.0, ..Settings::default() };
        assert_eq!(Settings::decode(encoded, base), Some(Settings::default()));
        assert_eq!(Settings::decode("e30=", base), Some(base));
        assert_eq!(Settings::decode("not base64!", base), None);
    }

    #[test]
    fn the_whole_cleanup_runs_in_order() {
        let settings = Settings::default();
        let mut raw: Vec<EditPoint> = (0..20).map(|step| at(step as f64 * 400.0, 0.0, step as f64 * 2.0)).collect();
        raw.insert(5, at(1650.0, 30.0, 8.0 + 1.0 / 60.0));
        let cleaned = optimize(&raw, &settings);
        assert!(cleaned.len() >= 2 && cleaned.len() < raw.len());
        assert_eq!(cleaned.first().map(|p| p.time), Some(Some(0)));
        assert_eq!(cleaned.last().map(|p| p.time), raw.last().map(|p| p.time));
    }
}
