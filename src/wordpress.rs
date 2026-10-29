//! Talking to a self-hosted WordPress site over its REST API: finding the
//! API from the site's address, and checking an account with an
//! Application Password (`docs/TRAVEL.md`).
//!
//! Requests go through [`Http`], so the tests answer them without a
//! network. Redirects are followed here rather than by the HTTP client, so
//! a redirect to another origin is refused before the credentials could go
//! along with it.
//!
//! Publishing to a site uses it; until then only its tests do.
#![cfg_attr(not(test), allow(dead_code))]

use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::Value;

/// Redirects followed before giving up.
const MAX_REDIRECTS: usize = 5;
/// The largest reply read; API answers here are small JSON documents.
const MAX_REPLY: u64 = 4 * 1024 * 1024;
/// The relation WordPress names its REST API by in a Link header.
const API_REL: &str = "https://api.w.org/";

/// An address split into what decides where a request goes.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteUrl {
    /// "https" or "http".
    pub scheme: String,
    /// Lowercase, without the port.
    pub host: String,
    pub port: Option<u16>,
    /// From the first "/", with any query; "/" when there is none.
    pub path: String,
}

impl SiteUrl {
    /// `text` as typed: "example.com/blog" means HTTPS.
    pub fn parse(text: &str) -> Result<SiteUrl, Failure> {
        let text = text.trim();
        let (scheme, rest) = match text.split_once("://") {
            Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
            None => ("https".to_string(), text),
        };
        if scheme != "https" && scheme != "http" {
            return Err(Failure::InvalidUrl);
        }
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(split);
        let path = path.split('#').next().unwrap_or_default();
        // A user name in the address could send it somewhere else than it reads.
        if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
            return Err(Failure::InvalidUrl);
        }
        // An IPv6 address has colons of its own, inside brackets.
        let (host, port) = if authority.starts_with('[') {
            let end = authority.find(']').ok_or(Failure::InvalidUrl)?;
            (&authority[..=end], authority[end + 1..].strip_prefix(':'))
        } else {
            match authority.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            }
        };
        let port = match port {
            Some(port) if !port.is_empty() => Some(port.parse::<u16>().map_err(|_| Failure::InvalidUrl)?),
            _ => None,
        };
        if host.is_empty() || host == "[]" || host.contains(['[', ']']) && !host.starts_with('[') {
            return Err(Failure::InvalidUrl);
        }
        let path = if path.is_empty() {
            "/".to_string()
        } else if path.starts_with('?') {
            format!("/{path}")
        } else {
            path.to_string()
        };
        Ok(SiteUrl { scheme, host: host.to_ascii_lowercase(), port, path })
    }

    /// Scheme, host and port: two addresses with the same origin may share credentials.
    pub fn origin(&self) -> String {
        let default = if self.scheme == "https" { 443 } else { 80 };
        match self.port {
            Some(port) if port != default => format!("{}://{}:{port}", self.scheme, self.host),
            _ => format!("{}://{}", self.scheme, self.host),
        }
    }

    pub fn to_url(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }

    /// `location` from a redirect or a Link header, against this address.
    pub fn join(&self, location: &str) -> Result<SiteUrl, Failure> {
        if location.contains("://") {
            SiteUrl::parse(location)
        } else if let Some(rest) = location.strip_prefix("//") {
            SiteUrl::parse(&format!("{}://{rest}", self.scheme))
        } else if location.starts_with('/') {
            SiteUrl::parse(&format!("{}{location}", self.origin()))
        } else {
            let base = self.path.split('?').next().unwrap_or("/");
            let dir = &base[..base.rfind('/').map_or(0, |i| i + 1)];
            SiteUrl::parse(&format!("{}{dir}{location}", self.origin()))
        }
    }

    /// A machine on this computer or a test name, where plain HTTP may be
    /// allowed for development.
    pub fn is_development_host(&self) -> bool {
        let host = self.host.trim_start_matches('[').trim_end_matches(']');
        matches!(host, "localhost" | "127.0.0.1" | "::1") || host.ends_with(".localhost") || host.ends_with(".test")
    }

    /// An address on a private network, typed as numbers.
    fn is_private_address(&self) -> bool {
        let host = self.host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
            return ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified();
        }
        if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
            let first = ip.segments()[0];
            return ip.is_loopback() || ip.is_unspecified() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80;
        }
        false
    }
}

