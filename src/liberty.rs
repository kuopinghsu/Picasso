//! OpenFreeMap vector tiles drawn in the colors of its Liberty style, with
//! the water in the aqua of the app's travel colors.
//!
//! Shapes are painted into the tile image. Names come back as labels, which
//! the window draws on top with its own fonts, so they stay sharp, read in
//! every script, and don't collide across tiles.

use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform};

use crate::i18n::Lang;
use crate::mvt::{self, Feature, Kind, Layer};

/// Pixels per side of a drawn tile, twice the 256 points it covers, so
/// lines stay sharp on high-density screens.
pub const TILE_SIZE: u32 = 512;
/// Deepest zoom OpenFreeMap serves. Closer views enlarge a part of a tile
/// at this zoom.
pub const DATA_ZOOM: u8 = 14;
/// Drawn pixels per pixel of a 256 px web map tile, the unit of the widths.
const PX: f32 = TILE_SIZE as f32 / 256.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LabelKind {
    Country,
    State,
    City,
    Town,
    Village,
    Suburb,
    Water,
    Road,
}

/// A name to write over the map.
#[derive(Clone, Debug, PartialEq)]
pub struct MapLabel {
    pub text: String,
    pub kind: LabelKind,
    /// Position in the tile, 0 to 1 across and down.
    pub pos: [f32; 2],
    /// Clockwise angle of a road name, in radians, kept upright.
    pub angle: f32,
    /// Length of the road piece the name runs along, as a fraction of the
    /// tile width; 0 for names on a point.
    pub span: f32,
    /// Lower goes first when names overlap.
    pub rank: u32,
}

/// Which part of the data tile to draw: the cell at `x`, `y` of a
/// `2^depth` by `2^depth` grid over it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Part {
    pub depth: u8,
    pub x: u32,
    pub y: u32,
}

impl Part {
    /// Data tile and part for the map tile `z`/`x`/`y`.
    pub fn of(z: u8, x: u32, y: u32) -> ((u8, u32, u32), Part) {
        let depth = z.saturating_sub(DATA_ZOOM);
        let mask = (1u32 << depth) - 1;
        ((z - depth, x >> depth, y >> depth), Part { depth, x: x & mask, y: y & mask })
    }
}

pub struct Drawn {
    /// Straight RGBA, `TILE_SIZE` square.
    pub rgba: Vec<u8>,
    pub labels: Vec<MapLabel>,
}

/// Property keys tried for a name, before `name`, in the app language.
pub fn name_keys(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::English => &["name:en", "name_en"],
        Lang::TraditionalChinese => &["name:zh-Hant", "name:zh"],
        Lang::SimplifiedChinese => &["name:zh-Hans", "name:zh"],
        Lang::Japanese => &["name:ja"],
        Lang::Korean => &["name:ko"],
        Lang::German => &["name:de", "name_de"],
        Lang::Spanish => &["name:es"],
        Lang::French => &["name:fr"],
        Lang::Italian => &["name:it"],
        Lang::Portuguese => &["name:pt"],
    }
}

/// Maps tile units of one layer to pixels of the drawn tile.
#[derive(Clone, Copy)]
struct Frame {
    scale: f32,
    dx: f32,
    dy: f32,
}

impl Frame {
    fn new(extent: u32, part: Part) -> Frame {
        let grow = (1u32 << part.depth) as f32;
        let size = TILE_SIZE as f32;
        Frame {
            scale: size * grow / extent as f32,
            dx: part.x as f32 * size,
            dy: part.y as f32 * size,
        }
    }

    fn map(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        [x * self.scale - self.dx, y * self.scale - self.dy]
    }
}

