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
    /// The site didn't look at the Application Password at all: WordPress
    /// accepts them only over HTTPS or on a site set up as local, and some
    /// servers drop the header that carries them.
    CredentialsIgnored,
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

impl Failure {
    /// Whether a write that failed this way may still have been done: the
    /// request may have reached the site before the answer was lost.
    pub fn outcome_unknown(&self) -> bool {
        matches!(self, Failure::Timeout | Failure::Unreachable | Failure::BadReply) || matches!(self, Failure::Status(status) if *status >= 500)
    }
}

/// One request to the site.
#[derive(Clone, Debug)]
pub struct Request<'b> {
    /// "GET" or "POST": WordPress takes changes as POST.
    pub method: &'static str,
    pub url: SiteUrl,
    /// The Authorization header, sent only to the origin it was made for.
    pub auth: Option<String>,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<&'b [u8]>,
}

/// The network, as Picasso uses it: one request at a time, with no
/// redirects followed and any status returned as a [`Reply`].
pub trait Http {
    fn send(&self, request: &Request) -> Result<Reply, Failure>;
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
    /// How long it has waited before trying a request again, in all.
    waited: std::cell::Cell<Duration>,
    /// Set when the person cancels; a wait before trying again ends at once.
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
}

/// Tries of one request, the first included.
const MAX_TRIES: u32 = 3;
/// The longest wait before trying again; a site asking for longer is left
/// to the person to try later.
const MAX_WAIT: Duration = Duration::from_secs(30);

impl<'a> Client<'a> {
    pub fn new(http: &'a dyn Http, policy: Policy) -> Self {
        Client { http, policy, waited: std::cell::Cell::new(Duration::ZERO), cancel: None }
    }

