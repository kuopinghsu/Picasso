//! Asks GitHub for the latest Picasso release, at most once a day, so the
//! header can say when a newer version is out. Nothing is downloaded.

use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/kuopinghsu/picasso/releases/latest";
/// Seconds between checks.
pub const EVERY: i64 = 86_400;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Version number without the leading `v`, such as `0.10.0`.
    pub version: String,
    /// The release page.
    pub url: String,
}

/// The latest published release, or `None` when there is none yet.
pub fn latest() -> Result<Option<Release>, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .user_agent(concat!("Picasso/", env!("CARGO_PKG_VERSION"), " (update check)"))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|err| err.to_string())?;
    match response.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        status => return Err(format!("HTTP {status}")),
    }
    let body = response.body_mut().with_config().limit(1 << 20).read_to_string().map_err(|err| err.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|err| err.to_string())?;
    let tag = json["tag_name"].as_str().ok_or("no tag_name")?;
    let url = json["html_url"].as_str().unwrap_or(env!("CARGO_PKG_REPOSITORY"));
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: url.to_string(),
    }))
}

/// Whether `version` comes after the running build.
pub fn is_newer(version: &str) -> bool {
    newer_than(version, env!("CARGO_PKG_VERSION"))
}

/// Compares dotted version numbers; anything after `-` or `+` is ignored.
fn newer_than(version: &str, current: &str) -> bool {
    let parts = |text: &str| -> Vec<u64> {
        text.trim_start_matches(['v', 'V'])
            .split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|part| part.trim().parse().unwrap_or(0))
            .collect()
    };
    let (mut new, mut old) = (parts(version), parts(current));
    let len = new.len().max(old.len());
    new.resize(len, 0);
    old.resize(len, 0);
    new > old
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer_than("0.10.0", "0.9.1"));
        assert!(newer_than("v1.0", "0.9.9"));
        assert!(newer_than("0.9.2-beta", "0.9.1"));
        assert!(!newer_than("0.9.1", "0.9.1"));
        assert!(!newer_than("0.9", "0.9.0"));
        assert!(!newer_than("0.8.12", "0.9.1"));
    }
}