/// Draws the vector tile `data` as map tile zoom `zoom`.
pub fn draw(data: &[u8], zoom: u8, part: Part, names: &[&str]) -> Option<Drawn> {
    let layers = mvt::decode(data);
    let mut pixmap = Pixmap::new(TILE_SIZE, TILE_SIZE)?;
    pixmap.fill(rgb(0xF8, 0xF4, 0xF0));
    let z = f32::from(zoom);
    let layer = |name: &str| layers.iter().find(|layer| layer.name == name);
    let mut canvas = Canvas { pixmap: &mut pixmap, part };

    if let Some(landcover) = layer("landcover") {
        for (class, color) in [
            ("farmland", rgba(0xEE, 0xF0, 0xD5, 0.6)),
            ("grass", rgba(0xCD, 0xE6, 0xB4, 0.55)),
            ("wood", rgba(0x9C, 0xCB, 0x80, 0.4)),
            ("wetland", rgba(0xC8, 0xE1, 0xDC, 0.6)),
            ("sand", rgba(0xF5, 0xEE, 0xCD, 0.8)),
            ("ice", rgba(0xFF, 0xFF, 0xFF, 0.8)),
            ("rock", rgba(0xE2, 0xDE, 0xD8, 0.7)),
        ] {
            canvas.fill(landcover, |feature| landcover.text(feature, "class") == Some(class), color);
        }
    }
    if let Some(landuse) = layer("landuse") {
        for (classes, color) in [
            (&["residential", "suburb", "neighbourhood"][..], rgba(0xEA, 0xE2, 0xD8, 0.5)),
            (&["commercial", "retail", "industrial", "railway"][..], rgba(0xF2, 0xDE, 0xDE, 0.45)),
            (&["cemetery"][..], rgb(0xE0, 0xE4, 0xDD)),
            (&["hospital"][..], rgb(0xFD, 0xE8, 0xEC)),
            (&["school", "university", "college", "kindergarten"][..], rgb(0xF0, 0xF0, 0xD8)),
            (&["stadium", "pitch", "track", "playground"][..], rgb(0xD4, 0xEB, 0xCB)),
        ] {
            canvas.fill(landuse, |feature| landuse.text(feature, "class").is_some_and(|class| classes.contains(&class)), color);
        }
    }
    if let Some(park) = layer("park") {
        canvas.fill(park, |_| true, rgba(0xD8, 0xE8, 0xC8, 0.7));
    }
    let water = rgb(0xAA, 0xD4, 0xDB);
    if let Some(waters) = layer("water") {
        canvas.fill(waters, |_| true, water);
    }
    if let Some(waterway) = layer("waterway") {
        for (classes, stops) in [
            (&["stream", "drain", "ditch"][..], &[(12.0, 0.3), (14.0, 0.8), (18.0, 3.0)][..]),
            (&["river", "canal"][..], &[(8.0, 0.5), (12.0, 1.2), (14.0, 2.0), (18.0, 8.0)][..]),
        ] {
            let width = ramp(z, stops);
            canvas.stroke(
                waterway,
                |feature| waterway.text(feature, "class").is_some_and(|class| classes.contains(&class)),
                water,
                width,
                None,
            );
        }
    }
    if let Some(aeroway) = layer("aeroway") {
        canvas.fill(aeroway, |feature| feature.kind == Kind::Polygon, rgba(0xE5, 0xE4, 0xE0, 0.8));
        for (class, stops) in [("taxiway", &[(12.0, 0.5), (16.0, 6.0)][..]), ("runway", &[(10.0, 1.0), (16.0, 20.0)][..])] {
            canvas.stroke(
                aeroway,
                |feature| feature.kind == Kind::Line && aeroway.text(feature, "class") == Some(class),
                rgb(0xFF, 0xFF, 0xFF),
                ramp(z, stops),
                None,
            );
        }
    }
    if zoom >= 13 {
        if let Some(building) = layer("building") {
            canvas.fill(building, |_| true, rgb(0xE6, 0xDD, 0xD3));
            if zoom >= 15 {
                canvas.stroke(building, |_| true, rgb(0xD5, 0xCB, 0xC1), 0.5, None);
            }
        }
    }
    if let Some(roads) = layer("transportation") {
        draw_roads(&mut canvas, roads, z);
    }
    if let Some(boundary) = layer("boundary") {
        let on_land = |feature: &Feature| boundary.number(feature, "maritime").unwrap_or(0.0) == 0.0;
        let level = |feature: &Feature| boundary.number(feature, "admin_level").unwrap_or(99.0);
        canvas.stroke(
            boundary,
            |feature| on_land(feature) && (3.0..=4.0).contains(&level(feature)),
            rgb(0xB8, 0xB4, 0xC6),
            ramp(z, &[(4.0, 0.5), (10.0, 1.0), (14.0, 1.5)]),
            Some([3.0, 2.0]),
        );
        canvas.stroke(
            boundary,
            |feature| on_land(feature) && level(feature) <= 2.0,
            rgb(0x9E, 0x9C, 0xAB),
            ramp(z, &[(2.0, 0.6), (8.0, 1.4), (14.0, 2.2)]),
            None,
        );
    }

    let frame_of = |layer: &Layer| Frame::new(layer.extent, part);
    let mut labels = Vec::new();
    if let Some(places) = layer("place") {
        place_labels(places, frame_of(places), zoom, names, &mut labels);
    }
    if let Some(waters) = layer("water_name") {
        point_labels(waters, frame_of(waters), names, LabelKind::Water, &mut labels);
    }
    if zoom >= 13 {
        if let Some(roads) = layer("transportation_name") {
            road_labels(roads, frame_of(roads), names, &mut labels);
        }
    }
    Some(Drawn { rgba: pixmap.take(), labels })
}

