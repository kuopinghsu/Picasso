//! Mapbox Vector Tile decoding: layers, feature tags, and geometry.
//!
//! Only what the map needs is read; unknown fields are skipped. The format
//! is protocol buffers, read here by hand rather than through generated code.

/// A property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Number(f64),
    Bool(bool),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(number) => Some(*number),
            Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
            Value::Text(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Unknown,
    Point,
    Line,
    Polygon,
}

#[derive(Debug, Default)]
pub struct Layer {
    pub name: String,
    pub extent: u32,
    keys: Vec<String>,
    values: Vec<Value>,
    pub features: Vec<Feature>,
}

#[derive(Debug)]
pub struct Feature {
    tags: Vec<u32>,
    pub kind: Kind,
    commands: Vec<u32>,
}

impl Layer {
    /// Property `key` of `feature`.
    pub fn get<'a>(&'a self, feature: &Feature, key: &str) -> Option<&'a Value> {
        feature.tags.as_chunks::<2>().0.iter().find_map(|&[k, v]| {
            (self.keys.get(k as usize)? == key).then(|| self.values.get(v as usize)).flatten()
        })
    }

    pub fn text<'a>(&'a self, feature: &Feature, key: &str) -> Option<&'a str> {
        self.get(feature, key).and_then(Value::as_str)
    }

    pub fn number(&self, feature: &Feature, key: &str) -> Option<f64> {
        self.get(feature, key).and_then(Value::as_f64)
    }
}

impl Feature {
    /// Point lists in tile units: one per point for points, one per line,
    /// and one per ring for polygons.
    pub fn geometry(&self) -> Vec<Vec<[f32; 2]>> {
        let mut parts: Vec<Vec<[f32; 2]>> = Vec::new();
        let (mut x, mut y) = (0i32, 0i32);
        let mut at = 0;
        while at < self.commands.len() {
            let header = self.commands[at];
            at += 1;
            let (command, count) = (header & 7, (header >> 3) as usize);
            match command {
                1 | 2 => {
                    for _ in 0..count {
                        let (Some(&dx), Some(&dy)) = (self.commands.get(at), self.commands.get(at + 1)) else {
                            return parts;
                        };
                        at += 2;
                        x = x.wrapping_add(zigzag(dx));
                        y = y.wrapping_add(zigzag(dy));
                        let point = [x as f32, y as f32];
                        if command == 1 || parts.is_empty() {
                            parts.push(vec![point]);
                        } else if let Some(last) = parts.last_mut() {
                            last.push(point);
                        }
                    }
                }
                7 => {
                    if let Some(first) = parts.last().and_then(|part| part.first().copied()) {
                        if let Some(last) = parts.last_mut() {
                            last.push(first);
                        }
                    }
                }
                _ => return parts,
            }
        }
        parts
    }
}

fn zigzag(value: u32) -> i32 {
    ((value >> 1) as i32) ^ -((value & 1) as i32)
}

/// Every layer of a tile. Damaged data ends the read with what was decoded.
pub fn decode(data: &[u8]) -> Vec<Layer> {
    let mut layers = Vec::new();
    let mut reader = Reader::new(data);
    while let Some((field, wire)) = reader.key() {
        match (field, wire) {
            (3, 2) => match reader.bytes() {
                Some(bytes) => layers.push(layer(bytes)),
                None => break,
            },
            _ => {
                if reader.skip(wire).is_none() {
                    break;
                }
            }
        }
    }
    layers
}

fn layer(data: &[u8]) -> Layer {
    let mut layer = Layer { extent: 4096, ..Layer::default() };
    let mut reader = Reader::new(data);
    while let Some((field, wire)) = reader.key() {
        let read = match (field, wire) {
            (1, 2) => reader.bytes().map(|bytes| layer.name = String::from_utf8_lossy(bytes).into_owned()),
            (2, 2) => reader.bytes().map(|bytes| layer.features.push(feature(bytes))),
            (3, 2) => reader.bytes().map(|bytes| layer.keys.push(String::from_utf8_lossy(bytes).into_owned())),
            (4, 2) => reader.bytes().map(|bytes| layer.values.push(value(bytes))),
            (5, 0) => reader.varint().map(|extent| layer.extent = (extent as u32).max(1)),
            _ => reader.skip(wire),
        };
        if read.is_none() {
            break;
        }
    }
    layer
}

fn feature(data: &[u8]) -> Feature {
    let mut feature = Feature {
        tags: Vec::new(),
        kind: Kind::Unknown,
        commands: Vec::new(),
    };
    let mut reader = Reader::new(data);
    while let Some((field, wire)) = reader.key() {
        let read = match (field, wire) {
            (2, 2) => reader.packed(&mut feature.tags),
            (2, 0) => reader.varint().map(|tag| feature.tags.push(tag as u32)),
            (3, 0) => reader.varint().map(|kind| {
                feature.kind = match kind {
                    1 => Kind::Point,
                    2 => Kind::Line,
                    3 => Kind::Polygon,
                    _ => Kind::Unknown,
                }
            }),
            (4, 2) => reader.packed(&mut feature.commands),
            (4, 0) => reader.varint().map(|command| feature.commands.push(command as u32)),
            _ => reader.skip(wire),
        };
        if read.is_none() {
            break;
        }
    }
    feature
}