    /// Lets `cancel` end a wait before a request is tried again.
    pub fn cancelled_by(self, cancel: &'a std::sync::atomic::AtomicBool) -> Self {
        Client { cancel: Some(cancel), ..self }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Sends `request`, trying again after a pause when that is safe: a
    /// read after a timeout, a lost connection or a server error, and any
    /// request the site turned away for coming too often, which it didn't
    /// do. A write whose answer was lost is never sent again here; the
    /// caller looks for what it made instead.
    fn send(&self, request: &Request) -> Result<Reply, Failure> {
        let read = request.method == "GET";
        let mut tries = 1;
        loop {
            let result = self.http.send(request);
            let asked = |reply: &Reply| reply.header("retry-after").next().and_then(|value| value.trim().parse().ok()).map(Duration::from_secs);
            let wait = match &result {
                Ok(reply) if reply.status == 429 => Some(asked(reply).unwrap_or_else(|| backoff(tries))),
                Ok(reply) if read && matches!(reply.status, 500 | 502 | 503 | 504) => Some(asked(reply).unwrap_or_else(|| backoff(tries))),
                Err(Failure::Timeout | Failure::Unreachable) if read => Some(backoff(tries)),
                _ => None,
            };
            match wait {
                Some(wait) if tries < MAX_TRIES && wait <= MAX_WAIT && !self.is_cancelled() => {
                    self.waited.set(self.waited.get() + wait);
                    // In short steps, so a cancel isn't kept waiting.
                    let until = std::time::Instant::now() + wait;
                    while !cfg!(test) && !self.is_cancelled() && std::time::Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(100).min(until - std::time::Instant::now()));
                    }
                    if self.is_cancelled() {
                        return result;
                    }
                    tries += 1;
                }
                _ => return result,
            }
        }
    }

    /// GET `url`, following redirects within its origin. `auth` goes only
    /// to that origin; any redirect away from it is refused.
    fn get(&self, url: &SiteUrl, auth: Option<&Credentials>) -> Result<(SiteUrl, Reply), Failure> {
        self.policy.check(url)?;
        let origin = url.origin();
        let header = auth.map(Credentials::header);
        let mut url = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let request = Request { method: "GET", url: url.clone(), auth: header.clone(), headers: Vec::new(), body: None };
            let reply = self.send(&request)?;
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

impl Client<'_> {
    /// POST `body` to `url` with `credentials`. A change is never sent on
    /// through a redirect: one is refused, so nothing is sent twice or elsewhere.
    fn post(&self, url: &SiteUrl, credentials: &Credentials, headers: Vec<(&'static str, String)>, body: &[u8]) -> Result<Reply, Failure> {
        self.policy.check(url)?;
        let request = Request { method: "POST", url: url.clone(), auth: Some(credentials.header()), headers, body: Some(body) };
        let reply = self.send(&request)?;
        if (300..400).contains(&reply.status) {
            return Err(Failure::Status(reply.status));
        }
        Ok(reply)
    }

    /// Uploads one JPEG to the site's media library as `name`.
    pub fn upload_media(&self, api: &Api, credentials: &Credentials, bytes: &[u8], name: &str) -> Result<Media, Failure> {
        let url = api.route("/wp/v2/media", "");
        // The name is ours, made of letters, digits, and dashes, so it needs no quoting rules.
        let headers = vec![("Content-Type", "image/jpeg".to_string()), ("Content-Disposition", format!("attachment; filename=\"{name}\""))];
        let reply = self.post(&url, credentials, headers, bytes)?;
        checked(&reply)?;
        media_of(&reply.json().ok_or(Failure::BadReply)?)
    }

    /// Sets the upload `id`'s description in the site's media library, which
    /// WordPress offers again when the photo is put in another post.
    pub fn set_media_alt(&self, api: &Api, credentials: &Credentials, id: i64, alt: &str) -> Result<(), Failure> {
        let url = api.route(&format!("/wp/v2/media/{id}"), "");
        let bytes = serde_json::to_vec(&serde_json::json!({ "alt_text": alt })).map_err(|_| Failure::BadReply)?;
        checked(&self.post(&url, credentials, vec![("Content-Type", "application/json".to_string())], &bytes)?)?;
        Ok(())
    }

    /// The uploads in the site's media library named `name` (without its
    /// extension, as WordPress titles an upload), oldest first: how an upload
    /// whose answer was lost is found.
    pub fn find_media(&self, api: &Api, credentials: &Credentials, name: &str) -> Result<Vec<Media>, Failure> {
        let query = format!("search={}&per_page=20&orderby=id&order=asc&context=edit&_fields=id,source_url,title,mime_type,media_details", query_value(name));
        let (_, reply) = self.get(&api.route("/wp/v2/media", &query), Some(credentials))?;
        checked(&reply)?;
        let list = reply.json().and_then(|json| json.as_array().cloned()).ok_or(Failure::BadReply)?;
        list.iter().filter(|item| raw_title(item) == name).map(media_of).collect()
    }

    /// The ids of the drafts and published posts titled exactly `title`
    /// changed on the site since `since` (Unix seconds), oldest first: how a
    /// post whose answer was lost is found.
    pub fn find_drafts(&self, api: &Api, credentials: &Credentials, title: &str, since: i64) -> Result<Vec<i64>, Failure> {
        let after = chrono::DateTime::from_timestamp(since, 0).ok_or(Failure::BadReply)?.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let query = format!(
            "search={}&status=draft,publish&modified_after={}&per_page=20&orderby=id&order=asc&context=edit&_fields=id,title",
            query_value(title),
            query_value(&after)
        );
        let (_, reply) = self.get(&api.route("/wp/v2/posts", &query), Some(credentials))?;
        checked(&reply)?;
        let list = reply.json().and_then(|json| json.as_array().cloned()).ok_or(Failure::BadReply)?;
        Ok(list.iter().filter(|post| raw_title(post) == title).filter_map(|post| post.get("id").and_then(Value::as_i64)).collect())
    }

    /// Whether the upload `id` is still in the site's media library.
    pub fn media_exists(&self, api: &Api, credentials: &Credentials, id: i64) -> Result<bool, Failure> {
        let url = api.route(&format!("/wp/v2/media/{id}"), "context=edit");
        let (_, reply) = self.get(&url, Some(credentials))?;
        if reply.status == 404 || reply.status == 410 {
            return Ok(false);
        }
        checked(&reply)?;
        Ok(true)
    }

    /// Post `id` as the site has it now, or `None` when the site has no such post.
    pub fn post_state(&self, api: &Api, credentials: &Credentials, id: i64) -> Result<Option<PostState>, Failure> {
        let url = api.route(&format!("/wp/v2/posts/{id}"), "context=edit");
        let (_, reply) = self.get(&url, Some(credentials))?;
        if reply.status == 404 || reply.status == 410 {
            return Ok(None);
        }
        checked(&reply)?;
        let post = reply.json().ok_or(Failure::BadReply)?;
        let status = post.get("status").and_then(Value::as_str).unwrap_or_default().to_string();
        Ok(Some(PostState { status, fingerprint: fingerprint(&post) }))
    }

    /// Every term of `taxonomy` on the site, by name. Reads only; terms are
    /// never made here. Read a page at a time up to [`MAX_TERM_PAGES`].
    pub fn terms(&self, api: &Api, credentials: &Credentials, taxonomy: Taxonomy) -> Result<Vec<Term>, Failure> {
        let mut terms = Vec::new();
        for page in 1..=MAX_TERM_PAGES {
            let query = format!("per_page={TERMS_PER_PAGE}&page={page}&orderby=name&order=asc&_fields=id,name,parent");
            let (_, reply) = self.get(&api.route(taxonomy.route(), &query), Some(credentials))?;
            // A page past the last is an error on some versions, an empty list on others.
            if page > 1 && reply.error_code().as_deref() == Some("rest_post_invalid_page_number") {
                break;
            }
            checked(&reply)?;
            let list = reply.json().and_then(|json| json.as_array().cloned()).ok_or(Failure::BadReply)?;
            let count = list.len();
            for term in list {
                let id = term.get("id").and_then(Value::as_i64).ok_or(Failure::BadReply)?;
                let name = term.get("name").and_then(Value::as_str).unwrap_or_default();
                terms.push(Term { id, name: decode_entities(name), parent: term.get("parent").and_then(Value::as_i64).unwrap_or(0) });
            }
            let pages = reply.header("x-wp-totalpages").next().and_then(|n| n.trim().parse::<u32>().ok());
            if count < TERMS_PER_PAGE as usize || pages.is_some_and(|pages| page >= pages) {
                break;
            }
        }
        Ok(terms)
    }

    /// Makes the term `name` in `taxonomy` on the site, at the top level, or
    /// returns the one already there by that name, so a retry after a lost
    /// answer never makes a second. Needs the account to be allowed to make
    /// terms; a refusal comes back as [`Failure::Forbidden`].
    pub fn create_term(&self, api: &Api, credentials: &Credentials, taxonomy: Taxonomy, name: &str) -> Result<Term, Failure> {
        let url = api.route(taxonomy.route(), "");
        let bytes = serde_json::to_vec(&serde_json::json!({ "name": name })).map_err(|_| Failure::BadReply)?;
        let reply = self.post(&url, credentials, vec![("Content-Type", "application/json".to_string())], &bytes)?;
        if reply.error_code().as_deref() == Some("term_exists") {
            let id = reply.json().and_then(|json| json.get("data")?.get("term_id")?.as_i64()).ok_or(Failure::BadReply)?;
            return Ok(Term { id, name: name.to_string(), parent: 0 });
        }
        checked(&reply)?;
        let term = reply.json().ok_or(Failure::BadReply)?;
        let id = term.get("id").and_then(Value::as_i64).ok_or(Failure::BadReply)?;
        let made = term.get("name").and_then(Value::as_str).map_or_else(|| name.to_string(), decode_entities);
        Ok(Term { id, name: made, parent: term.get("parent").and_then(Value::as_i64).unwrap_or(0) })
    }

    /// Creates a draft with `title`, `content`, and `featured` media, or,
    /// with `id`, changes those of that post and leaves its status as it is.
    /// `terms` sets its categories and tags, each only when given, so the
    /// post keeps the ones it has on the site otherwise. Returns the post as
    /// the site keeps it.
    pub fn save_draft(&self, api: &Api, credentials: &Credentials, id: Option<i64>, post: &DraftPost) -> Result<PostSaved, Failure> {
        let DraftPost { title, excerpt, content, featured, categories, tags, publish, schedule } = post;
        let mut body = serde_json::json!({ "title": title, "excerpt": excerpt, "content": content, "featured_media": featured.unwrap_or(0) });
        if let Some(categories) = categories {
            body["categories"] = Value::from(categories.clone());
        }
        if let Some(tags) = tags {
            body["tags"] = Value::from(tags.clone());
        }
        let route = match id {
            Some(id) => format!("/wp/v2/posts/{id}"),
            None => {
                body["status"] = Value::from("draft");
                "/wp/v2/posts".to_string()
            }
        };
        match (publish, schedule) {
            // The site's own time: WordPress turns it into UTC by its time
            // zone, summer time included.
            (true, Some(at)) => {
                body["status"] = Value::from("future");
                body["date"] = Value::from(at.format("%Y-%m-%dT%H:%M:00").to_string());
            }
            (true, None) => body["status"] = Value::from("publish"),
            _ => {}
        }
        let url = api.route(&route, "");
        let bytes = serde_json::to_vec(&body).map_err(|_| Failure::BadReply)?;
        let reply = self.post(&url, credentials, vec![("Content-Type", "application/json".to_string())], &bytes)?;
        checked(&reply)?;
        let post = reply.json().ok_or(Failure::BadReply)?;
        let id = post.get("id").and_then(Value::as_i64).ok_or(Failure::BadReply)?;
        let status = post.get("status").and_then(Value::as_str).unwrap_or_default().to_string();
        Ok(PostSaved { id, fingerprint: fingerprint(&post), status })
    }

    /// The time now on the site's clock, in its time zone, read from the
    /// API's index and the time its answer was sent.
    pub fn site_now(&self, api: &Api) -> Result<chrono::NaiveDateTime, Failure> {
        let (_, reply) = self.get(&api.root, None)?;
        checked(&reply)?;
        let index = reply.json().ok_or(Failure::BadReply)?;
        // Hours, given as a number or as text, and not always whole.
        let hours = match index.get("gmt_offset") {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(text)) => text.parse().ok(),
            _ => None,
        }
        .ok_or(Failure::BadReply)?;
        let sent = reply.header("date").next().and_then(|date| chrono::DateTime::parse_from_rfc2822(date).ok()).map_or_else(chrono::Utc::now, |date| date.with_timezone(&chrono::Utc));
        Ok(sent.naive_utc() + chrono::Duration::minutes((hours * 60.0).round() as i64))
    }
}