/// Roads under bridges first and tunnels faded; within each, every
/// casing goes down before any fill so crossings join cleanly.
fn draw_roads(canvas: &mut Canvas, roads: &Layer, z: f32) {
    struct Road {
        classes: &'static [&'static str],
        from: f32,
        width: &'static [(f32, f32)],
        fill: Color,
        casing: Option<Color>,
    }
    let main_casing = rgb(0xE9, 0xAC, 0x77);
    let minor_casing = rgb(0xCF, 0xCD, 0xCA);
    let styles = [
        Road {
            classes: &["service"],
            from: 13.0,
            width: &[(13.0, 0.4), (14.0, 1.2), (16.0, 3.0), (18.0, 7.0)],
            fill: rgb(0xFF, 0xFF, 0xFF),
            casing: Some(minor_casing),
        },
        Road {
            classes: &["minor", "busway", "raceway"],
            from: 11.0,
            width: &[(11.0, 0.3), (13.0, 1.0), (14.0, 2.5), (16.0, 5.5), (18.0, 12.0)],
            fill: rgb(0xFF, 0xFF, 0xFF),
            casing: Some(minor_casing),
        },
        Road {
            classes: &["secondary", "tertiary"],
            from: 8.0,
            width: &[(8.0, 0.3), (10.0, 0.8), (12.0, 1.5), (14.0, 3.5), (16.0, 7.0), (18.0, 14.0)],
            fill: rgb(0xFF, 0xEE, 0xAA),
            casing: Some(main_casing),
        },
        Road {
            classes: &["trunk", "primary"],
            from: 7.0,
            width: &[(7.0, 0.3), (8.0, 0.6), (12.0, 2.0), (14.0, 4.5), (16.0, 8.0), (18.0, 16.0)],
            fill: rgb(0xFF, 0xEE, 0xAA),
            casing: Some(main_casing),
        },
        Road {
            classes: &["motorway"],
            from: 4.0,
            width: &[(4.0, 0.4), (8.0, 1.0), (12.0, 2.5), (14.0, 5.0), (16.0, 9.0), (18.0, 18.0)],
            fill: rgb(0xFF, 0xCC, 0x88),
            casing: Some(main_casing),
        },
    ];
    let brunnel = |feature: &Feature| roads.text(feature, "brunnel").unwrap_or("");
    for level in ["tunnel", "", "bridge"] {
        let on_level = |feature: &Feature| match level {
            "" => !matches!(brunnel(feature), "tunnel" | "bridge"),
            _ => brunnel(feature) == level,
        };
        let fade = if level == "tunnel" { 0.45 } else { 1.0 };
        if z >= 14.0 {
            for (class, color, dash) in [
                ("path", rgb(0xC6, 0xB9, 0xA9), Some([2.0, 2.0])),
                ("track", rgb(0xB8, 0xA8, 0x8E), Some([3.0, 2.0])),
            ] {
                canvas.stroke(roads, |feature| on_level(feature) && roads.text(feature, "class") == Some(class), color, 1.0, dash);
            }
        }
        // Ramps (slip roads) are drawn narrower than their class.
        let is_ramp = |feature: &Feature| roads.number(feature, "ramp").unwrap_or(0.0) != 0.0;
        let visible = |style: &Road, ramp_road: bool, feature: &Feature| {
            on_level(feature)
                && is_ramp(feature) == ramp_road
                && roads.text(feature, "class").is_some_and(|class| style.classes.contains(&class))
        };
        let width = |style: &Road, ramp_road: bool| ramp(z, style.width) * if ramp_road { 0.6 } else { 1.0 };
        let edge = ramp(z, &[(10.0, 0.5), (14.0, 1.0), (18.0, 2.0)]);
        let shown = || styles.iter().filter(|style| z >= style.from).flat_map(|style| [(style, false), (style, true)]);
        if z >= 10.0 {
            for (style, ramp_road) in shown() {
                let Some(casing) = style.casing else { continue };
                canvas.stroke(
                    roads,
                    |feature| visible(style, ramp_road, feature),
                    faded(casing, fade),
                    width(style, ramp_road) + edge * 2.0,
                    (level == "tunnel").then_some([4.0, 2.0]),
                );
            }
        }
        for (style, ramp_road) in shown() {
            canvas.stroke(
                roads,
                |feature| visible(style, ramp_road, feature),
                faded(style.fill, fade),
                width(style, ramp_road),
                None,
            );
        }
        canvas.stroke(
            roads,
            |feature| on_level(feature) && z >= 10.0 && matches!(roads.text(feature, "class"), Some("rail" | "transit")),
            faded(rgb(0xB5, 0xB5, 0xB5), fade),
            ramp(z, &[(10.0, 0.5), (14.0, 1.0), (18.0, 2.5)]),
            None,
        );
    }
    if z >= 10.0 {
        canvas.stroke(
            roads,
            |feature| roads.text(feature, "class") == Some("ferry"),
            rgb(0x6C, 0x9F, 0xB6),
            1.0,
            Some([4.0, 4.0]),
        );
    }
}