fn value(data: &[u8]) -> Value {
    let mut reader = Reader::new(data);
    let mut value = Value::Text(String::new());
    while let Some((field, wire)) = reader.key() {
        let read = match (field, wire) {
            (1, 2) => reader.bytes().map(|bytes| value = Value::Text(String::from_utf8_lossy(bytes).into_owned())),
            (2, 5) => reader.fixed32().map(|bits| value = Value::Number(f64::from(f32::from_bits(bits)))),
            (3, 1) => reader.fixed64().map(|bits| value = Value::Number(f64::from_bits(bits))),
            (4, 0) | (5, 0) => reader.varint().map(|number| value = Value::Number(number as i64 as f64)),
            (6, 0) => reader.varint().map(|number| {
                let signed = ((number >> 1) as i64) ^ -((number & 1) as i64);
                value = Value::Number(signed as f64);
            }),
            (7, 0) => reader.varint().map(|flag| value = Value::Bool(flag != 0)),
            _ => reader.skip(wire),
        };
        if read.is_none() {
            break;
        }
    }
    value
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, at: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.data.get(self.at)?;
            self.at += 1;
            value |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn key(&mut self) -> Option<(u32, u8)> {
        if self.at >= self.data.len() {
            return None;
        }
        let key = self.varint()?;
        Some(((key >> 3) as u32, (key & 7) as u8))
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.varint()?).ok()?;
        let end = self.at.checked_add(len)?;
        let bytes = self.data.get(self.at..end)?;
        self.at = end;
        Some(bytes)
    }

    fn fixed32(&mut self) -> Option<u32> {
        let bytes = self.data.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes(bytes.try_into().ok()?))
    }

    fn fixed64(&mut self) -> Option<u64> {
        let bytes = self.data.get(self.at..self.at + 8)?;
        self.at += 8;
        Some(u64::from_le_bytes(bytes.try_into().ok()?))
    }

    fn packed(&mut self, out: &mut Vec<u32>) -> Option<()> {
        let mut inner = Reader::new(self.bytes()?);
        while inner.at < inner.data.len() {
            out.push(inner.varint()? as u32);
        }
        Some(())
    }

    fn skip(&mut self, wire: u8) -> Option<()> {
        match wire {
            0 => self.varint().map(drop),
            1 => self.fixed64().map(drop),
            2 => self.bytes().map(drop),
            5 => self.fixed32().map(drop),
            _ => None,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn varint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    pub fn message(out: &mut Vec<u8>, field: u32, body: &[u8]) {
        varint(out, u64::from(field << 3 | 2));
        varint(out, body.len() as u64);
        out.extend_from_slice(body);
    }

    fn packed(values: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        for value in values {
            varint(&mut out, u64::from(*value));
        }
        out
    }

    pub fn command(id: u32, count: u32) -> u32 {
        id | count << 3
    }

    pub fn zz(value: i32) -> u32 {
        ((value << 1) ^ (value >> 31)) as u32
    }

    /// A one-layer tile with the given features, each a kind, tags and
    /// geometry, plus the layer's keys and string values.
    pub fn tile(name: &str, keys: &[&str], values: &[&str], features: &[(u32, Vec<u32>, Vec<u32>)]) -> Vec<u8> {
        let mut layer = Vec::new();
        message(&mut layer, 1, name.as_bytes());
        for (kind, tags, geometry) in features {
            let mut feature = Vec::new();
            message(&mut feature, 2, &packed(tags));
            varint(&mut feature, 3 << 3);
            varint(&mut feature, u64::from(*kind));
            message(&mut feature, 4, &packed(geometry));
            message(&mut layer, 2, &feature);
        }
        for key in keys {
            message(&mut layer, 3, key.as_bytes());
        }
        for text in values {
            let mut value = Vec::new();
            message(&mut value, 1, text.as_bytes());
            message(&mut layer, 4, &value);
        }
        varint(&mut layer, 5 << 3);
        varint(&mut layer, 4096);
        let mut tile = Vec::new();
        message(&mut tile, 3, &layer);
        tile
    }

    #[test]
    fn reads_layers_tags_and_geometry() {
        let square = vec![
            command(1, 1), zz(10), zz(20),
            command(2, 3), zz(100), zz(0), zz(0), zz(100), zz(-100), zz(0),
            command(7, 1),
        ];
        let data = tile("water", &["class", "name"], &["lake", "Sun Moon Lake"], &[(3, vec![0, 0, 1, 1], square)]);
        let layers = decode(&data);
        assert_eq!(layers.len(), 1);
        let layer = &layers[0];
        assert_eq!((layer.name.as_str(), layer.extent), ("water", 4096));
        let feature = &layer.features[0];
        assert_eq!(feature.kind, Kind::Polygon);
        assert_eq!(layer.text(feature, "class"), Some("lake"));
        assert_eq!(layer.text(feature, "name"), Some("Sun Moon Lake"));
        assert_eq!(layer.get(feature, "missing"), None);
        assert_eq!(
            feature.geometry(),
            vec![vec![[10.0, 20.0], [110.0, 20.0], [110.0, 120.0], [10.0, 120.0], [10.0, 20.0]]]
        );
    }

    #[test]
    fn damaged_data_stops_without_panicking() {
        let data = tile("roads", &[], &[], &[(2, vec![], vec![command(1, 1), zz(1), zz(1), command(2, 5), zz(3)])]);
        let layers = decode(&data);
        assert_eq!(layers[0].features[0].geometry(), vec![vec![[1.0, 1.0]]]);
        for cut in 0..data.len() {
            let _ = decode(&data[..cut]);
        }
        assert!(decode(&[0xFF, 0xFF, 0xFF]).is_empty());
    }
}