/// Where requests may go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Policy {
    /// Plain HTTP and private addresses for a site on this computer, for
    /// development; never for a real site.
    pub development: bool,
}

impl Policy {
    fn check(self, url: &SiteUrl) -> Result<(), Failure> {
        if self.development && url.is_development_host() {
            return Ok(());
        }
        if url.scheme != "https" {
            return Err(Failure::NotHttps);
        }
        if url.is_private_address() || url.is_development_host() {
            return Err(Failure::PrivateAddress);
        }
        Ok(())
    }
}

/// An Application Password for one account, sent only to the site's origin.
#[derive(Clone, PartialEq)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    // The password stays out of logs and panics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("user", &self.user).finish_non_exhaustive()
    }
}

impl Credentials {
    fn header(&self) -> String {
        // WordPress shows Application Passwords in groups with spaces; either works.
        format!("Basic {}", STANDARD.encode(format!("{}:{}", self.user, self.password)))
    }
}

/// One answer from the network, whatever its status.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub status: u16,
    /// Names in lowercase.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    fn header<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.headers.iter().filter(move |(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// The `code` WordPress puts in an error reply, such as "rest_forbidden".
    fn error_code(&self) -> Option<String> {
        self.json()?.get("code")?.as_str().map(str::to_string)
    }
}

/// Why talking to the site didn't work, in terms a person can act on.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// Not a web address.
    InvalidUrl,
    /// Plain HTTP for a site that isn't on this computer.
    NotHttps,
    /// A private-network or local address outside development.
    PrivateAddress,
    /// No such host, or nothing answered.
    Unreachable,
    /// The connection's certificate or encryption failed.
    Tls,
    /// The site took too long.
    Timeout,
    /// A redirect to another origin, where the credentials must not go.
    CrossOriginRedirect(String),
    TooManyRedirects,
    /// The address answers, but not with a WordPress REST API.
    NotWordPress,
    /// The REST API is there but turned off or blocked, as by a security plugin.
    ApiBlocked,
    /// The user name or Application Password was refused.
    Unauthorized,
    /// The account may not do this.
    Forbidden,
    /// The server refused the size of what was sent.
    TooLarge,
    /// Too many requests; the site asks to wait this many seconds, when it says.
    RateLimited(Option<u64>),
    /// Any other status the site answered with.
    Status(u16),
    /// An answer that wasn't what the API sends.
    BadReply,
}

/// The network, as Picasso uses it: one request at a time, with no
/// redirects followed and any status returned as a [`Reply`].
pub trait Http {
    fn get(&self, url: &SiteUrl, auth: Option<&str>) -> Result<Reply, Failure>;
}

/// The site's REST API, found from its address.
#[derive(Clone, Debug, PartialEq)]
pub struct Api {
    /// What the person typed, after redirects within the same origin.
    pub site: SiteUrl,
    /// The API's root: ".../wp-json/" or ".../?rest_route=/".
    pub root: SiteUrl,
    /// The site's title.
    pub name: String,
    /// What the site offers, such as "wp/v2".
    pub namespaces: Vec<String>,
}