struct Canvas<'a> {
    pixmap: &'a mut Pixmap,
    part: Part,
}

impl Canvas<'_> {
    /// Fills the polygons of `layer` that pass `pick`, all as one path.
    fn fill(&mut self, layer: &Layer, pick: impl Fn(&Feature) -> bool, color: Color) {
        let frame = Frame::new(layer.extent, self.part);
        let mut path = PathBuilder::new();
        for feature in layer.features.iter().filter(|feature| feature.kind == Kind::Polygon && pick(feature)) {
            for ring in feature.geometry() {
                if ring.len() < 3 {
                    continue;
                }
                let [x, y] = frame.map(ring[0]);
                path.move_to(x, y);
                for point in &ring[1..] {
                    let [x, y] = frame.map(*point);
                    path.line_to(x, y);
                }
                path.close();
            }
        }
        let Some(path) = path.finish() else { return };
        // Holes wind the other way, so the nonzero rule leaves them open.
        self.pixmap.fill_path(&path, &paint(color), FillRule::Winding, Transform::identity(), None);
    }

    /// Strokes the lines (and polygon outlines) of `layer` that pass `pick`.
    /// `width` and `dash` are in pixels of a 256 px tile.
    fn stroke(&mut self, layer: &Layer, pick: impl Fn(&Feature) -> bool, color: Color, width: f32, dash: Option<[f32; 2]>) {
        if width <= 0.0 {
            return;
        }
        let frame = Frame::new(layer.extent, self.part);
        let mut path = PathBuilder::new();
        for feature in layer.features.iter().filter(|feature| feature.kind != Kind::Point && pick(feature)) {
            for line in feature.geometry() {
                if line.len() < 2 {
                    continue;
                }
                let [x, y] = frame.map(line[0]);
                path.move_to(x, y);
                for point in &line[1..] {
                    let [x, y] = frame.map(*point);
                    path.line_to(x, y);
                }
            }
        }
        let Some(path) = path.finish() else { return };
        let stroke = Stroke {
            width: (width * PX).max(0.5),
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            dash: dash.and_then(|[on, off]| StrokeDash::new(vec![on * PX, off * PX], 0.0)),
            ..Stroke::default()
        };
        self.pixmap.stroke_path(&path, &paint(color), &stroke, Transform::identity(), None);
    }
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn rgba(r: u8, g: u8, b: u8, alpha: f32) -> Color {
    Color::from_rgba8(r, g, b, (alpha * 255.0).round() as u8)
}

