//! Reader for the `.picasa.ini` sidecar Picasa wrote next to each folder.
//!
//! Stars, captions, and rotations are imported so an existing Picasa library opens with
//! the same organization.

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct PicasaIni {
    photos: HashMap<String, IniPhoto>,
}

#[derive(Clone, Debug, Default)]
pub struct IniPhoto {
    pub star: Option<bool>,
    pub caption: Option<String>,
    /// Clockwise degrees from `rotate=rotate(n)`.
    pub rotation: Option<i32>,
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
        let mut photos = HashMap::new();
        let mut current: Option<(String, IniPhoto)> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                if let Some((name, photo)) = current.take() {
                    photos.insert(name, photo);
                }
                let name = line[1..line.len() - 1].trim().to_lowercase();
                let skip = matches!(name.as_str(), "picasa" | "contacts" | "contacts2");
                current = if skip { None } else { Some((name, IniPhoto::default())) };
                continue;
            }
            let Some((_, photo)) = current.as_mut() else { continue };
            let Some((key, value)) = line.split_once('=') else { continue };
            match key.trim().to_ascii_lowercase().as_str() {
                "star" => photo.star = parse_star(value),
                "caption" => photo.caption = Some(value.trim().to_string()),
                "rotate" => photo.rotation = parse_rotation(value),
                _ => {}
            }
        }
        if let Some((name, photo)) = current {
            photos.insert(name, photo);
        }
        Self { photos }
    }
}

/// Picasa stores `rotate(0..3)` as quarter turns clockwise.
fn parse_rotation(value: &str) -> Option<i32> {
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

fn parse_star(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_star_caption_and_rotation() {
        let ini = PicasaIni::parse(
            "\
[Contacts]
abc=Ada Lovelace
[IMG_0001.JPG]
star=yes
caption=On the steps
rotate=rotate(1)
[IMG_0002.JPG]
star=no
",
        );
        let photo = ini.get("img_0001.jpg").unwrap();
        assert_eq!(photo.star, Some(true));
        assert_eq!(photo.caption.as_deref(), Some("On the steps"));
        assert_eq!(photo.rotation, Some(90));
        assert_eq!(ini.get("IMG_0002.JPG").unwrap().star, Some(false));
        assert!(ini.get("contacts").is_none());
    }
}