impl Api {
    /// The address of `route`, such as "/wp/v2/users/me", under the root,
    /// with `query` such as "context=edit".
    pub fn route(&self, route: &str, query: &str) -> SiteUrl {
        let route = route.trim_start_matches('/');
        let mut url = self.root.clone();
        let (path, root_query) = url.path.split_once('?').map_or((url.path.as_str(), None), |(p, q)| (p, Some(q)));
        let mut path = path.to_string();
        let mut params: Vec<String> = Vec::new();
        match root_query {
            // Without pretty permalinks the route is a query parameter.
            Some(q) if q.starts_with("rest_route=") => {
                params.push(format!("rest_route=/{route}"));
                params.extend(q.split('&').skip(1).map(str::to_string));
            }
            other => {
                if !path.ends_with('/') {
                    path.push('/');
                }
                path.push_str(route);
                params.extend(other.into_iter().flat_map(|q| q.split('&')).filter(|p| !p.is_empty()).map(str::to_string));
            }
        }
        params.extend(query.split('&').filter(|p| !p.is_empty()).map(str::to_string));
        url.path = if params.is_empty() { path } else { format!("{path}?{}", params.join("&")) };
        url
    }
}

/// The account an Application Password belongs to, and what it may do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub roles: Vec<String>,
    pub upload_files: bool,
    pub edit_posts: bool,
    pub publish_posts: bool,
}

/// A site reached through `http`, within `policy`.
pub struct Client<'a> {
    http: &'a dyn Http,
    policy: Policy,
}

impl<'a> Client<'a> {
    pub fn new(http: &'a dyn Http, policy: Policy) -> Self {
        Client { http, policy }
    }

    /// GET `url`, following redirects within its origin. `auth` goes only
    /// to that origin; any redirect away from it is refused.
    fn get(&self, url: &SiteUrl, auth: Option<&Credentials>) -> Result<(SiteUrl, Reply), Failure> {
        self.policy.check(url)?;
        let origin = url.origin();
        let header = auth.map(Credentials::header);
        let mut url = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let reply = self.http.get(&url, header.as_deref())?;
            if !matches!(reply.status, 301 | 302 | 303 | 307 | 308) {
                return Ok((url, reply));
            }
            let location = reply.header("location").next().ok_or(Failure::BadReply)?;
            let next = url.join(location)?;
            // An upgrade to HTTPS on the same host keeps the request where it was meant to go.
            let upgrade = url.scheme == "http" && next.scheme == "https" && next.host == url.host;
            if next.origin() != origin && !(upgrade && auth.is_none()) {
                return Err(Failure::CrossOriginRedirect(next.origin()));
            }
            self.policy.check(&next)?;
            url = next;
        }
        Err(Failure::TooManyRedirects)
    }

    /// The REST API of the site at `site`, as typed. Looks for the API's
    /// Link header first, then the two places WordPress serves the API
    /// from, so a site in a subdirectory or without pretty permalinks is
    /// found too.
    pub fn discover(&self, site: &str) -> Result<Api, Failure> {
        let site = SiteUrl::parse(site)?;
        let (site, page) = self.get(&site, None)?;
        let mut candidates = Vec::new();
        for link in page.header("link") {
            candidates.extend(api_links(link).into_iter().filter_map(|href| site.join(&href).ok()));
        }
        let base = {
            let path = site.path.split('?').next().unwrap_or("/");
            let dir = if path.ends_with('/') { path.to_string() } else { format!("{path}/") };
            SiteUrl { path: dir, ..site.clone() }
        };
        candidates.push(base.join("wp-json/")?);
        candidates.push(base.join("?rest_route=/")?);
        let mut blocked = false;
        for root in candidates {
            if root.origin() != site.origin() {
                continue;
            }
            let (root, reply) = match self.get(&root, None) {
                Ok(found) => found,
                Err(Failure::CrossOriginRedirect(_)) => continue,
                Err(err) => return Err(err),
            };
            if let Some(api) = api_index(&site, &root, &reply) {
                return Ok(api);
            }
            blocked |= matches!(reply.status, 401 | 403) || reply.error_code().is_some_and(|code| code.contains("rest_") && code.contains("disabled"));
        }
        Err(if blocked { Failure::ApiBlocked } else { Failure::NotWordPress })
    }

    /// The account `credentials` sign in as, and what it may do. Reads
    /// only; nothing is created on the site.
    pub fn account(&self, api: &Api, credentials: &Credentials) -> Result<Account, Failure> {
        if !api.namespaces.iter().any(|namespace| namespace == "wp/v2") {
            return Err(Failure::NotWordPress);
        }
        let url = api.route("/wp/v2/users/me", "context=edit");
        let (_, reply) = self.get(&url, Some(credentials))?;
        checked(&reply)?;
        let me = reply.json().ok_or(Failure::BadReply)?;
        let capabilities = me.get("capabilities").cloned().unwrap_or(Value::Null);
        let can = |name: &str| capabilities.get(name).and_then(Value::as_bool).unwrap_or(false);
        Ok(Account {
            id: me.get("id").and_then(Value::as_i64).ok_or(Failure::BadReply)?,
            name: me.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            roles: me
                .get("roles")
                .and_then(Value::as_array)
                .map(|roles| roles.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
            upload_files: can("upload_files"),
            edit_posts: can("edit_posts"),
            publish_posts: can("publish_posts"),
        })
    }
}

