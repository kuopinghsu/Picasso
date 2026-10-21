//! Reader for the `.picasa.ini` sidecar Picasa wrote next to each folder.
//!
//! Stars, captions, rotations, and named face rectangles are imported so an
//! existing Picasa library opens with the same organization.

use std::collections::HashMap;
use std::path::Path;

use crate::types::RelRect;

#[derive(Clone, Debug, Default)]
pub struct PicasaIni {
    photos: HashMap<String, IniPhoto>,
}

#[derive(Clone, Debug, Default)]
pub struct IniPhoto {
    pub star: Option<bool>,
    pub caption: Option<String>,
    pub rotation: Option<i32>,
    pub faces: Option<Vec<IniFace>>,
}

#[derive(Clone, Debug)]
pub struct IniFace {
    pub rect: RelRect,
    pub contact: String,
    pub name: Option<String>,
}

impl PicasaIni {
    pub fn load(dir: &Path) -> Self {
        for name in [".picasa.ini", "Picasa.ini", ".Picasa.ini"] {
            let path = dir.join(name);
            if path.is_file() {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                return Self::parse(&text);
            }
        }
        Self::default()
    }

    pub fn get(&self, filename: &str) -> Option<&IniPhoto> {
        self.photos.get(&filename.to_lowercase())
    }

    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut current: Option<(String, Vec<(String, String)>)> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                if let Some(section) = current.take() {
                    sections.push(section);
                }
                current = Some((line[1..line.len() - 1].trim().to_string(), Vec::new()));
                continue;
            }
            if let Some((_, keys)) = current.as_mut() {
                if let Some((key, value)) = line.split_once('=') {
                    keys.push((key.trim().to_string(), value.trim().to_string()));
                }
            }
        }
        if let Some(section) = current {
            sections.push(section);
        }

        let mut contacts: HashMap<String, String> = HashMap::new();
        for (name, keys) in &sections {
            if name.eq_ignore_ascii_case("contacts") || name.eq_ignore_ascii_case("contacts2") {
                for (key, value) in keys {
                    if !value.is_empty() {
                        contacts.insert(key.clone(), value.clone());
                    }
                }
            }
        }

        let mut photos = HashMap::new();
        for (name, keys) in &sections {
            if name.eq_ignore_ascii_case("picasa")
                || name.eq_ignore_ascii_case("contacts")
                || name.eq_ignore_ascii_case("contacts2")
            {
                continue;
            }
            let mut photo = IniPhoto::default();
            for (key, value) in keys {
                match key.to_ascii_lowercase().as_str() {
                    "star" => photo.star = parse_star(value),
                    "caption" => photo.caption = Some(value.clone()),
                    "rotate" | "transforms" => photo.rotation = parse_rotation(value),
                    "faces" => photo.faces = Some(parse_faces(value, &contacts)),
                    _ => {}
                }
            }
            photos.insert(name.to_lowercase(), photo);
        }
        Self { photos }
    }
}

fn parse_star(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// Picasa stores `rotate(0..3)` as quarter-turns clockwise.
pub fn parse_rotation(value: &str) -> Option<i32> {
    let value = value.trim();
    let number = value
        .strip_prefix("rotate(")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(value)
        .trim();
    match number {
        "0" => Some(0),
        "1" => Some(90),
        "2" => Some(180),
        "3" => Some(270),
        _ => None,
    }
}

fn parse_faces(value: &str, contacts: &HashMap<String, String>) -> Vec<IniFace> {
    let mut faces = Vec::new();
    for part in value.split(';') {
        let part = part.trim();
        let Some(rest) = part.strip_prefix("rect64(") else {
            continue;
        };
        let Some((hex, after)) = rest.split_once(')') else {
            continue;
        };
        let Some(rect) = decode_rect64(hex) else {
            continue;
        };
        let contact = after.trim().trim_start_matches(',').trim().to_string();
        let name = contacts.get(&contact).cloned().filter(|n| !n.is_empty());
        faces.push(IniFace { rect, contact, name });
    }
    faces
}

/// Picasa packs left, top, right, bottom as four 16-bit fractions of the image.
pub fn decode_rect64(hex: &str) -> Option<RelRect> {
    let hex = hex.trim();
    if hex.len() != 16 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u64::from_str_radix(hex, 16).ok()?;
    let left = ((n >> 48) & 0xffff) as f32 / 65536.0;
    let top = ((n >> 32) & 0xffff) as f32 / 65536.0;
    let right = ((n >> 16) & 0xffff) as f32 / 65536.0;
    let bottom = (n & 0xffff) as f32 / 65536.0;
    if right <= left || bottom <= top {
        return None;
    }
    Some(RelRect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn encode_rect64(rect: RelRect) -> String {
    let left = (rect.x * 65536.0).round().clamp(0.0, 65535.0) as u64;
    let top = (rect.y * 65536.0).round().clamp(0.0, 65535.0) as u64;
    let right = ((rect.x + rect.w) * 65536.0)
        .round()
        .clamp(0.0, 65535.0) as u64;
    let bottom = ((rect.y + rect.h) * 65536.0)
        .round()
        .clamp(0.0, 65535.0) as u64;
    format!("{left:04x}{top:04x}{right:04x}{bottom:04x}")
}

pub fn unnamed_contact(contact: &str) -> bool {
    let trimmed = contact.trim();
    trimmed.is_empty()
        || trimmed.chars().all(|c| c == '0')
        || trimmed.chars().all(|c| c.eq_ignore_ascii_case(&'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect64_roundtrip() {
        let rect = RelRect {
            x: 0.25,
            y: 0.10,
            w: 0.20,
            h: 0.30,
        };
        let decoded = decode_rect64(&encode_rect64(rect)).unwrap();
        assert!((decoded.x - rect.x).abs() < 0.001);
        assert!((decoded.y - rect.y).abs() < 0.001);
        assert!((decoded.w - rect.w).abs() < 0.001);
        assert!((decoded.h - rect.h).abs() < 0.001);
    }

    #[test]
    fn parses_star_caption_rotation_and_named_face() {
        let ini = PicasaIni::parse(
            "\
[Contacts]
abc=Ada Lovelace
[IMG_0001.JPG]
star=yes
caption=On the steps
rotate=rotate(1)
faces=rect64(4000100080006000),abc
[IMG_0002.JPG]
faces=
",
        );
        let photo = ini.get("img_0001.jpg").unwrap();
        assert_eq!(photo.star, Some(true));
        assert_eq!(photo.caption.as_deref(), Some("On the steps"));
        assert_eq!(photo.rotation, Some(90));
        let face = &photo.faces.as_ref().unwrap()[0];
        assert_eq!(face.name.as_deref(), Some("Ada Lovelace"));
        assert!(face.rect.w > 0.0);
        assert!(ini.get("IMG_0002.JPG").unwrap().faces.as_ref().unwrap().is_empty());
    }
}