/// A post as the site has it.
#[derive(Clone, Debug, PartialEq)]
pub struct PostState {
    /// Such as "draft" or "publish".
    pub status: String,
    /// See [`fingerprint`].
    pub fingerprint: String,
}

/// A post the site confirmed saving.
#[derive(Clone, Debug, PartialEq)]
pub struct PostSaved {
    pub id: i64,
    /// See [`fingerprint`].
    pub fingerprint: String,
    /// As the site gave it, such as "draft" or "future".
    pub status: String,
}

/// What a post is saved with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DraftPost {
    pub title: String,
    /// Sent always, so a subtitle taken off takes the excerpt off too.
    pub excerpt: String,
    pub content: String,
    pub featured: Option<i64>,
    /// Term ids; `None` leaves the post's own on the site as they are.
    pub categories: Option<Vec<i64>>,
    pub tags: Option<Vec<i64>>,
    /// Make it public; otherwise a new post is a draft and an earlier one
    /// keeps its status.
    pub publish: bool,
    /// With `publish`, when it goes public instead, in the site's own time.
    pub schedule: Option<chrono::NaiveDateTime>,
}

/// The two kinds of term a post is filed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Taxonomy {
    Categories,
    Tags,
}

impl Taxonomy {
    fn route(self) -> &'static str {
        match self {
            Taxonomy::Categories => "/wp/v2/categories",
            Taxonomy::Tags => "/wp/v2/tags",
        }
    }
}