fn faded(mut color: Color, alpha: f32) -> Color {
    color.apply_opacity(alpha);
    color
}

/// Value at zoom `z` along straight lines between `(zoom, value)` stops,
/// held flat past either end.
fn ramp(z: f32, stops: &[(f32, f32)]) -> f32 {
    let Some(&(first_z, first)) = stops.first() else { return 0.0 };
    if z <= first_z {
        return first;
    }
    for pair in stops.windows(2) {
        let ((z0, v0), (z1, v1)) = (pair[0], pair[1]);
        if z <= z1 {
            return v0 + (v1 - v0) * (z - z0) / (z1 - z0);
        }
    }
    stops.last().map_or(first, |&(_, value)| value)
}

fn label_name(layer: &Layer, feature: &Feature, names: &[&str]) -> Option<String> {
    names
        .iter()
        .chain(&["name"])
        .find_map(|key| layer.text(feature, key).map(str::trim).filter(|text| !text.is_empty()))
        .map(str::to_string)
}

/// Tile fraction of a point, when it lies inside the tile.
fn inside(frame: Frame, point: [f32; 2]) -> Option<[f32; 2]> {
    let [x, y] = frame.map(point);
    let size = TILE_SIZE as f32;
    let pos = [x / size, y / size];
    ((0.0..1.0).contains(&pos[0]) && (0.0..1.0).contains(&pos[1])).then_some(pos)
}

fn place_labels(layer: &Layer, frame: Frame, zoom: u8, names: &[&str], out: &mut Vec<MapLabel>) {
    for feature in layer.features.iter().filter(|feature| feature.kind == Kind::Point) {
        let kind = match layer.text(feature, "class") {
            Some("country") => LabelKind::Country,
            Some("state" | "province") if zoom >= 4 => LabelKind::State,
            Some("city") => LabelKind::City,
            Some("town") if zoom >= 6 => LabelKind::Town,
            Some("village") if zoom >= 10 => LabelKind::Village,
            Some("suburb" | "quarter") if zoom >= 11 => LabelKind::Suburb,
            Some("neighbourhood") if zoom >= 14 => LabelKind::Suburb,
            _ => continue,
        };
        let Some(text) = label_name(layer, feature, names) else { continue };
        let rank = layer.number(feature, "rank").unwrap_or(30.0).clamp(0.0, 99.0) as u32;
        for point in feature.geometry().into_iter().flatten() {
            if let Some(pos) = inside(frame, point) {
                out.push(MapLabel { text: text.clone(), kind, pos, angle: 0.0, span: 0.0, rank: kind as u32 * 100 + rank });
            }
        }
    }
}

fn point_labels(layer: &Layer, frame: Frame, names: &[&str], kind: LabelKind, out: &mut Vec<MapLabel>) {
    for feature in &layer.features {
        let Some(text) = label_name(layer, feature, names) else { continue };
        let parts = feature.geometry();
        let point = match feature.kind {
            Kind::Point => parts.first().and_then(|part| part.first().copied()),
            _ => parts.first().and_then(|part| part.get(part.len() / 2).copied()),
        };
        if let Some(pos) = point.and_then(|point| inside(frame, point)) {
            out.push(MapLabel { text, kind, pos, angle: 0.0, span: 0.0, rank: kind as u32 * 100 });
        }
    }
}