/// The API root a reply describes, when it is one.
fn api_index(site: &SiteUrl, root: &SiteUrl, reply: &Reply) -> Option<Api> {
    if reply.status != 200 {
        return None;
    }
    let index = reply.json()?;
    let namespaces: Vec<String> = index.get("namespaces")?.as_array()?.iter().filter_map(Value::as_str).map(str::to_string).collect();
    Some(Api {
        site: site.clone(),
        root: root.clone(),
        name: index.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
        namespaces,
    })
}

/// The addresses a Link header gives for the REST API.
fn api_links(header: &str) -> Vec<String> {
    header
        .split(',')
        .filter_map(|part| {
            let (target, params) = part.trim().split_once('>')?;
            let target = target.trim().strip_prefix('<')?;
            let api = params.split(';').any(|param| {
                let param = param.trim();
                param.strip_prefix("rel=").is_some_and(|rel| rel.trim_matches('"') == API_REL)
            });
            api.then(|| target.to_string())
        })
        .collect()
}

/// A reply turned into the failure its status means, when it isn't a success.
fn checked(reply: &Reply) -> Result<(), Failure> {
    match reply.status {
        200..=299 => Ok(()),
        401 => Err(Failure::Unauthorized),
        403 => Err(if reply.error_code().is_some_and(|code| code.contains("disabled")) { Failure::ApiBlocked } else { Failure::Forbidden }),
        404 => Err(Failure::NotWordPress),
        413 => Err(Failure::TooLarge),
        429 => Err(Failure::RateLimited(reply.header("retry-after").next().and_then(|value| value.trim().parse().ok()))),
        status => Err(Failure::Status(status)),
    }
}

/// [`Http`] over the network.
pub struct Network {
    agent: ureq::Agent,
}

impl Network {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(concat!("Picasso/", env!("CARGO_PKG_VERSION"), " (local photo library; WordPress publishing)"))
            .build();
        Network { agent: ureq::Agent::new_with_config(config) }
    }
}

impl Http for Network {
    fn get(&self, url: &SiteUrl, auth: Option<&str>) -> Result<Reply, Failure> {
        let mut request = self.agent.get(url.to_url()).header("Accept", "application/json");
        if let Some(auth) = auth {
            request = request.header("Authorization", auth);
        }
        let mut response = request.call().map_err(|err| network_failure(&err))?;
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| Some((name.as_str().to_ascii_lowercase(), value.to_str().ok()?.to_string())))
            .collect();
        let status = response.status().as_u16();
        let body = response.body_mut().with_config().limit(MAX_REPLY).read_to_vec().map_err(|err| network_failure(&err))?;
        Ok(Reply { status, headers, body })
    }
}

