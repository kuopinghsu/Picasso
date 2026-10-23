//! GPS tracks: the paths a phone or GPS logger recorded, read from GPX and
//! KML files in the photo folders and drawn on the map under the photos.

use std::path::Path;

use crate::geo;
use crate::i18n::tr;

/// Extensions of the track files the scanner reads, lower case.
pub const EXTENSIONS: [&str; 2] = ["gpx", "kml"];

/// Larger files are left alone rather than read into memory whole.
const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Bytes per point in [`Track::encode`]: latitude and longitude in
/// ten-millionths of a degree, then the time.
const POINT_BYTES: usize = 16;

/// Stands for a point without a time in [`Track::encode`].
const NO_TIME: i64 = i64::MIN;

pub fn is_track_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    /// When the point was recorded, in Unix seconds.
    pub time: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub name: String,
    /// Stretches recorded without a break. Each has two points or more.
    pub segments: Vec<Vec<Point>>,
}

impl Track {
    /// Reads a GPX or KML file. A file without a name of its own is named
    /// after the file.
    pub fn read(path: &Path) -> Result<Track, String> {
        let bytes = std::fs::metadata(path).map_err(|err| err.to_string())?.len();
        if bytes > MAX_BYTES {
            return Err(tr("The track file is too large").to_string());
        }
        let data = std::fs::read(path).map_err(|err| err.to_string())?;
        let text = String::from_utf8_lossy(&data);
        let kml = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("kml"));
        let mut track = if kml { Track::parse_kml(&text) } else { Track::parse_gpx(&text) }?;
        if track.name.is_empty() {
            track.name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        }
        Ok(track)
    }

    /// Tracks (`trk`) and routes (`rte`) of a GPX file. Waypoints alone
    /// don't make a path and are left out.
    pub fn parse_gpx(text: &str) -> Result<Track, String> {
        let doc = parse_xml(text)?;
        let mut track = Track::default();
        for node in doc.descendants().filter(|node| node.is_element()) {
            match node.tag_name().name() {
                "trk" | "rte" => {
                    if track.name.is_empty() {
                        track.name = child_text(node, "name");
                    }
                    if node.tag_name().name() == "rte" {
                        track.add(node.children().filter(|child| child.has_tag_name_local("rtept")).filter_map(gpx_point).collect());
                    }
                }
                "trkseg" => track.add(node.children().filter(|child| child.has_tag_name_local("trkpt")).filter_map(gpx_point).collect()),
                _ => {}
            }
        }
        if track.name.is_empty() {
            if let Some(metadata) = doc.root_element().children().find(|child| child.has_tag_name_local("metadata")) {
                track.name = child_text(metadata, "name");
            }
        }
        track.finish()
    }

    /// Lines (`LineString`) and recorded tracks (`gx:Track`, also inside
    /// `gx:MultiTrack`) of a KML file. Outlines of areas are left out.
    pub fn parse_kml(text: &str) -> Result<Track, String> {
        let doc = parse_xml(text)?;
        let mut track = Track::default();
        for node in doc.descendants().filter(|node| node.is_element()) {
            let points = match node.tag_name().name() {
                "LineString" => node
                    .children()
                    .find(|child| child.has_tag_name_local("coordinates"))
                    .map(|coordinates| kml_coordinates(coordinates.text().unwrap_or_default()))
                    .unwrap_or_default(),
                "Track" => kml_track(node),
                _ => continue,
            };
            if points.len() >= 2 && track.name.is_empty() {
                if let Some(placemark) = node.ancestors().find(|parent| parent.has_tag_name_local("Placemark")) {
                    track.name = child_text(placemark, "name");
                }
            }
            track.add(points);
        }
        if track.name.is_empty() {
            if let Some(document) = doc.descendants().find(|node| node.has_tag_name_local("Document")) {
                track.name = child_text(document, "name");
            }
        }
        track.finish()
    }

    fn add(&mut self, points: Vec<Point>) {
        if points.len() >= 2 {
            self.segments.push(points);
        }
    }

    fn finish(self) -> Result<Track, String> {
        if self.segments.is_empty() {
            Err(tr("No track in the file").to_string())
        } else {
            Ok(self)
        }
    }

    pub fn points(&self) -> impl Iterator<Item = &Point> {
        self.segments.iter().flatten()
    }

    /// Times of the first and last points that have one.
    pub fn time_span(&self) -> Option<(i64, i64)> {
        let mut times = self.points().filter_map(|point| point.time);
        let first = times.next()?;
        Some(times.fold((first, first), |(start, end), time| (start.min(time), end.max(time))))
    }

    /// Length along the segments, in meters.
    pub fn meters(&self) -> f64 {
        self.segments
            .iter()
            .flat_map(|segment| segment.windows(2))
            .map(|pair| distance_m(&pair[0], &pair[1]))
            .sum()
    }

    /// Points for the catalog: per segment, its point count, then each point.
    pub fn encode(&self) -> Vec<u8> {
        let count: usize = self.segments.iter().map(Vec::len).sum();
        let mut out = Vec::with_capacity(self.segments.len() * 4 + count * POINT_BYTES);
        for segment in &self.segments {
            out.extend_from_slice(&(segment.len() as u32).to_le_bytes());
            for point in segment {
                out.extend_from_slice(&((point.lat * 1e7).round() as i32).to_le_bytes());
                out.extend_from_slice(&((point.lon * 1e7).round() as i32).to_le_bytes());
                out.extend_from_slice(&point.time.unwrap_or(NO_TIME).to_le_bytes());
            }
        }
        out
    }

    /// Reads [`Track::encode`]. A cut-off end is dropped.
    pub fn decode(name: String, bytes: &[u8]) -> Track {
        let mut segments = Vec::new();
        let mut rest = bytes;
        while let Some((head, tail)) = rest.split_first_chunk::<4>() {
            let count = u32::from_le_bytes(*head) as usize;
            let Some(body) = tail.get(..count.saturating_mul(POINT_BYTES)) else { break };
            let points = body
                .chunks_exact(POINT_BYTES)
                .map(|chunk| {
                    let int = |at: usize| i32::from_le_bytes(chunk[at..at + 4].try_into().unwrap_or_default());
                    let time = i64::from_le_bytes(chunk[8..16].try_into().unwrap_or_default());
                    Point {
                        lat: f64::from(int(0)) / 1e7,
                        lon: f64::from(int(4)) / 1e7,
                        time: (time != NO_TIME).then_some(time),
                    }
                })
                .collect();
            segments.push(points);
            rest = &tail[body.len()..];
        }
        Track { name, segments }
    }
}

