//! Names for a position, looked up from OpenStreetMap's Nominatim only when
//! someone asks for one and has allowed it in Settings.
//!
//! The position goes out rounded to about 100 m, with nothing else about the
//! photos. Answers are kept on disk, so a place already looked up is named
//! again without asking the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

const ENDPOINT: &str = "https://nominatim.openstreetmap.org/reverse";

/// Decimals the position is rounded to before it leaves the computer.
const DECIMALS: i32 = 3;

/// A name in the app's language and the one used where the place is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaceNames {
    pub name: String,
    pub local_name: String,
}

/// The position as it is sent and cached: rounded to [`DECIMALS`].
pub fn rounded(lat: f64, lon: f64) -> (f64, f64) {
    let scale = 10f64.powi(DECIMALS);
    ((lat * scale).round() / scale, (lon * scale).round() / scale)
}

fn cache_file(dir: &Path, lat: f64, lon: f64, lang: &str) -> PathBuf {
    let lang = if lang.is_empty() { "local" } else { lang };
    dir.join(format!("{lat:.3}_{lon:.3}_{lang}.json"))
}

/// The name of the place at `lat`, `lon`, in `lang` and in the local
/// language, from the cache in `dir` or else from Nominatim.
pub fn reverse(dir: &Path, lat: f64, lon: f64, lang: &str) -> Result<PlaceNames, String> {
    let (lat, lon) = rounded(lat, lon);
    let agent = agent();
    let name = lookup(&agent, dir, lat, lon, lang)?;
    let local_name = lookup(&agent, dir, lat, lon, "")?;
    let name = if name.is_empty() { local_name.clone() } else { name };
    let local_name = if local_name == name { String::new() } else { local_name };
    Ok(PlaceNames { name, local_name })
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .user_agent(concat!("Picasso/", env!("CARGO_PKG_VERSION"), " (local photo library; place names)"))
        .build();
    ureq::Agent::new_with_config(config)
}

/// One name, cached by position and language. An empty `lang` asks for
/// the names used locally.
fn lookup(agent: &ureq::Agent, dir: &Path, lat: f64, lon: f64, lang: &str) -> Result<String, String> {
    let file = cache_file(dir, lat, lon, lang);
    if let Some(name) = std::fs::read(&file).ok().and_then(|data| serde_json::from_slice::<Value>(&data).ok()).map(|value| best_name(&value)) {
        return Ok(name);
    }
    let mut request = agent
        .get(ENDPOINT)
        .query("format", "jsonv2")
        .query("zoom", "16")
        .query("lat", format!("{lat:.3}"))
        .query("lon", format!("{lon:.3}"));
    if !lang.is_empty() {
        request = request.query("accept-language", lang);
    }
    // Nominatim asks for at most one request a second.
    std::thread::sleep(Duration::from_millis(1100));
    let mut response = request.call().map_err(|err| err.to_string())?;
    let data = response.body_mut().with_config().limit(1 << 20).read_to_vec().map_err(|err| err.to_string())?;
    let value: Value = serde_json::from_slice(&data).map_err(|err| err.to_string())?;
    if let Some(err) = value.get("error").and_then(Value::as_str) {
        return Err(err.to_string());
    }
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&file, &data);
    Ok(best_name(&value))
}

/// The place's own name, else the nearest named area around it.
fn best_name(value: &Value) -> String {
    if let Some(name) = value.get("name").and_then(Value::as_str).filter(|name| !name.trim().is_empty()) {
        return name.trim().to_string();
    }
    let address = value.get("address");
    for key in ["neighbourhood", "suburb", "village", "town", "city", "municipality", "county", "state", "country"] {
        if let Some(name) = address.and_then(|address| address.get(key)).and_then(Value::as_str).filter(|name| !name.trim().is_empty()) {
            return name.trim().to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_are_rounded_to_about_a_hundred_meters() {
        assert_eq!(rounded(40.416_775, -3.703_790), (40.417, -3.704));
    }

    #[test]
    fn a_place_without_its_own_name_takes_the_area_around_it() {
        let named: Value = serde_json::json!({"name": "Plaza Mayor", "address": {"city": "Madrid"}});
        assert_eq!(best_name(&named), "Plaza Mayor");
        let street: Value = serde_json::json!({"name": "", "address": {"road": "Calle Mayor", "suburb": "Sol", "city": "Madrid"}});
        assert_eq!(best_name(&street), "Sol");
        assert_eq!(best_name(&serde_json::json!({})), "");
    }

    #[test]
    fn a_cached_answer_is_used_without_the_network() {
        let dir = std::env::temp_dir().join(format!("picasso-geocode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(cache_file(&dir, 41.404, 2.175, "en"), r#"{"name":"Sagrada Família"}"#).unwrap();
        std::fs::write(cache_file(&dir, 41.404, 2.175, ""), r#"{"name":"Sagrada Família"}"#).unwrap();
        let names = reverse(&dir, 41.40363, 2.17486, "en").unwrap();
        assert_eq!(names, PlaceNames { name: "Sagrada Família".into(), local_name: String::new() });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