/// A category or tag on the site.
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub id: i64,
    pub name: String,
    /// The parent category's id, 0 for none and for tags.
    pub parent: i64,
}

/// Terms asked for at once, the most WordPress gives in one page.
const TERMS_PER_PAGE: u32 = 100;
/// Pages of terms read at most, so a site with very many tags can't keep
/// the dialog waiting: 5000 of each.
const MAX_TERM_PAGES: u32 = 50;

/// An upload in the site's media library.
#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub id: i64,
    /// The address of the file as uploaded.
    pub url: String,
    /// What the site made of it, as compact JSON: its width, height and
    /// type, and the address of each smaller size it made, by name. Empty
    /// when the site didn't say.
    pub details: String,
}

fn media_of(json: &Value) -> Result<Media, Failure> {
    let details = json.get("media_details").map(|info| {
        let sizes: serde_json::Map<String, Value> = info
            .get("sizes")
            .and_then(Value::as_object)
            .map(|sizes| sizes.iter().filter_map(|(name, size)| Some((name.clone(), size.get("source_url")?.clone()))).collect())
            .unwrap_or_default();
        serde_json::json!({ "width": info.get("width"), "height": info.get("height"), "mime": json.get("mime_type"), "sizes": sizes }).to_string()
    });
    Ok(Media {
        details: details.unwrap_or_default(),
        id: json.get("id").and_then(Value::as_i64).ok_or(Failure::BadReply)?,
        url: json.get("source_url").and_then(Value::as_str).ok_or(Failure::BadReply)?.to_string(),
    })
}

/// The address of post `id` in the site's editor, which needs the person
/// signed in, as a draft's own address may not show it.
/// A digest of what Picasso writes in a post, as the site keeps it: its
/// title, excerpt, content and featured photo, after [`FINGERPRINT`]. Kept after a save and compared before
/// the next, it tells whether the post was edited on the site in between;
/// its status, terms and the rest are left out, as Picasso leaves them alone.
fn fingerprint(post: &Value) -> String {
    let raw = |field: &str| post.get(field).and_then(|value| value.get("raw").or(Some(value))).and_then(Value::as_str).unwrap_or_default().to_string();
    let featured = post.get("featured_media").and_then(Value::as_i64).unwrap_or(0);
    let managed = serde_json::json!([raw("title"), raw("excerpt"), raw("content"), featured]);
    format!("{FINGERPRINT}{}", crate::upload::hash(managed.to_string().as_bytes()))
}

/// The pause before try `tries + 1`: a second, then two, each with up to a
/// quarter more so many waiting clients don't all come back at once.
fn backoff(tries: u32) -> Duration {
    let base = Duration::from_secs(1 << (tries - 1).min(4));
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |now| now.subsec_nanos());
    base + base.mul_f64(f64::from(nanos % 1000) / 4000.0)
}

/// What starts a digest made the way [`fingerprint`] makes it now; one
/// made otherwise, by an earlier Picasso, can't be compared.
pub const FINGERPRINT: &str = "2:";

/// A post's or upload's title as typed, in the edit context.
fn raw_title(item: &Value) -> &str {
    item.get("title").and_then(|title| title.get("raw")).and_then(Value::as_str).unwrap_or_default()
}

/// `text` as one value of a query string.
fn query_value(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn edit_link(api: &Api, id: i64) -> String {
    let base = api.site.path.split('?').next().unwrap_or("/");
    let dir = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
    format!("{}{dir}wp-admin/post.php?post={id}&action=edit", api.site.origin())
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
        name: decode_entities(index.get("name").and_then(Value::as_str).unwrap_or_default()),
        namespaces,
    })
}

/// `text` with the HTML entities WordPress puts in titles, such as
/// "&#039;" and "&amp;", turned back into characters.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
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
        // WordPress and PHP refuse a large upload with their own words, often as 400 or 500.
        400..=599 if too_large(reply) => Err(Failure::TooLarge),
        // Signed out despite the credentials: WordPress never read them.
        401 if reply.error_code().as_deref() == Some("rest_not_logged_in") => Err(Failure::CredentialsIgnored),
        401 => Err(Failure::Unauthorized),
        403 => Err(if reply.error_code().is_some_and(|code| code.contains("disabled")) { Failure::ApiBlocked } else { Failure::Forbidden }),
        404 => Err(Failure::NotWordPress),
        413 => Err(Failure::TooLarge),
        429 => Err(Failure::RateLimited(reply.header("retry-after").next().and_then(|value| value.trim().parse().ok()))),
        status => Err(Failure::Status(status)),
    }
}