/// A track ready to draw, as Web Mercator positions at zoom 0, where the
/// world is one 256-pixel tile. Multiply by `2^zoom` for another zoom.
pub struct MapTrack {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub name: String,
    pub time_span: Option<(i64, i64)>,
    pub meters: f64,
    pub segments: Vec<MapSegment>,
    pub bounds: Bounds,
}

pub struct MapSegment {
    pub points: Vec<[f64; 2]>,
    pub bounds: Bounds,
}

/// Least and greatest x and y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds { min: [f64::INFINITY; 2], max: [f64::NEG_INFINITY; 2] };

    pub fn add(&mut self, [x, y]: [f64; 2]) {
        self.min = [self.min[0].min(x), self.min[1].min(y)];
        self.max = [self.max[0].max(x), self.max[1].max(y)];
    }

    pub fn union(self, other: Bounds) -> Bounds {
        Bounds {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.min[0] > self.max[0] || self.min[1] > self.max[1]
    }

    pub fn center(&self) -> [f64; 2] {
        [(self.min[0] + self.max[0]) / 2.0, (self.min[1] + self.max[1]) / 2.0]
    }

    pub fn size(&self) -> [f64; 2] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1]]
    }
}

impl MapTrack {
    pub fn new(id: i64, folder_id: i64, path: String, track: &Track) -> MapTrack {
        let segments: Vec<MapSegment> = track
            .segments
            .iter()
            .map(|segment| {
                let mut bounds = Bounds::EMPTY;
                let points = segment
                    .iter()
                    .map(|point| {
                        let (x, y) = geo::latlon_to_world(point.lat, point.lon, 0);
                        bounds.add([x, y]);
                        [x, y]
                    })
                    .collect();
                MapSegment { points, bounds }
            })
            .collect();
        let bounds = segments.iter().fold(Bounds::EMPTY, |all, segment| all.union(segment.bounds));
        MapTrack {
            id,
            folder_id,
            path,
            name: track.name.clone(),
            time_span: track.time_span(),
            meters: track.meters(),
            segments,
            bounds,
        }
    }
}