fn network_failure(err: &ureq::Error) -> Failure {
    match err {
        ureq::Error::Timeout(_) => Failure::Timeout,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => Failure::Unreachable,
        ureq::Error::BodyExceedsLimit(_) => Failure::BadReply,
        ureq::Error::Tls(_) | ureq::Error::TlsRequired => Failure::Tls,
        other => {
            let text = other.to_string().to_ascii_lowercase();
            if text.contains("tls") || text.contains("certificate") {
                Failure::Tls
            } else {
                Failure::Unreachable
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Answers by address, and remembers what was asked with which credentials.
    #[derive(Default)]
    struct Fake {
        replies: HashMap<String, Reply>,
        asked: RefCell<Vec<(String, Option<String>)>>,
    }

    impl Fake {
        fn answer(mut self, url: &str, status: u16, headers: &[(&str, &str)], body: &str) -> Self {
            let headers = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            self.replies.insert(url.to_string(), Reply { status, headers, body: body.as_bytes().to_vec() });
            self
        }
    }

    impl Http for Fake {
        fn get(&self, url: &SiteUrl, auth: Option<&str>) -> Result<Reply, Failure> {
            self.asked.borrow_mut().push((url.to_url(), auth.map(str::to_string)));
            Ok(self.replies.get(&url.to_url()).cloned().unwrap_or(Reply { status: 404, ..Reply::default() }))
        }
    }

    const INDEX: &str = r#"{"name":"Travels","namespaces":["oembed/1.0","wp/v2"]}"#;

    #[test]
    fn an_address_as_typed_is_https_with_a_path() {
        let url = SiteUrl::parse(" Example.COM/blog ").unwrap();
        assert_eq!(url.to_url(), "https://example.com/blog");
        assert_eq!(SiteUrl::parse("http://example.com:8080").unwrap().origin(), "http://example.com:8080");
        assert_eq!(SiteUrl::parse("https://example.com:443/").unwrap().origin(), "https://example.com");
        assert_eq!(SiteUrl::parse("https://[::1]:8443/x").unwrap().host, "[::1]");
        for bad in ["", "ftp://example.com", "https://user@evil.com", "https://example.com:99999", "https://"] {
            assert_eq!(SiteUrl::parse(bad), Err(Failure::InvalidUrl), "{bad}");
        }
    }

    #[test]
    fn only_https_reaches_a_public_site_and_http_only_a_local_one_in_development() {
        let strict = Policy::default();
        let dev = Policy { development: true };
        let url = |text: &str| SiteUrl::parse(text).unwrap();
        assert_eq!(strict.check(&url("https://example.com")), Ok(()));
        assert_eq!(strict.check(&url("http://example.com")), Err(Failure::NotHttps));
        assert_eq!(dev.check(&url("http://example.com")), Err(Failure::NotHttps));
        assert_eq!(strict.check(&url("https://192.168.1.4")), Err(Failure::PrivateAddress));
        assert_eq!(strict.check(&url("https://localhost")), Err(Failure::PrivateAddress));
        assert_eq!(dev.check(&url("http://localhost:8080")), Ok(()));
        assert_eq!(dev.check(&url("http://blog.test")), Ok(()));
    }

    #[test]
    fn the_api_is_found_from_the_link_header_of_a_site_in_a_subdirectory() {
        let fake = Fake::default()
            .answer("https://example.com/blog", 200, &[("link", r#"<https://example.com/blog/wp-json/>; rel="https://api.w.org/""#)], "<html>")
            .answer("https://example.com/blog/wp-json/", 200, &[], INDEX);
        let api = Client::new(&fake, Policy::default()).discover("example.com/blog").unwrap();
        assert_eq!(api.root.to_url(), "https://example.com/blog/wp-json/");
        assert_eq!(api.name, "Travels");
        assert_eq!(api.route("/wp/v2/users/me", "context=edit").to_url(), "https://example.com/blog/wp-json/wp/v2/users/me?context=edit");
    }

    #[test]
    fn a_site_without_pretty_permalinks_uses_the_rest_route_parameter() {
        let fake = Fake::default()
            .answer("https://example.com/", 200, &[], "<html>")
            .answer("https://example.com/?rest_route=/", 200, &[], INDEX);
        let api = Client::new(&fake, Policy::default()).discover("https://example.com").unwrap();
        assert_eq!(api.route("/wp/v2/users/me", "context=edit").to_url(), "https://example.com/?rest_route=/wp/v2/users/me&context=edit");
    }

    #[test]
    fn a_blocked_api_and_a_site_without_wordpress_are_told_apart() {
        let blocked = Fake::default()
            .answer("https://example.com/", 200, &[], "<html>")
            .answer("https://example.com/wp-json/", 401, &[], r#"{"code":"rest_login_required"}"#);
        assert_eq!(Client::new(&blocked, Policy::default()).discover("example.com"), Err(Failure::ApiBlocked));
        let plain = Fake::default().answer("https://example.com/", 200, &[], "<html>");
        assert_eq!(Client::new(&plain, Policy::default()).discover("example.com"), Err(Failure::NotWordPress));
    }

    #[test]
    fn credentials_never_follow_a_redirect_to_another_origin() {
        let fake = Fake::default()
            .answer("https://example.com/wp-json/wp/v2/users/me?context=edit", 302, &[("location", "https://evil.example/steal")], "")
            .answer("https://evil.example/steal", 200, &[], "{}");
        let api = Api {
            site: SiteUrl::parse("https://example.com").unwrap(),
            root: SiteUrl::parse("https://example.com/wp-json/").unwrap(),
            name: String::new(),
            namespaces: vec!["wp/v2".into()],
        };
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let result = Client::new(&fake, Policy::default()).account(&api, &credentials);
        assert_eq!(result, Err(Failure::CrossOriginRedirect("https://evil.example".into())));
        assert!(fake.asked.borrow().iter().all(|(url, _)| url.starts_with("https://example.com/")));
    }

    #[test]
    fn the_account_says_what_it_may_do_and_refusals_say_why() {
        let me = r#"{"id":3,"name":"Ana","roles":["author"],"capabilities":{"upload_files":true,"edit_posts":true,"publish_posts":false}}"#;
        let url = "https://example.com/wp-json/wp/v2/users/me?context=edit";
        let api = Api {
            site: SiteUrl::parse("https://example.com").unwrap(),
            root: SiteUrl::parse("https://example.com/wp-json/").unwrap(),
            name: String::new(),
            namespaces: vec!["wp/v2".into()],
        };
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let fake = Fake::default().answer(url, 200, &[], me);
        let account = Client::new(&fake, Policy::default()).account(&api, &credentials).unwrap();
        assert_eq!((account.id, account.upload_files, account.edit_posts, account.publish_posts), (3, true, true, false));
        assert_eq!(account.roles, ["author"]);
        let sent = fake.asked.borrow()[0].1.clone().unwrap();
        assert_eq!(sent, format!("Basic {}", STANDARD.encode("ana:abcd efgh")));

        for (status, headers, failure) in [
            (401, vec![], Failure::Unauthorized),
            (403, vec![], Failure::Forbidden),
            (429, vec![("retry-after", "30")], Failure::RateLimited(Some(30))),
            (500, vec![], Failure::Status(500)),
        ] {
            let fake = Fake::default().answer(url, status, &headers, "{}");
            assert_eq!(Client::new(&fake, Policy::default()).account(&api, &credentials), Err(failure));
        }
    }

    #[test]
    fn network_errors_become_failures_a_person_can_act_on() {
        let _ = Network::new();
        assert_eq!(network_failure(&ureq::Error::HostNotFound), Failure::Unreachable);
        assert_eq!(network_failure(&ureq::Error::TlsRequired), Failure::Tls);
        assert_eq!(network_failure(&ureq::Error::BodyExceedsLimit(MAX_REPLY)), Failure::BadReply);
    }

    #[test]
    fn the_password_stays_out_of_debug_output() {
        let credentials = Credentials { user: "ana".into(), password: "secret pass".into() };
        assert!(!format!("{credentials:?}").contains("secret"));
    }
}