/// Whether the site's error says an upload was over its size limit.
fn too_large(reply: &Reply) -> bool {
    let Some(json) = reply.json() else { return false };
    let code = json.get("code").and_then(Value::as_str).unwrap_or_default();
    let message = json.get("message").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase();
    matches!(code, "rest_upload_file_too_big" | "rest_upload_limited_space") || message.contains("upload_max_filesize") || message.contains("post_max_size")
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
    fn send(&self, request: &Request) -> Result<Reply, Failure> {
        let url = request.url.to_url();
        let mut response = match (request.method, request.body) {
            ("POST", body) => {
                let mut call = self.agent.post(&url).header("Accept", "application/json");
                if let Some(auth) = &request.auth {
                    call = call.header("Authorization", auth);
                }
                for (name, value) in &request.headers {
                    call = call.header(*name, value);
                }
                call.send(body.unwrap_or_default())
            }
            _ => {
                let mut call = self.agent.get(&url).header("Accept", "application/json");
                if let Some(auth) = &request.auth {
                    call = call.header("Authorization", auth);
                }
                for (name, value) in &request.headers {
                    call = call.header(*name, value);
                }
                call.call()
            }
        }
        .map_err(|err| network_failure(&err))?;
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
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Answers by method and address, and remembers what was asked with
    /// which credentials and body.
    #[derive(Default)]
    pub(crate) struct Fake {
        replies: HashMap<String, Reply>,
        pub(crate) asked: RefCell<Vec<(String, Option<String>)>>,
        pub(crate) bodies: RefCell<Vec<(String, Vec<u8>)>>,
    }

    impl Fake {
        pub(crate) fn answer(self, url: &str, status: u16, headers: &[(&str, &str)], body: &str) -> Self {
            self.answer_to("GET", url, status, headers, body)
        }

        pub(crate) fn answer_to(mut self, method: &str, url: &str, status: u16, headers: &[(&str, &str)], body: &str) -> Self {
            let headers = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            self.replies.insert(format!("{method} {url}"), Reply { status, headers, body: body.as_bytes().to_vec() });
            self
        }
    }

    impl Http for Fake {
        fn send(&self, request: &Request) -> Result<Reply, Failure> {
            let key = format!("{} {}", request.method, request.url.to_url());
            self.asked.borrow_mut().push((request.url.to_url(), request.auth.clone()));
            if let Some(body) = request.body {
                self.bodies.borrow_mut().push((key.clone(), body.to_vec()));
            }
            // A reply kept under an address ending in "*" answers any address starting so.
            let found = self.replies.get(&key).or_else(|| self.replies.iter().find(|(known, _)| known.strip_suffix('*').is_some_and(|start| key.starts_with(start))).map(|(_, reply)| reply));
            Ok(found.cloned().unwrap_or(Reply { status: 404, ..Reply::default() }))
        }
    }

    fn draft(title: &str, content: &str, featured: Option<i64>) -> DraftPost {
        DraftPost { title: title.into(), content: content.into(), featured, ..DraftPost::default() }
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
            (401, vec![("content-type", "application/json")], Failure::Unauthorized),
            (403, vec![], Failure::Forbidden),
            (429, vec![("retry-after", "30")], Failure::RateLimited(Some(30))),
            (500, vec![], Failure::Status(500)),
        ] {
            let fake = Fake::default().answer(url, status, &headers, "{}");
            assert_eq!(Client::new(&fake, Policy::default()).account(&api, &credentials), Err(failure));
        }
    }

    #[test]
    fn an_upload_over_the_sites_limit_is_named_so_and_not_sent_again() {
        let api = local_api();
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let media = "https://example.com/blog/wp-json/wp/v2/media";
        for (status, body) in [
            (500, r#"{"code":"rest_upload_unknown_error","message":"The uploaded file exceeds the upload_max_filesize directive in php.ini."}"#),
            (400, r#"{"code":"rest_upload_file_too_big","message":"This file is too big."}"#),
            (413, ""),
        ] {
            let fake = Fake::default().answer_to("POST", media, status, &[], body);
            assert_eq!(Client::new(&fake, Policy::default()).upload_media(&api, &credentials, b"jpeg", "a.jpg"), Err(Failure::TooLarge), "{status}");
            assert_eq!(fake.bodies.borrow().len(), 1);
        }
        assert!(!Failure::TooLarge.outcome_unknown(), "refused, so known not to be done");
    }

    #[test]
    fn network_errors_become_failures_a_person_can_act_on() {
        let _ = Network::new();
        assert_eq!(network_failure(&ureq::Error::HostNotFound), Failure::Unreachable);
        assert_eq!(network_failure(&ureq::Error::TlsRequired), Failure::Tls);
        assert_eq!(network_failure(&ureq::Error::BodyExceedsLimit(MAX_REPLY)), Failure::BadReply);
    }

    #[test]
    fn a_site_that_ignores_the_password_is_told_apart_from_a_wrong_one() {
        let url = "http://localhost:10003/wp-json/wp/v2/users/me?context=edit";
        let api = Api {
            site: SiteUrl::parse("http://localhost:10003").unwrap(),
            root: SiteUrl::parse("http://localhost:10003/wp-json/").unwrap(),
            name: String::new(),
            namespaces: vec!["wp/v2".into()],
        };
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let dev = Policy { development: true };
        let ignored = Fake::default().answer(url, 401, &[], r#"{"code":"rest_not_logged_in"}"#);
        assert_eq!(Client::new(&ignored, dev).account(&api, &credentials), Err(Failure::CredentialsIgnored));
        let wrong = Fake::default().answer(url, 401, &[], r#"{"code":"incorrect_password"}"#);
        assert_eq!(Client::new(&wrong, dev).account(&api, &credentials), Err(Failure::Unauthorized));
    }

    #[test]
    fn reads_and_turned_away_requests_are_tried_again_and_lost_writes_never() {
        let api = local_api();
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let busy = Fake::default().answer("https://example.com/blog/wp-json/wp/v2/posts/7?context=edit", 503, &[], "");
        let client = Client::new(&busy, Policy::default());
        assert_eq!(client.post_state(&api, &credentials, 7), Err(Failure::Status(503)));
        assert_eq!(busy.asked.borrow().len(), MAX_TRIES as usize);
        let waited = client.waited.get();
        assert!(waited >= Duration::from_secs(3) && waited <= Duration::from_millis(3750), "a second, then two, each a little more: {waited:?}");

        let posts = "https://example.com/blog/wp-json/wp/v2/posts";
        let lost = Fake::default().answer_to("POST", posts, 504, &[], "");
        assert!(Client::new(&lost, Policy::default()).save_draft(&api, &credentials, None, &draft("t", "c", None)).is_err());
        assert_eq!(lost.bodies.borrow().len(), 1, "a write that may have been done is sent once");

        let limited = Fake::default().answer_to("POST", posts, 429, &[("retry-after", "5")], "");
        let client = Client::new(&limited, Policy::default());
        assert_eq!(client.save_draft(&api, &credentials, None, &draft("t", "c", None)), Err(Failure::RateLimited(Some(5))));
        assert_eq!((limited.bodies.borrow().len(), client.waited.get()), (3, Duration::from_secs(10)), "turned away, so not done: sent again when asked");

        let cancel = std::sync::atomic::AtomicBool::new(true);
        let cancelled = Fake::default().answer("https://example.com/blog/wp-json/wp/v2/posts/7?context=edit", 503, &[], "");
        assert!(Client::new(&cancelled, Policy::default()).cancelled_by(&cancel).post_state(&api, &credentials, 7).is_err());
        assert_eq!(cancelled.asked.borrow().len(), 1, "cancelled: not tried again");

        let later = Fake::default().answer_to("POST", posts, 429, &[("retry-after", "600")], "");
        let client = Client::new(&later, Policy::default());
        assert!(client.save_draft(&api, &credentials, None, &draft("t", "c", None)).is_err());
        assert_eq!((later.bodies.borrow().len(), client.waited.get()), (1, Duration::ZERO), "too long a wait is left to the person");
    }

    #[test]
    fn a_term_is_made_once_and_one_already_there_is_used() {
        let api = local_api();
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let tags = "https://example.com/blog/wp-json/wp/v2/tags";
        let made = Fake::default().answer_to("POST", tags, 201, &[], r#"{"id":12,"name":"Tea &amp; Tapas","parent":0}"#);
        assert_eq!(Client::new(&made, Policy::default()).create_term(&api, &credentials, Taxonomy::Tags, "Tea & Tapas").unwrap(), Term { id: 12, name: "Tea & Tapas".into(), parent: 0 });
        let sent: Value = serde_json::from_slice(&made.bodies.borrow()[0].1).unwrap();
        assert_eq!(sent, serde_json::json!({ "name": "Tea & Tapas" }));

        let there = Fake::default().answer_to("POST", tags, 400, &[], r#"{"code":"term_exists","data":{"status":400,"term_id":8}}"#);
        assert_eq!(Client::new(&there, Policy::default()).create_term(&api, &credentials, Taxonomy::Tags, "Granada").unwrap().id, 8, "already there: used, not made again");

        let categories = "https://example.com/blog/wp-json/wp/v2/categories";
        let refused = Fake::default().answer_to("POST", categories, 403, &[], r#"{"code":"rest_cannot_create"}"#);
        assert_eq!(Client::new(&refused, Policy::default()).create_term(&api, &credentials, Taxonomy::Categories, "Spain"), Err(Failure::Forbidden));
    }

    #[test]
    fn a_lost_upload_or_draft_is_found_by_its_exact_name() {
        let api = local_api();
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let fake = Fake::default()
            .answer(
                "https://example.com/blog/wp-json/wp/v2/media?search=picasso-ab12&per_page=20&orderby=id&order=asc&context=edit&_fields=id,source_url,title,mime_type,media_details",
                200,
                &[],
                r#"[{"id":40,"source_url":"https://example.com/x.jpg","title":{"raw":"picasso-ab12-2"}},{"id":41,"source_url":"https://example.com/a.jpg","title":{"raw":"picasso-ab12"}}]"#,
            )
            .answer(
                "https://example.com/blog/wp-json/wp/v2/posts?search=Caf%C3%A9%20Sevilla&status=draft,publish&modified_after=2026-10-29T10%3A00%3A00Z*",
                200,
                &[],
                r#"[{"id":7,"title":{"raw":"Café Sevilla"}},{"id":8,"title":{"raw":"Café Sevilla 2"}}]"#,
            );
        let client = Client::new(&fake, Policy::default());
        assert_eq!(client.find_media(&api, &credentials, "picasso-ab12").unwrap(), vec![Media { id: 41, url: "https://example.com/a.jpg".into(), details: String::new() }]);
        assert_eq!(client.find_drafts(&api, &credentials, "Café Sevilla", 1_793_268_000).unwrap(), vec![7]);
        assert!(Failure::Timeout.outcome_unknown() && Failure::Status(502).outcome_unknown());
        assert!(!Failure::Forbidden.outcome_unknown() && !Failure::Status(400).outcome_unknown(), "a refusal is known not to be done");
    }

    pub(crate) fn local_api() -> Api {
        Api {
            site: SiteUrl::parse("https://example.com/blog").unwrap(),
            root: SiteUrl::parse("https://example.com/blog/wp-json/").unwrap(),
            name: String::new(),
            namespaces: vec!["wp/v2".into()],
        }
    }

    #[test]
    fn a_photo_is_uploaded_and_a_draft_made_and_changed_in_place() {
        let api = local_api();
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let fake = Fake::default()
            .answer_to("POST", "https://example.com/blog/wp-json/wp/v2/media", 201, &[], r#"{"id":41,"source_url":"https://example.com/blog/wp-content/uploads/a.jpg","mime_type":"image/jpeg","media_details":{"width":2048,"height":1365,"sizes":{"medium":{"source_url":"https://example.com/blog/wp-content/uploads/a-300x200.jpg","width":300}}}}"#)
            .answer_to("POST", "https://example.com/blog/wp-json/wp/v2/posts", 201, &[], r#"{"id":7,"status":"draft"}"#)
            .answer_to("POST", "https://example.com/blog/wp-json/wp/v2/posts/7", 200, &[], r#"{"id":7,"status":"draft","title":{"raw":"Spain"},"content":{"raw":"<p>y</p>"}}"#)
            .answer("https://example.com/blog/wp-json/wp/v2/posts/7?context=edit", 200, &[], r#"{"id":7,"status":"draft","title":{"raw":"Spain","rendered":"Spain"},"content":{"raw":"<p>y</p>"},"featured_media":0}"#)
            .answer("https://example.com/blog/wp-json/wp/v2/media/41?context=edit", 200, &[], r#"{"id":41}"#);
        let client = Client::new(&fake, Policy::default());
        let media = client.upload_media(&api, &credentials, b"jpeg", "picasso-ab12.jpg").unwrap();
        assert_eq!((media.id, media.url.as_str()), (41, "https://example.com/blog/wp-content/uploads/a.jpg"));
        let details: Value = serde_json::from_str(&media.details).unwrap();
        assert_eq!((details["width"].as_i64(), details["mime"].as_str()), (Some(2048), Some("image/jpeg")));
        assert_eq!(details["sizes"]["medium"], "https://example.com/blog/wp-content/uploads/a-300x200.jpg", "the sizes the site made are kept");
        assert!(client.media_exists(&api, &credentials, 41).unwrap());
        assert!(!client.media_exists(&api, &credentials, 42).unwrap(), "a 404 is a removed upload");

        assert_eq!(client.save_draft(&api, &credentials, None, &draft("Spain", "<p>x</p>", Some(41))).unwrap().id, 7);
        assert_eq!(client.post_state(&api, &credentials, 8).unwrap(), None);
        let saved = client.save_draft(&api, &credentials, Some(7), &draft("Spain", "<p>y</p>", None)).unwrap();
        let found = client.post_state(&api, &credentials, 7).unwrap().unwrap();
        assert_eq!(found, PostState { status: "draft".into(), fingerprint: saved.fingerprint.clone() }, "unchanged since the save");
        let edited = Fake::default().answer("https://example.com/blog/wp-json/wp/v2/posts/7?context=edit", 200, &[], r#"{"status":"draft","title":{"raw":"Spain"},"content":{"raw":"<p>edited</p>"}}"#);
        assert_ne!(Client::new(&edited, Policy::default()).post_state(&api, &credentials, 7).unwrap().unwrap().fingerprint, saved.fingerprint, "edited on the site");

        let bodies = fake.bodies.borrow();
        let json = |index: usize| serde_json::from_slice::<Value>(&bodies[index].1).unwrap();
        assert_eq!(bodies[0].1, b"jpeg");
        assert_eq!(json(1)["status"], "draft", "a new post is a draft");
        assert_eq!(json(1)["featured_media"], 41);
        assert!(json(2).get("status").is_none(), "an update leaves the status alone");
        assert_eq!(json(2)["featured_media"], 0);
        assert!(json(2).get("categories").is_none() && json(2).get("tags").is_none(), "terms not chosen are left as they are");
        assert_eq!(edit_link(&api, 7), "https://example.com/blog/wp-admin/post.php?post=7&action=edit");
    }

    #[test]
    fn a_post_is_scheduled_in_the_sites_own_time_read_from_its_clock() {
        let api = local_api();
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(9, 0, 0).unwrap();
        let posts = "https://example.com/blog/wp-json/wp/v2/posts";
        let fake = Fake::default().answer_to("POST", posts, 201, &[], r#"{"id":7,"status":"future"}"#);
        let post = DraftPost { publish: true, schedule: Some(at), ..draft("Spain", "c", None) };
        let saved = Client::new(&fake, Policy::default()).save_draft(&api, &credentials, None, &post).unwrap();
        assert_eq!(saved.status, "future");
        let sent: Value = serde_json::from_slice(&fake.bodies.borrow()[0].1).unwrap();
        assert_eq!((&sent["status"], &sent["date"]), (&Value::from("future"), &Value::from("2026-11-01T09:00:00")));
        assert!(sent.get("date_gmt").is_none(), "WordPress turns the site's time into UTC itself");

        // The site's clock: when its answer was sent, in its time zone.
        let root = "https://example.com/blog/wp-json/";
        for offset in ["5.5", r#""-3""#] {
            let index = format!(r#"{{"namespaces":["wp/v2"],"gmt_offset":{offset}}}"#);
            let fake = Fake::default().answer(root, 200, &[("date", "Sun, 01 Nov 2026 08:00:00 GMT")], &index);
            let now = Client::new(&fake, Policy::default()).site_now(&api).unwrap();
            let expected = if offset == "5.5" { "2026-11-01 13:30" } else { "2026-11-01 05:00" };
            assert_eq!(now.format("%Y-%m-%d %H:%M").to_string(), expected);
        }
    }

    #[test]
    fn categories_and_tags_are_read_page_by_page_and_sent_only_when_chosen() {
        let api = local_api();
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let page = |taxonomy: &str, n: u32| format!("https://example.com/blog/wp-json/wp/v2/{taxonomy}?per_page=100&page={n}&orderby=name&order=asc&_fields=id,name,parent");
        let many: Vec<String> = (1..=100).map(|id| format!(r#"{{"id":{id},"name":"Tag {id}","parent":0}}"#)).collect();
        let first = format!("[{}]", many.join(","));
        let fake = Fake::default()
            .answer(&page("categories", 1), 200, &[("x-wp-totalpages", "1")], r#"[{"id":2,"name":"Europe","parent":0},{"id":5,"name":"Food &amp; Wine","parent":2}]"#)
            .answer(&page("tags", 1), 200, &[("x-wp-totalpages", "2")], &first)
            .answer(&page("tags", 2), 200, &[("x-wp-totalpages", "2")], r#"[{"id":101,"name":"Alhambra"}]"#)
            .answer_to("POST", "https://example.com/blog/wp-json/wp/v2/posts", 201, &[], r#"{"id":7}"#);
        let client = Client::new(&fake, Policy::default());
        let categories = client.terms(&api, &credentials, Taxonomy::Categories).unwrap();
        assert_eq!(categories[1], Term { id: 5, name: "Food & Wine".into(), parent: 2 });
        let tags = client.terms(&api, &credentials, Taxonomy::Tags).unwrap();
        assert_eq!(tags.len(), 101, "every page is read");
        assert_eq!(tags[100].name, "Alhambra");
        assert!(fake.asked.borrow().iter().all(|(_, auth)| auth.is_some()));

        let post = DraftPost { categories: Some(vec![5]), tags: Some(vec![101, 3]), ..draft("Spain", "c", None) };
        client.save_draft(&api, &credentials, None, &post).unwrap();
        let bodies = fake.bodies.borrow();
        let sent: Value = serde_json::from_slice(&bodies[0].1).unwrap();
        assert_eq!(sent["categories"], serde_json::json!([5]));
        assert_eq!(sent["tags"], serde_json::json!([101, 3]));
        // Reading terms never writes: the only change sent was the post.
        assert_eq!(bodies.len(), 1);

        let missing = Fake::default();
        assert!(Client::new(&missing, Policy::default()).terms(&api, &credentials, Taxonomy::Tags).is_err());
    }

    #[test]
    fn a_change_is_never_sent_on_through_a_redirect() {
        let api = local_api();
        let credentials = Credentials { user: "ana".into(), password: "abcd efgh".into() };
        let fake = Fake::default().answer_to("POST", "https://example.com/blog/wp-json/wp/v2/posts", 301, &[("location", "https://example.com/new/")], "");
        let result = Client::new(&fake, Policy::default()).save_draft(&api, &credentials, None, &draft("t", "c", None));
        assert_eq!(result, Err(Failure::Status(301)));
        assert_eq!(fake.asked.borrow().len(), 1);
    }

    #[test]
    fn a_site_title_loses_its_html_entities() {
        assert_eq!(decode_entities("Kuoping&#039;s Travel"), "Kuoping's Travel");
        assert_eq!(decode_entities("A &amp; B &#x41; &lt;3"), "A & B A <3");
        assert_eq!(decode_entities("Fish & chips &unknown; &"), "Fish & chips &unknown; &");
    }

    #[test]
    fn the_password_stays_out_of_debug_output() {
        let credentials = Credentials { user: "ana".into(), password: "secret pass".into() };
        assert!(!format!("{credentials:?}").contains("secret"));
    }
}

