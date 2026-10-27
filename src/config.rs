//! Preferences from the Settings dialog, kept in `settings.json` in the
//! system's settings folder for Picasso rather than in the library, so they
//! apply to whichever library is open.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::db::Database;
use crate::types::FileType;

const RECENT_SEARCHES: usize = 10;

pub struct Config {
    path: PathBuf,
    values: Map<String, Value>,
}

impl Config {
    /// Reads `settings.json` in `dir`. The first time, settings an older
    /// build kept in the library `db` are copied over.
    pub fn load(dir: &Path, db: &Database) -> Config {
        let path = dir.join("settings.json");
        let stored = read(&path);
        let mut config = Config {
            path,
            values: Map::new(),
        };
        match stored {
            Some(values) => config.values = values,
            _ => {
                config.copy_from_library(db);
                config.save();
            }
        }
        config
    }

    fn copy_from_library(&mut self, db: &Database) {
        for key in ["language", "theme"] {
            if let Some(value) = db.setting(key) {
                self.values.insert(key.into(), Value::from(value));
            }
        }
        for key in ["thumb_size", "slideshow_interval"] {
            if let Some(value) = db.setting(key).and_then(|value| value.parse::<f64>().ok()) {
                self.values.insert(key.into(), Value::from(value));
            }
        }
        if let Some(value) = db.setting("confirm_trash") {
            self.values.insert("confirm_trash".into(), Value::from(value != "0"));
        }
    }

    /// Where `settings.json` is kept.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.values.get(key)?.as_str()
    }

    pub fn number(&self, key: &str) -> Option<f32> {
        self.values.get(key)?.as_f64().map(|value| value as f32)
    }

    /// [`Config::number`] at full precision.
    pub fn float(&self, key: &str) -> Option<f64> {
        self.values.get(key)?.as_f64()
    }

    pub fn flag(&self, key: &str) -> Option<bool> {
        self.values.get(key)?.as_bool()
    }

    /// Types the scanner indexes; every type when the setting is missing.
    pub fn file_types(&self) -> Vec<FileType> {
        match self.values.get("file_types").and_then(Value::as_array) {
            Some(codes) => codes.iter().filter_map(|code| FileType::from_code(code.as_str()?)).collect(),
            None => FileType::ALL.to_vec(),
        }
    }

    pub fn set_file_types(&mut self, types: &[FileType]) {
        let codes = types.iter().map(|kind| Value::from(kind.code())).collect();
        self.set("file_types", Value::Array(codes));
    }

    /// Recent searches, newest first.
    pub fn searches(&self) -> Vec<String> {
        let Some(searches) = self.values.get("recent_searches").and_then(Value::as_array) else { return Vec::new() };
        searches.iter().filter_map(Value::as_str).map(str::to_string).collect()
    }

    /// Puts `search` first in the recent searches, once, dropping the oldest
    /// past [`RECENT_SEARCHES`].
    pub fn remember_search(&mut self, search: &str) {
        let search = search.trim();
        if search.is_empty() {
            return;
        }
        if let Some(stored) = read(&self.path) {
            self.values = stored;
        }
        let mut searches = self.searches();
        let lower = search.to_lowercase();
        searches.retain(|earlier| earlier.to_lowercase() != lower);
        searches.insert(0, search.to_string());
        searches.truncate(RECENT_SEARCHES);
        self.set("recent_searches", Value::Array(searches.into_iter().map(Value::from).collect()));
    }

    pub fn forget_searches(&mut self) {
        self.set("recent_searches", Value::Array(Vec::new()));
    }

    /// Stores `value` and writes the file. Other settings are taken from the
    /// file first, so another Picasso window's changes since this one loaded
    /// are kept.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        let value = value.into();
        if self.values.get(key) != Some(&value) {
            if let Some(stored) = read(&self.path) {
                self.values = stored;
            }
            self.values.insert(key.into(), value);
            self.save();
        }
    }

    /// Writes to a temporary file first, so a crash never leaves half a file.
    fn save(&self) {
        let Ok(text) = serde_json::to_string_pretty(&self.values) else { return };
        let temporary = self.path.with_extension("json.tmp");
        let written = self
            .path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&temporary, text))
            .and_then(|()| std::fs::rename(&temporary, &self.path));
        if let Err(err) = written {
            eprintln!("picasso settings: {err}");
        }
    }
}

/// A text setting from `settings.json` in `dir`, read without the library.
pub fn stored_text(dir: &Path, key: &str) -> Option<String> {
    read(&dir.join("settings.json"))?.get(key)?.as_str().map(str::to_string)
}

fn read(path: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(values) => Some(values),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_library_settings_once_then_keeps_its_own() {
        let dir = std::env::temp_dir().join(format!("picasso-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("library.db")).unwrap();
        db.set_setting("language", "zh-Hant");
        db.set_setting("thumb_size", "180");
        db.set_setting("confirm_trash", "0");

        let mut config = Config::load(&dir, &db);
        assert_eq!(config.text("language"), Some("zh-Hant"));
        assert_eq!(config.number("thumb_size"), Some(180.0));
        assert_eq!(config.flag("confirm_trash"), Some(false));
        assert_eq!(config.file_types(), FileType::ALL.to_vec());

        config.set("thumb_size", 120.0);
        config.set_file_types(&[FileType::Jpeg, FileType::Raw]);
        db.set_setting("language", "ja");
        let config = Config::load(&dir, &db);
        assert_eq!(config.text("language"), Some("zh-Hant"));
        assert_eq!(config.number("thumb_size"), Some(120.0));
        assert_eq!(config.file_types(), vec![FileType::Jpeg, FileType::Raw]);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_windows_keep_each_others_settings() {
        let dir = std::env::temp_dir().join(format!("picasso-config-two-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("library.db")).unwrap();
        let mut first = Config::load(&dir, &db);
        let mut second = Config::load(&dir, &db);

        first.set("carto_key", "abc123");
        second.set("thumb_size", 150.0);
        let config = Config::load(&dir, &db);
        assert_eq!(config.text("carto_key"), Some("abc123"));
        assert_eq!(config.number("thumb_size"), Some(150.0));
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_searches_are_kept_newest_first_once_each() {
        let dir = std::env::temp_dir().join(format!("picasso-config-searches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("library.db")).unwrap();
        let mut config = Config::load(&dir, &db);
        assert!(config.searches().is_empty());

        config.remember_search("  beach ");
        config.remember_search("");
        config.remember_search("type:raw");
        config.remember_search("Beach");
        assert_eq!(config.searches(), ["Beach", "type:raw"]);
        for n in 0..20 {
            config.remember_search(&format!("search {n}"));
        }
        let mut config = Config::load(&dir, &db);
        assert_eq!(config.searches().len(), RECENT_SEARCHES);
        assert_eq!(config.searches()[0], "search 19");

        config.forget_searches();
        assert!(Config::load(&dir, &db).searches().is_empty());
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