/// Great-circle distance in meters.
pub fn distance_m(a: &Point, b: &Point) -> f64 {
    const EARTH_M: f64 = 6_371_008.8;
    let (lat1, lat2) = (a.lat.to_radians(), b.lat.to_radians());
    let half_lat = (lat2 - lat1) / 2.0;
    let half_lon = (b.lon - a.lon).to_radians() / 2.0;
    let h = half_lat.sin().powi(2) + lat1.cos() * lat2.cos() * half_lon.sin().powi(2);
    2.0 * EARTH_M * h.sqrt().min(1.0).asin()
}

/// "12.3 km", or meters below one kilometer.
pub fn format_distance(meters: f64) -> String {
    if meters < 1000.0 {
        format!("{meters:.0} m")
    } else if meters < 100_000.0 {
        format!("{:.1} km", meters / 1000.0)
    } else {
        format!("{:.0} km", meters / 1000.0)
    }
}

fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|err| err.to_string())
}

trait LocalName {
    fn has_tag_name_local(&self, name: &str) -> bool;
}

impl LocalName for roxmltree::Node<'_, '_> {
    /// Matches the tag in any namespace, since files use GPX 1.0 and 1.1,
    /// and KML with or without the `gx` extensions.
    fn has_tag_name_local(&self, name: &str) -> bool {
        self.is_element() && self.tag_name().name() == name
    }
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> String {
    node.children()
        .find(|child| child.has_tag_name_local(name))
        .and_then(|child| child.text())
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

fn gpx_point(node: roxmltree::Node<'_, '_>) -> Option<Point> {
    let lat = node.attribute("lat")?.trim().parse().ok()?;
    let lon = node.attribute("lon")?.trim().parse().ok()?;
    let time = node
        .children()
        .find(|child| child.has_tag_name_local("time"))
        .and_then(|child| child.text())
        .and_then(parse_time);
    valid(Point { lat, lon, time })
}

/// `lon,lat[,alt]` tuples separated by white space.
fn kml_coordinates(text: &str) -> Vec<Point> {
    text.split_whitespace()
        .filter_map(|tuple| {
            let mut parts = tuple.split(',');
            let lon = parts.next()?.parse().ok()?;
            let lat = parts.next()?.parse().ok()?;
            valid(Point { lat, lon, time: None })
        })
        .collect()
}

/// `when` times paired in order with `coord` positions, `lon lat [alt]`.
fn kml_track(node: roxmltree::Node<'_, '_>) -> Vec<Point> {
    let whens: Vec<Option<i64>> = node
        .children()
        .filter(|child| child.has_tag_name_local("when"))
        .map(|child| child.text().and_then(parse_time))
        .collect();
    node.children()
        .filter(|child| child.has_tag_name_local("coord"))
        .enumerate()
        .filter_map(|(index, child)| {
            let mut parts = child.text()?.split_whitespace();
            let lon = parts.next()?.parse().ok()?;
            let lat = parts.next()?.parse().ok()?;
            valid(Point { lat, lon, time: whens.get(index).copied().flatten() })
        })
        .collect()
}

fn valid(point: Point) -> Option<Point> {
    let fine = point.lat.is_finite() && point.lon.is_finite() && point.lat.abs() <= 90.0 && point.lon.abs() <= 180.0;
    fine.then_some(point)
}

/// An ISO 8601 time. One without a zone is taken as UTC, as GPX asks.
fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(time.timestamp());
    }
    ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"]
        .iter()
        .find_map(|format| chrono::NaiveDateTime::parse_from_str(text, format).ok())
        .map(|time| time.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1">
  <metadata><name>Morning ride</name></metadata>
  <wpt lat="1" lon="1"><name>Start</name></wpt>
  <trk>
    <name>Ride to the lake</name>
    <trkseg>
      <trkpt lat="25.0330" lon="121.5654"><ele>10</ele><time>2024-05-01T08:00:00Z</time></trkpt>
      <trkpt lat="25.0340" lon="121.5664"><time>2024-05-01T08:01:00Z</time></trkpt>
      <trkpt lat="bad" lon="121.5664"/>
    </trkseg>
    <trkseg>
      <trkpt lat="25.0400" lon="121.5700"><time>2024-05-01T09:00:00+08:00</time></trkpt>
    </trkseg>
    <trkseg>
      <trkpt lat="25.0500" lon="121.5800"/>
      <trkpt lat="25.0510" lon="121.5810"/>
    </trkseg>
  </trk>
  <rte><rtept lat="0" lon="0"/><rtept lat="0" lon="1"/></rte>
</gpx>"#;

    #[test]
    fn gpx_tracks_and_routes_are_read() {
        let track = Track::parse_gpx(GPX).unwrap();
        assert_eq!(track.name, "Ride to the lake");
        // The one-point segment is not a path.
        assert_eq!(track.segments.len(), 3);
        assert_eq!(track.segments[0].len(), 2);
        assert_eq!(track.segments[0][0], Point { lat: 25.033, lon: 121.5654, time: Some(1_714_550_400) });
        assert_eq!(track.segments[2], vec![Point { lat: 0.0, lon: 0.0, time: None }, Point { lat: 0.0, lon: 1.0, time: None }]);
        assert_eq!(track.time_span(), Some((1_714_550_400, 1_714_550_460)));
    }

    #[test]
    fn gpx_1_0_without_names_uses_metadata_or_nothing() {
        let text = r#"<gpx version="1.0" xmlns="http://www.topografix.com/GPX/1/0">
            <trk><trkseg><trkpt lat="1" lon="2"><time>2020-01-02T03:04:05</time></trkpt><trkpt lat="1.1" lon="2.1"/></trkseg></trk></gpx>"#;
        let track = Track::parse_gpx(text).unwrap();
        assert_eq!(track.name, "");
        assert_eq!(track.segments[0][0].time, Some(1_577_934_245));
        assert!(Track::parse_gpx(r#"<gpx><wpt lat="1" lon="2"/></gpx>"#).is_err());
        assert!(Track::parse_gpx("<gpx><trk>").is_err());
    }

    #[test]
    fn kml_lines_and_gx_tracks_are_read() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2">
  <Document>
    <name>Holiday</name>
    <Placemark><name>Lake</name><Polygon><outerBoundaryIs><LinearRing>
      <coordinates>0,0 1,0 1,1 0,0</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark>
    <Placemark>
      <name>Walk</name>
      <MultiGeometry>
        <LineString><coordinates>
          121.5,25.0,12 121.6,25.1,13
          121.7,25.2
        </coordinates></LineString>
      </MultiGeometry>
    </Placemark>
    <Placemark>
      <gx:MultiTrack><gx:Track>
        <when>2024-05-01T08:00:00Z</when><when>2024-05-01T08:00:10Z</when>
        <gx:coord>121.5 25.0 10</gx:coord><gx:coord>121.5001 25.0001 11</gx:coord>
      </gx:Track></gx:MultiTrack>
    </Placemark>
  </Document>
</kml>"#;
        let track = Track::parse_kml(text).unwrap();
        assert_eq!(track.name, "Walk");
        assert_eq!(track.segments.len(), 2);
        assert_eq!(track.segments[0].len(), 3);
        assert_eq!(track.segments[0][2], Point { lat: 25.2, lon: 121.7, time: None });
        assert_eq!(track.segments[1][1].time, Some(1_714_550_410));
        assert!(Track::parse_kml(r#"<kml><Placemark><Point><coordinates>1,2</coordinates></Point></Placemark></kml>"#).is_err());
    }

    #[test]
    fn encoded_points_come_back() {
        let track = Track::parse_gpx(GPX).unwrap();
        let bytes = track.encode();
        let back = Track::decode(track.name.clone(), &bytes);
        assert_eq!(back.segments.len(), track.segments.len());
        for (a, b) in back.points().zip(track.points()) {
            assert!((a.lat - b.lat).abs() < 1e-7 && (a.lon - b.lon).abs() < 1e-7);
            assert_eq!(a.time, b.time);
        }
        // A cut-off blob keeps the whole segments before the cut.
        assert_eq!(Track::decode(String::new(), &bytes[..bytes.len() - 3]).segments.len(), 2);
    }

    #[test]
    fn distance_and_bounds() {
        let a = Point { lat: 0.0, lon: 0.0, time: None };
        let b = Point { lat: 0.0, lon: 1.0, time: None };
        assert!((distance_m(&a, &b) - 111_195.0).abs() < 10.0);
        assert_eq!(format_distance(850.4), "850 m");
        assert_eq!(format_distance(12_345.0), "12.3 km");
        let map = MapTrack::new(1, 1, String::new(), &Track { name: "x".into(), segments: vec![vec![a, b]] });
        assert_eq!(map.bounds.min, [128.0, 128.0]);
        assert!((map.bounds.max[0] - 128.0 - 256.0 / 360.0).abs() < 1e-9);
        assert!(is_track_path(Path::new("ride.GPX")) && is_track_path(Path::new("a.kml")) && !is_track_path(Path::new("a.kmz")));
    }
}