/// One name per road, on its longest straight piece inside the tile.
fn road_labels(layer: &Layer, frame: Frame, names: &[&str], out: &mut Vec<MapLabel>) {
    let size = TILE_SIZE as f32;
    for feature in layer.features.iter().filter(|feature| feature.kind == Kind::Line) {
        let Some(text) = label_name(layer, feature, names) else { continue };
        let importance = match layer.text(feature, "class") {
            Some("motorway") => 0,
            Some("trunk" | "primary") => 1,
            Some("secondary" | "tertiary") => 2,
            _ => 3,
        };
        let mut best: Option<([f32; 2], [f32; 2], f32)> = None;
        for line in feature.geometry() {
            for pair in line.windows(2) {
                let (a, b) = (frame.map(pair[0]), frame.map(pair[1]));
                let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                let within = (0.0..size).contains(&middle[0]) && (0.0..size).contains(&middle[1]);
                if within && best.is_none_or(|(_, _, longest)| length > longest) {
                    best = Some((a, b, length));
                }
            }
        }
        let Some((a, b, length)) = best else { continue };
        let mut angle = (b[1] - a[1]).atan2(b[0] - a[0]);
        if angle > std::f32::consts::FRAC_PI_2 {
            angle -= std::f32::consts::PI;
        } else if angle < -std::f32::consts::FRAC_PI_2 {
            angle += std::f32::consts::PI;
        }
        out.push(MapLabel {
            text,
            kind: LabelKind::Road,
            pos: [(a[0] + b[0]) / 2.0 / size, (a[1] + b[1]) / 2.0 / size],
            angle,
            span: length / size,
            rank: LabelKind::Road as u32 * 100 + importance,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvt::tests::{command, tile, zz};

    #[test]
    fn deep_zooms_enlarge_a_part_of_the_data_tile() {
        assert_eq!(Part::of(12, 5, 9), ((12, 5, 9), Part::default()));
        assert_eq!(Part::of(14, 5, 9), ((14, 5, 9), Part::default()));
        assert_eq!(Part::of(16, 13, 6), ((14, 3, 1), Part { depth: 2, x: 1, y: 2 }));
        let frame = Frame::new(4096, Part { depth: 1, x: 1, y: 0 });
        assert_eq!(frame.map([2048.0, 1024.0]), [0.0, 256.0]);
    }

    #[test]
    fn ramps_hold_past_their_ends() {
        let stops = [(10.0, 1.0), (14.0, 5.0)];
        assert_eq!(ramp(8.0, &stops), 1.0);
        assert_eq!(ramp(12.0, &stops), 3.0);
        assert_eq!(ramp(20.0, &stops), 5.0);
    }

    #[test]
    fn draws_water_and_names_places_in_the_app_language() {
        let lake = vec![command(1, 1), zz(0), zz(0), command(2, 3), zz(2048), zz(0), zz(0), zz(2048), zz(-2048), zz(0), command(7, 1)];
        let mut data = tile("water", &[], &[], &[(3, vec![], lake)]);
        data.extend(tile(
            "place",
            &["class", "name", "name:en"],
            &["city", "臺北市", "Taipei"],
            &[(1, vec![0, 0, 1, 1, 2, 2], vec![command(1, 1), zz(1024), zz(3072)])],
        ));
        let drawn = draw(&data, 10, Part::default(), &["name:en"]).unwrap();
        assert_eq!(drawn.rgba.len(), (TILE_SIZE * TILE_SIZE * 4) as usize);
        let pixel = |x: u32, y: u32| {
            let at = ((y * TILE_SIZE + x) * 4) as usize;
            [drawn.rgba[at], drawn.rgba[at + 1], drawn.rgba[at + 2], drawn.rgba[at + 3]]
        };
        assert_eq!(pixel(100, 100), [0xAA, 0xD4, 0xDB, 255], "water");
        assert_eq!(pixel(400, 400), [0xF8, 0xF4, 0xF0, 255], "land");
        assert_eq!(drawn.labels.len(), 1);
        assert_eq!(drawn.labels[0].text, "Taipei");
        assert_eq!(drawn.labels[0].kind, LabelKind::City);
        assert_eq!(drawn.labels[0].pos, [0.25, 0.75]);
        let local = draw(&data, 10, Part::default(), &["name:ja"]).unwrap();
        assert_eq!(local.labels[0].text, "臺北市");
    }

    #[test]
    fn road_names_lie_along_the_road_and_stay_upright() {
        let road = vec![command(1, 1), zz(3000), zz(1000), command(2, 2), zz(-2000), zz(0), zz(-100), zz(100)];
        let data = tile("transportation_name", &["class", "name"], &["primary", "Zhongshan Rd"], &[(2, vec![0, 0, 1, 1], road)]);
        let drawn = draw(&data, 14, Part::default(), &[]).unwrap();
        let label = &drawn.labels[0];
        assert_eq!(label.kind, LabelKind::Road);
        assert!(label.angle.abs() < 1e-6, "drawn right to left, read left to right");
        assert!((label.span - 2000.0 / 4096.0).abs() < 1e-4);
        assert_eq!(label.pos, [2000.0 / 4096.0, 1000.0 / 4096.0]);
    }
}
