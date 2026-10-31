//! A Story as a web page: one self-contained HTML file with its styles
//! inline and its photos beside it, used for the preview and for export.
//! Photos are rendered through the edit pipeline; originals are never copied.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ring::digest;

use crate::edit::Settings;
use crate::inline;
use crate::trip::{Block, MediaUse, Story, StoryBlock};

/// How the page's photos are made. Its layout is [`Profile::VERSION`] in
/// the manifest; a change to what it means raises that number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Longest side, in pixels; smaller photos are not enlarged.
    pub edge: u32,
    pub quality: u8,
}

impl Profile {
    pub const VERSION: u32 = 1;
}

impl Default for Profile {
    fn default() -> Self {
        Self { edge: 1600, quality: 85 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// `index.html`, readable in any browser.
    #[default]
    Html,
    /// `story.md`, for blogs and notes that take Markdown.
    Markdown,
}

/// A photo written beside the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebImage {
    /// Relative to the page, named after the rendered bytes.
    pub file: String,
    pub width: u32,
    pub height: u32,
}

/// The words the page needs in the reader's language.
pub struct Words {
    pub lang: &'static str,
    pub untitled: String,
    pub unavailable: String,
    /// The heading of a Travel Guide's list of sections.
    pub contents: String,
}

/// Where a Story photo comes from: its file, edit, and catalog rotation,
/// or `None` while it is unavailable.
pub type Source = Option<(PathBuf, Settings, i32)>;

/// Where route maps' tiles come from: the map source chosen in Settings,
/// its key, and the folder the tile cache is in.
#[derive(Clone, Debug)]
pub struct MapTiles {
    pub source: crate::geo::MapSource,
    pub key: String,
    pub cache: PathBuf,
}

/// Photos with a shorter long side than this, before any crop, may look
/// soft across a page.
pub const LOW_RES_EDGE: u32 = 1200;

/// Everything a page of a Story is made from.
pub struct Page<'a> {
    pub story: &'a Story,
    /// The Trip's dates as shown, if wanted.
    pub dates: Option<&'a str>,
    pub blocks: &'a [StoryBlock],
    pub sources: &'a HashMap<i64, Source>,
    pub words: &'a Words,
    /// Tiles for the route maps; without them a route is drawn on its own.
    pub map: Option<&'a MapTiles>,
    /// Each Route block's elevation, by its uid, drawn in its map's corner.
    pub profiles: &'a HashMap<String, crate::elevation::Profile>,
}

/// How far a page's photos are, shared with the thread making them, and
/// a way to stop it.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

/// What to look at before exporting a Story.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Preflight {
    /// Nothing to export.
    pub empty: bool,
    /// Photos not in the library now, or whose file is gone. They are shown
    /// as unavailable only if the user says so.
    pub missing: usize,
    /// Photos whose file is there but can't be read as a photo. Like
    /// missing ones, they are shown as unavailable only if the user says so.
    pub unreadable: usize,
    /// Photos without a description for people who can't see them.
    pub no_alt: usize,
    /// Blocks from a newer Picasso, which can't be exported.
    pub unknown: usize,
    /// Photos smaller than [`LOW_RES_EDGE`] on their long side, which may
    /// look soft at the page's width. Only a warning.
    pub low_res: usize,
    /// The Story has no cover. Only a warning: the page starts with its title.
    pub no_cover: bool,
    /// The first block with each of those, to show it: missing, no
    /// description, newer, unreadable, low resolution.
    pub first: [Option<usize>; 5],
    /// Photos that will be rendered, for the size of the export.
    pub photos: usize,
}

impl Preflight {
    /// Whether export must wait for the user to decide.
    pub fn blocked(&self, skip_missing: bool) -> bool {
        self.empty || (self.missing + self.unreadable > 0 && !skip_missing)
    }
}

/// Checks a Story before it is exported. Reads whether files exist and
/// the head of each, not the whole photo.
pub fn preflight(page: &Page) -> Preflight {
    let mut check = Preflight { empty: page.blocks.is_empty(), no_cover: page.story.cover.is_none(), ..Preflight::default() };
    // Some(true) readable, Some(false) there but unreadable, None missing;
    // with the long side of the readable ones.
    let mut long_side: HashMap<i64, u32> = HashMap::new();
    let state: HashMap<i64, Option<bool>> = page
        .sources
        .iter()
        .map(|(&media_id, source)| {
            let state = match source {
                Some((path, _, _)) if path.is_file() => match crate::image_io::probe(path, 0) {
                    Ok(probe) => {
                        long_side.insert(media_id, probe.width.max(probe.height));
                        Some(true)
                    }
                    Err(_) => Some(false),
                },
                _ => None,
            };
            (media_id, state)
        })
        .collect();
    check.photos = state.values().filter(|state| **state == Some(true)).count();
    let available = |media_id: i64| state.get(&media_id).copied().flatten();
    match page.story.cover.map(available) {
        Some(None) => check.missing += 1,
        Some(Some(false)) => check.unreadable += 1,
        _ => {}
    }
    for (index, block) in page.blocks.iter().enumerate() {
        if matches!(block.block, Block::Unknown { .. }) {
            check.unknown += 1;
            check.first[2].get_or_insert(index);
            continue;
        }
        for media in block.block.media() {
            let readable = available(media.media_id);
            if readable.is_none() {
                check.missing += 1;
                check.first[0].get_or_insert(index);
            } else if readable == Some(false) {
                check.unreadable += 1;
                check.first[3].get_or_insert(index);
            } else {
                if media.alt.trim().is_empty() {
                    check.no_alt += 1;
                    check.first[1].get_or_insert(index);
                }
                if long_side.get(&media.media_id).is_some_and(|&side| side > 0 && side < LOW_RES_EDGE) {
                    check.low_res += 1;
                    check.first[4].get_or_insert(index);
                }
            }
        }
    }
    check
}

/// What making a page found: how many photos couldn't be shown, and how
/// many blocks this build can't show.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub missing: usize,
    pub unknown: usize,
}

/// Writes the Story in `folder` as `format`, with its photos in
/// `folder/images`, counting them in `progress`. Returns the page's file,
/// or `Err` with the reason, also when `progress` was cancelled.
pub fn write_story(folder: &Path, format: Format, profile: Profile, input: &Page, progress: &Progress) -> Result<(PathBuf, Outcome), String> {
    let Page { story, dates, blocks, sources, words, map, profiles } = *input;
    let images_dir = folder.join("images");
    std::fs::create_dir_all(&images_dir).map_err(|err| err.to_string())?;
    let mut images = HashMap::new();
    let mut order: Vec<(i64, &Source)> = sources.iter().map(|(id, source)| (*id, source)).collect();
    // A cropped cover is a picture of its own, beside the photo uncropped.
    if let (Some(key), Some(media)) = (story.cover_key().filter(|key| *key < 0), story.cover) {
        order.push((key, sources.get(&media).unwrap_or(&None)));
    }
    order.sort_by_key(|(id, _)| *id);
    progress.total.store(order.len(), Ordering::Relaxed);
    let mut read = Vec::new();
    for (media_id, source) in order {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        progress.done.fetch_add(1, Ordering::Relaxed);
        let Some((path, settings, rotation)) = source else { continue };
        let before = stamp(path);
        // A photo that can't be read is shown as unavailable, never left out.
        let Ok(rgba) = crate::edit::render_edited(path, settings, *rotation, Some(profile.edge)) else { continue };
        let rgba = if media_id < 0 { crop_cover(&rgba, story.cover_crop) } else { rgba };
        let bytes = crate::image_io::encode_jpeg(&rgba, profile.quality.clamp(1, 100))?;
        let file = format!("images/{}.jpg", key(&bytes));
        crate::image_io::write_whole(&folder.join(&file), &bytes)?;
        if media_id > 0 {
            read.push((media_id, path, before));
        }
        images.insert(media_id, WebImage { file, width: rgba.width(), height: rgba.height() });
    }
    // Each route on its map, as a picture like the photos.
    let mut maps = HashMap::new();
    for block in blocks {
        let Block::Route { route, .. } = &block.block else { continue };
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let Some(rgba) = map.and_then(|tiles| route_map(route, tiles, profiles.get(&block.uid).map(Vec::as_slice))) else { continue };
        let bytes = crate::image_io::encode_jpeg(&rgba, profile.quality.clamp(1, 100))?;
        let file = format!("images/{}.jpg", key(&bytes));
        crate::image_io::write_whole(&folder.join(&file), &bytes)?;
        let credit = map.map_or("", |tiles| tiles.source.attribution());
        maps.insert(block.uid.clone(), RouteImage { image: WebImage { file, width: rgba.width(), height: rgba.height() }, credit });
    }
    // The page is what the files were when read: one replaced or changed
    // meanwhile stops it, rather than mixing two versions.
    if read.iter().any(|(_, path, before)| stamp(path) != *before) {
        return Err(SOURCE_CHANGED.into());
    }
    let (text, outcome, name) = match format {
        Format::Html => {
            let (html, outcome) = page(story, dates, blocks, &images, &maps, words);
            (html, outcome, "index.html")
        }
        Format::Markdown => {
            let (markdown, outcome) = markdown(story, dates, blocks, &images, &maps, words);
            (markdown, outcome, "story.md")
        }
    };
    let index = folder.join(name);
    crate::image_io::write_whole(&index, text.as_bytes())?;
    let manifest = manifest(story, format, profile, &read, &images);
    crate::image_io::write_whole(&folder.join("manifest.json"), manifest.as_bytes())?;
    Ok((index, outcome))
}

/// The size and modification time of a source file, to notice a change.
type Stamp = Option<(u64, std::time::SystemTime)>;

fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// What the page was made from: the Story revision, the profile, and for
/// each photo its media reference, the file it became, and the size and
/// modification time of its source. No paths, names, or camera details.
fn manifest(story: &Story, format: Format, profile: Profile, read: &[(i64, &PathBuf, Stamp)], images: &HashMap<i64, WebImage>) -> String {
    let photos: Vec<_> = read
        .iter()
        .filter_map(|(media_id, _, stamp)| {
            let image = images.get(media_id)?;
            let (bytes, modified) = stamp.map(|(bytes, time)| (bytes, time.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))).unzip();
            Some(serde_json::json!({
                "media": media_id,
                "file": image.file,
                "width": image.width,
                "height": image.height,
                "source_bytes": bytes,
                "source_modified": modified,
            }))
        })
        .collect();
    let manifest = serde_json::json!({
        "manifest": 1,
        "story": story.id,
        "revision": story.revision,
        "format": match format { Format::Html => "html", Format::Markdown => "markdown" },
        "profile": { "version": Profile::VERSION, "edge": profile.edge, "quality": profile.quality, "metadata": "none" },
        "photos": photos,
    });
    serde_json::to_string_pretty(&manifest).unwrap_or_default()
}

/// About how many bytes an export of `photos` photos takes: JPEGs of the
/// profile's size at 3:2, plus the page. Meant for a free-space check, so
/// it errs on the large side.
pub fn estimated_bytes(photos: usize, profile: Profile) -> u64 {
    let pixels = u64::from(profile.edge) * u64::from(profile.edge) * 2 / 3;
    // Bits per pixel grow with quality: about 1.2 at 60, 3.6 at 95.
    let bits = 12 + (u64::from(profile.quality.clamp(60, 95)) - 60) * 24 / 35;
    photos as u64 * pixels * bits / 80 + 256 * 1024
}

/// Free bytes on the volume of `folder`, where the system says.
pub fn free_space(folder: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(folder.as_os_str().as_bytes()).ok()?;
        // SAFETY: an all-zero `statvfs` is valid, `path` is a valid C string,
        // and `stat` lives for the call.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = folder;
        None
    }
}

/// The reason [`write_story`] gives when it was cancelled.
pub const CANCELLED: &str = "cancelled";

/// The reason [`write_story`] gives when a photo's file changed while it ran.
pub const SOURCE_CHANGED: &str = "source changed";

/// Exports the Story into a new folder named `name` in `parent` (or
/// `name 2`, `name 3`, … when that is taken), so nothing there is ever
/// replaced. It is written under a hidden name first and renamed once
/// complete; a failed export leaves no folder behind. Returns the folder.
pub fn export_story(parent: &Path, name: &str, format: Format, profile: Profile, page: &Page, progress: &Progress) -> Result<(PathBuf, Outcome), String> {
    let name = folder_name(name);
    let target = (1..)
        .map(|n| if n == 1 { parent.join(&name) } else { parent.join(format!("{name} {n}")) })
        .find(|path| !path.exists())
        .unwrap_or_else(|| parent.join(&name));
    let staging = parent.join(format!(".{name}.partial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let written = write_story(&staging, format, profile, page, progress)
        .and_then(|(_, outcome)| std::fs::rename(&staging, &target).map(|()| outcome).map_err(|err| err.to_string()));
    match written {
        Ok(outcome) => Ok((target, outcome)),
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(err)
        }
    }
}

/// A folder name from a Story title: no path separators or characters
/// that some systems refuse, and never empty or hidden.
pub fn folder_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_start_matches('.').trim();
    let cleaned: String = cleaned.chars().take(80).collect();
    if cleaned.is_empty() {
        "Story".into()
    } else {
        cleaned
    }
}

/// The Story as Markdown. Photos are images with their alt text, and the
/// caption below in italics; missing ones are named as unavailable.
pub fn markdown(
    story: &Story,
    dates: Option<&str>,
    blocks: &[StoryBlock],
    images: &HashMap<i64, WebImage>,
    maps: &HashMap<String, RouteImage>,
    words: &Words,
) -> (String, Outcome) {
    let mut outcome = Outcome::default();
    let title = if story.title.trim().is_empty() { words.untitled.as_str() } else { story.title.trim() };
    let mut out = format!("# {}\n\n", md(title));
    if !story.subtitle.trim().is_empty() {
        let _ = write!(out, "*{}*\n\n", md(story.subtitle.trim()));
    }
    let byline: Vec<&str> = [Some(story.author.trim()).filter(|a| !a.is_empty()), dates.filter(|_| story.show_date)].into_iter().flatten().collect();
    if !byline.is_empty() {
        let _ = write!(out, "{}\n\n", md(&byline.join(" · ")));
    }
    if let Some(cover) = story.cover_key() {
        out.push_str(&md_image(&MediaUse::new(cover), images, words, &mut outcome));
    }
    let sections = headings(blocks);
    if crate::trip::template_code(&story.template) == "guide" && !sections.is_empty() {
        let _ = write!(out, "**{}**\n\n", md(&words.contents));
        for (n, text) in sections.iter().enumerate() {
            let _ = writeln!(out, "{}. {}", n + 1, inline::to_markdown(text));
        }
        out.push('\n');
    }
    md_paragraphs(&mut out, &story.introduction);
    for block in blocks {
        match &block.block {
            Block::Heading(text) => {
                let _ = write!(out, "## {}\n\n", inline::to_markdown(text.trim()));
            }
            Block::Subheading(text) => {
                let _ = write!(out, "### {}\n\n", inline::to_markdown(text.trim()));
            }
            Block::Paragraph(text) | Block::Credits(text) => md_paragraphs(&mut out, text),
            Block::Info { title, items } => {
                let items: Vec<&(String, String)> = items.iter().filter(|(label, value)| !label.trim().is_empty() || !value.trim().is_empty()).collect();
                if !title.trim().is_empty() {
                    let _ = write!(out, "**{}**\n\n", md(title.trim()));
                }
                for (label, value) in &items {
                    match (label.trim(), value.trim()) {
                        ("", value) => { let _ = writeln!(out, "- {}", inline::to_markdown(value)); }
                        (label, value) => { let _ = writeln!(out, "- **{}:** {}", md(label), inline::to_markdown(value)); }
                    }
                }
                if !items.is_empty() {
                    out.push('\n');
                }
            }
            Block::Link { url, title, note } => {
                let shown = if title.trim().is_empty() { url.trim() } else { title.trim() };
                let line = match crate::trip::web_address(url) {
                    Some(url) => format!("[{}](<{}>)", md(shown), url.replace('>', "%3E")),
                    None => md(shown),
                };
                match note.trim() {
                    "" if line.is_empty() => {}
                    "" => { let _ = write!(out, "{line}\n\n"); }
                    note => { let _ = write!(out, "{line} — {}\n\n", inline::to_markdown(note)); }
                }
            }
            Block::List { ordered, items } => {
                for (n, item) in items.iter().map(|item| item.trim()).filter(|item| !item.is_empty()).enumerate() {
                    let bullet = if *ordered { format!("{}.", n + 1) } else { "-".into() };
                    let _ = writeln!(out, "{bullet} {}", inline::to_markdown(item));
                }
                out.push('\n');
            }
            Block::Quote { text, citation } => {
                let mut quoted = String::new();
                md_paragraphs(&mut quoted, text);
                if !citation.trim().is_empty() {
                    let _ = write!(quoted, "— {}\n\n", inline::to_markdown(citation.trim()));
                }
                for line in quoted.trim_end().lines() {
                    let _ = writeln!(out, "> {line}");
                }
                out.push('\n');
            }
            Block::Photo(media) => out.push_str(&md_image(media, images, words, &mut outcome)),
            Block::Gallery(items) => {
                for media in items {
                    out.push_str(&md_image(media, images, words, &mut outcome));
                }
            }
            // Markdown has no grid: the photos in order, then the description.
            Block::Grid { caption, media, .. } => {
                for item in media {
                    let bare = MediaUse { caption: String::new(), ..item.clone() };
                    out.push_str(&md_image(&bare, images, words, &mut outcome));
                }
                if !caption.trim().is_empty() {
                    let _ = write!(out, "*{}*\n\n", md(caption.trim()));
                }
            }
            Block::Separator => out.push_str("---\n\n"),
            // The route's map, when there is one, then its words and length.
            Block::Route { caption, route, .. } => {
                if !route.is_empty() {
                    if let Some(RouteImage { image, credit }) = maps.get(&block.uid) {
                        let _ = write!(out, "![{}]({})\n{}\n\n", md(caption.trim()), image.file, md(credit));
                    }
                    let length = crate::track::format_distance(crate::trip::route_meters(route));
                    let words = [caption.trim(), &length].into_iter().filter(|text| !text.is_empty()).map(md).collect::<Vec<_>>().join(" · ");
                    let _ = write!(out, "*{words}*\n\n");
                }
            }
            Block::Unknown { .. } => outcome.unknown += 1,
        }
    }
    md_paragraphs(&mut out, &story.conclusion);
    (out, outcome)
}

/// Paragraphs split by blank lines, each line kept as its own.
fn md_paragraphs(out: &mut String, text: &str) {
    for part in text.split("\n\n").map(str::trim).filter(|part| !part.is_empty()) {
        let lines: Vec<String> = part.lines().map(|line| inline::to_markdown(line.trim_end())).collect();
        let _ = write!(out, "{}\n\n", lines.join("  \n"));
    }
}

fn md_image(media: &MediaUse, images: &HashMap<i64, WebImage>, words: &Words, outcome: &mut Outcome) -> String {
    let mut out = match images.get(&media.media_id) {
        Some(image) => {
            let alt = media.alt.trim().replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]").replace('\n', " ");
            format!("![{alt}]({})\n", image.file)
        }
        None => {
            outcome.missing += 1;
            format!("*{}*\n", md(&words.unavailable))
        }
    };
    if !media.caption.trim().is_empty() {
        let _ = writeln!(out, "*{}*", md(media.caption.trim()));
    }
    out.push('\n');
    out
}

/// Text that Markdown shows as written: its marks are escaped, and HTML
/// in it stays text.
pub fn md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '#' | '|' | '!' => {
                out.push('\\');
                out.push(c);
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(c),
        }
    }
    // A line that starts like a list or a numbered item stays a line of text.
    if let Some(first) = out.chars().next().filter(|c| matches!(c, '-' | '+')) {
        out.replace_range(..first.len_utf8(), &format!("\\{first}"));
    }
    if let Some(dot) = out.find(". ").filter(|&at| at > 0 && out[..at].chars().all(|c| c.is_ascii_digit())) {
        out.replace_range(dot..dot + 1, "\\.");
    }
    out
}

/// A name for rendered bytes, the same whenever the bytes are.
fn key(bytes: &[u8]) -> String {
    digest::digest(&digest::SHA256, bytes).as_ref()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// The page's HTML. Photos missing from `images` are shown as unavailable.
pub fn page(
    story: &Story,
    dates: Option<&str>,
    blocks: &[StoryBlock],
    images: &HashMap<i64, WebImage>,
    maps: &HashMap<String, RouteImage>,
    words: &Words,
) -> (String, Outcome) {
    let mut outcome = Outcome::default();
    let title = if story.title.trim().is_empty() { words.untitled.as_str() } else { story.title.trim() };
    let template = crate::trip::template_code(&story.template);
    let mut html = String::new();
    let _ = write!(
        html,
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>{STYLE}{}</style>\n</head>\n<body class=\"{template}\">\n<article>\n<header>\n",
        esc(words.lang),
        esc(title),
        template_style(template)
    );
    if let Some(cover) = story.cover_key() {
        html.push_str(&figure(&MediaUse::new(cover), images, words, "cover", &mut outcome));
    }
    let _ = writeln!(html, "<h1>{}</h1>", esc(title));
    if !story.subtitle.trim().is_empty() {
        let _ = writeln!(html, "<p class=\"subtitle\">{}</p>", esc(story.subtitle.trim()));
    }
    let byline: Vec<&str> = [Some(story.author.trim()).filter(|a| !a.is_empty()), dates.filter(|_| story.show_date)].into_iter().flatten().collect();
    if !byline.is_empty() {
        let _ = writeln!(html, "<p class=\"byline\">{}</p>", esc(&byline.join(" · ")));
    }
    html.push_str("</header>\n");
    let sections = headings(blocks);
    if template == "guide" && !sections.is_empty() {
        let _ = write!(html, "<nav class=\"contents\" aria-labelledby=\"contents\">\n<h2 id=\"contents\">{}</h2>\n<ol>\n", esc(&words.contents));
        for (n, text) in sections.iter().enumerate() {
            let _ = writeln!(html, "<li><a href=\"#section-{}\">{}</a></li>", n + 1, inline::to_html(text));
        }
        html.push_str("</ol>\n</nav>\n");
    }
    if !story.introduction.trim().is_empty() {
        let _ = write!(html, "<section class=\"introduction\">\n{}</section>\n", paragraphs(&story.introduction));
    }
    let mut section = 0;
    for block in blocks {
        match &block.block {
            Block::Heading(text) if !text.trim().is_empty() => {
                section += 1;
                let _ = writeln!(html, "<h2 id=\"section-{section}\">{}</h2>", inline::to_html(text.trim()));
            }
            Block::Heading(_) => {}
            Block::Subheading(text) => {
                let _ = writeln!(html, "<h3>{}</h3>", inline::to_html(text.trim()));
            }
            Block::Paragraph(text) => html.push_str(&paragraphs(text)),
            Block::List { ordered, items } => {
                let tag = if *ordered { "ol" } else { "ul" };
                let _ = writeln!(html, "<{tag}>");
                for item in items.iter().map(|item| item.trim()).filter(|item| !item.is_empty()) {
                    let _ = writeln!(html, "<li>{}</li>", inline::to_html(item));
                }
                let _ = writeln!(html, "</{tag}>");
            }
            Block::Quote { text, citation } => {
                let _ = write!(html, "<blockquote>\n{}", paragraphs(text));
                if !citation.trim().is_empty() {
                    let _ = writeln!(html, "<cite>{}</cite>", inline::to_html(citation.trim()));
                }
                html.push_str("</blockquote>\n");
            }
            Block::Photo(media) => html.push_str(&figure(media, images, words, "photo", &mut outcome)),
            Block::Gallery(items) => {
                html.push_str("<div class=\"gallery\">\n");
                for media in items {
                    html.push_str(&figure(media, images, words, "", &mut outcome));
                }
                html.push_str("</div>\n");
            }
            Block::Grid { columns, caption, media, .. } => {
                let ratio = if *columns == 1 { "3/2" } else { "4/3" };
                let _ = writeln!(html, "<figure class=\"grid\" style=\"--cols:{columns};--ratio:{ratio}\"><div class=\"cells\">");
                for item in media {
                    match images.get(&item.media_id) {
                        Some(image) => {
                            let _ = writeln!(
                                html,
                                "<img src=\"{}\" width=\"{}\" height=\"{}\" alt=\"{}\" loading=\"lazy\">",
                                esc(&image.file),
                                image.width,
                                image.height,
                                esc(item.alt.trim())
                            );
                        }
                        None => {
                            outcome.missing += 1;
                            let _ = writeln!(html, "<div class=\"missing\">{}</div>", esc(&words.unavailable));
                        }
                    }
                }
                html.push_str("</div>");
                if !caption.trim().is_empty() {
                    let _ = write!(html, "<figcaption>{}</figcaption>", esc(caption.trim()));
                }
                html.push_str("</figure>\n");
            }
            Block::Separator => html.push_str("<hr>\n"),
            Block::Route { caption, route, .. } => match maps.get(&block.uid) {
                Some(RouteImage { image, credit }) => html.push_str(&route_picture(route, caption, image, credit)),
                None => html.push_str(&route_figure(route, caption)),
            },
            Block::Credits(text) => {
                let _ = write!(html, "<footer class=\"credits\">\n{}</footer>\n", paragraphs(text));
            }
            Block::Info { title, items } => html.push_str(&info_html(title, items)),
            Block::Link { url, title, note } => html.push_str(&link_html(url, title, note)),
            Block::Unknown { .. } => outcome.unknown += 1,
        }
    }
    if !story.conclusion.trim().is_empty() {
        let _ = write!(html, "<section class=\"conclusion\">\n{}</section>\n", paragraphs(&story.conclusion));
    }
    html.push_str("</article>\n</body>\n</html>\n");
    (html, outcome)
}

/// A route drawn on its map, written beside the page, and the credit its
/// map tiles ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteImage {
    pub image: WebImage,
    pub credit: &'static str,
}

/// Width and height of a route's drawing, before the page scales it.
const ROUTE_SIZE: (f32, f32) = (720.0, 400.0);

/// A Route block as a drawing of its line, from a dot where it starts to
/// a ring where it ends, with its words and length below. Nothing for a
/// route without points.
pub fn route_figure(route: &[Vec<[i32; 2]>], caption: &str) -> String {
    let (width, height) = ROUTE_SIZE;
    let Some((_, shape)) = crate::trip::route_shape(route, width, height, ROUTE_PAD) else {
        return String::new();
    };
    let (Some(start), Some(end)) = (shape.first().and_then(|segment| segment.first()), shape.last().and_then(|segment| segment.last())) else {
        return String::new();
    };
    let mut svg = format!("<svg viewBox=\"0 0 {width} {height}\" role=\"img\" aria-label=\"{}\">", esc(caption.trim()));
    for segment in &shape {
        let points: Vec<String> = segment.iter().map(|[x, y]| format!("{x:.1},{y:.1}")).collect();
        let _ = write!(svg, "<polyline points=\"{}\"/>", points.join(" "));
    }
    let _ = write!(svg, "<circle class=\"start\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"6\"/>", start[0], start[1]);
    let _ = write!(svg, "<circle class=\"end\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"6\"/></svg>", end[0], end[1]);
    format!("<figure class=\"route\">{svg}<figcaption>{}</figcaption></figure>\n", route_words(route, caption, ""))
}

/// A Route block as its picture on the map, with its words, length, and
/// the map's credit below.
fn route_picture(route: &[Vec<[i32; 2]>], caption: &str, image: &WebImage, credit: &str) -> String {
    format!(
        "<figure class=\"route\"><img src=\"{}\" width=\"{}\" height=\"{}\" alt=\"{}\" loading=\"lazy\"><figcaption>{}</figcaption></figure>\n",
        esc(&image.file),
        image.width,
        image.height,
        esc(caption.trim()),
        route_words(route, caption, credit)
    )
}

/// A route's description and length, and the map's credit, as escaped text.
pub fn route_words(route: &[Vec<[i32; 2]>], caption: &str, credit: &str) -> String {
    let length = crate::track::format_distance(crate::trip::route_meters(route));
    let words = [esc(caption.trim()), esc(&length)].into_iter().filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" · ");
    if credit.is_empty() {
        words
    } else {
        format!("{words}<small class=\"credit\">{}</small>", esc(credit))
    }
}

/// Room around a route on its map, at [`ROUTE_SIZE`].
const ROUTE_PAD: f32 = 28.0;
/// How many pixels a route map has for each of [`ROUTE_SIZE`]'s, so it is
/// sharp on fine screens.
const ROUTE_SCALE: u32 = 2;
/// The route's colour on its map, the page's accent.
const ROUTE_INK: [u8; 3] = [0x1f, 0x7a, 0x7a];

/// A route drawn on the map tiles of `tiles`: the map one zoom closer than
/// the drawing so it is [`ROUTE_SCALE`] times as sharp, the line in white
/// and then the accent, a dot where it starts and a ring where it ends.
/// `None` when the source needs a key it doesn't have or no tile could be
/// had, so the page draws the route on its own. A `profile` is drawn in
/// its bottom-right corner.
pub fn route_map(route: &[Vec<[i32; 2]>], tiles: &MapTiles, profile: Option<&[[f32; 2]]>) -> Option<image::RgbaImage> {
    if tiles.source.needs_key() && tiles.key.trim().is_empty() {
        return None;
    }
    let (width, height) = ROUTE_SIZE;
    let (view, shape) = crate::trip::route_shape(route, width, height, ROUTE_PAD)?;
    let scale = ROUTE_SCALE as f32;
    let (w, h) = (width as u32 * ROUTE_SCALE, height as u32 * ROUTE_SCALE);
    let zoom = view.zoom + 1;
    let (left, top) = (view.x * f64::from(ROUTE_SCALE) - f64::from(w) / 2.0, view.y * f64::from(ROUTE_SCALE) - f64::from(h) / 2.0);
    let tile = crate::geo::TILE_PX;
    let last = (1u32 << zoom) - 1;
    let range = |from: f64, size: u32| (from / tile).floor().max(0.0) as u32..=(((from + f64::from(size)) / tile).floor().max(0.0) as u32).min(last);
    let jobs: Vec<crate::geo::TileJob> = range(top, h)
        .flat_map(|y| range(left, w).map(move |x| (x, y)))
        .map(|(x, y)| crate::geo::TileJob { source: tiles.source, key: tiles.key.clone(), z: zoom, x, y })
        .collect();
    let loaded = crate::geo::load_tiles(&jobs, &tiles.cache);
    let mut picture = image::RgbaImage::from_pixel(w, h, image::Rgba([0xe8, 0xe4, 0xdc, 0xff]));
    let mut any = false;
    for (job, loaded) in jobs.iter().zip(loaded) {
        let Some(loaded) = loaded else { continue };
        let Some(pixels) = image::RgbaImage::from_raw(loaded.w as u32, loaded.h as u32, loaded.rgba) else { continue };
        let side = tile as u32;
        let pixels = if pixels.width() == side { pixels } else { image::imageops::resize(&pixels, side, side, image::imageops::FilterType::Triangle) };
        let (x, y) = (f64::from(job.x) * tile - left, f64::from(job.y) * tile - top);
        image::imageops::overlay(&mut picture, &pixels, x.round() as i64, y.round() as i64);
        any = true;
    }
    if !any {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::from_vec(picture.into_raw(), tiny_skia::IntSize::from_wh(w, h)?)?;
    let mut line = tiny_skia::PathBuilder::new();
    for segment in &shape {
        for (n, [x, y]) in segment.iter().enumerate() {
            if n == 0 {
                line.move_to(x * scale, y * scale);
            } else {
                line.line_to(x * scale, y * scale);
            }
        }
    }
    let line = line.finish()?;
    let paint = |[r, g, b]: [u8; 3]| {
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(r, g, b, 255);
        paint.anti_alias = true;
        paint
    };
    let (white, ink) = (paint([255, 255, 255]), paint(ROUTE_INK));
    let stroke = |width: f32| tiny_skia::Stroke { width: width * scale, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
    pixmap.stroke_path(&line, &white, &stroke(7.0), tiny_skia::Transform::identity(), None);
    pixmap.stroke_path(&line, &ink, &stroke(4.0), tiny_skia::Transform::identity(), None);
    let ends = [shape.first().and_then(|segment| segment.first()), shape.last().and_then(|segment| segment.last())];
    for (n, [x, y]) in ends.into_iter().flatten().enumerate() {
        let Some(dot) = tiny_skia::PathBuilder::from_circle(x * scale, y * scale, 6.0 * scale) else { continue };
        pixmap.fill_path(&dot, if n == 0 { &ink } else { &white }, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        pixmap.stroke_path(&dot, if n == 0 { &white } else { &ink }, &stroke(3.0), tiny_skia::Transform::identity(), None);
    }
    if let Some(profile) = profile {
        crate::elevation::draw(&mut pixmap, profile, scale, ROUTE_INK);
    }
    image::RgbaImage::from_raw(w, h, pixmap.take())
}

fn figure(media: &MediaUse, images: &HashMap<i64, WebImage>, words: &Words, class: &str, outcome: &mut Outcome) -> String {
    let class = if class.is_empty() { String::new() } else { format!(" class=\"{class}\"") };
    let caption = if media.caption.trim().is_empty() {
        String::new()
    } else {
        format!("<figcaption>{}</figcaption>", esc(media.caption.trim()))
    };
    match images.get(&media.media_id) {
        Some(image) => format!(
            "<figure{class}><img src=\"{}\" width=\"{}\" height=\"{}\" alt=\"{}\" loading=\"lazy\">{caption}</figure>\n",
            esc(&image.file),
            image.width,
            image.height,
            esc(media.alt.trim())
        ),
        None => {
            outcome.missing += 1;
            format!("<figure{class}><div class=\"missing\">{}</div>{caption}</figure>\n", esc(&words.unavailable))
        }
    }
}

/// The part of a rendered cover the Story shows.
pub fn crop_cover(rgba: &image::RgbaImage, crop: crate::trip::CoverCrop) -> image::RgbaImage {
    let [left, top, right, bottom] = crop.fractions();
    let (w, h) = (rgba.width() as f32, rgba.height() as f32);
    let x = ((left * w).round() as u32).min(rgba.width().saturating_sub(1));
    let y = ((top * h).round() as u32).min(rgba.height().saturating_sub(1));
    let cw = (((right - left) * w).round() as u32).clamp(1, rgba.width() - x);
    let ch = (((bottom - top) * h).round() as u32).clamp(1, rgba.height() - y);
    image::imageops::crop_imm(rgba, x, y, cw, ch).to_image()
}

/// Text as paragraphs: a blank line starts a new one, a line break stays.
fn paragraphs(text: &str) -> String {
    let mut html = String::new();
    for part in text.split("\n\n").map(str::trim).filter(|part| !part.is_empty()) {
        let lines: Vec<String> = part.lines().map(inline::to_html).collect();
        let _ = writeln!(html, "<p>{}</p>", lines.join("<br>\n"));
    }
    html
}

/// Text safe in HTML content and in quoted attributes.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A Practical info block: its title and a list of label and value lines.
/// Lines with neither are left out, and an empty block draws nothing.
pub fn info_html(title: &str, items: &[(String, String)]) -> String {
    let items: Vec<&(String, String)> = items.iter().filter(|(label, value)| !label.trim().is_empty() || !value.trim().is_empty()).collect();
    if title.trim().is_empty() && items.is_empty() {
        return String::new();
    }
    let mut html = String::from("<aside class=\"info\">\n");
    if !title.trim().is_empty() {
        let _ = writeln!(html, "<h3>{}</h3>", esc(title.trim()));
    }
    if !items.is_empty() {
        html.push_str("<dl>\n");
        for (label, value) in items {
            let _ = writeln!(html, "<dt>{}</dt><dd>{}</dd>", esc(label.trim()), inline::to_html(value.trim()));
        }
        html.push_str("</dl>\n");
    }
    html.push_str("</aside>\n");
    html
}

/// A Link block: its title linked to its address, with the note below.
/// An address that isn't a web page is shown as text, not followed.
pub fn link_html(url: &str, title: &str, note: &str) -> String {
    let shown = if title.trim().is_empty() { url.trim() } else { title.trim() };
    if shown.is_empty() && note.trim().is_empty() {
        return String::new();
    }
    let link = match crate::trip::web_address(url) {
        Some(url) => format!("<a href=\"{}\" rel=\"noopener\">{}</a>", esc(url), esc(shown)),
        None => esc(shown),
    };
    let note = if note.trim().is_empty() { String::new() } else { format!("<br><span class=\"note\">{}</span>", inline::to_html(note.trim())) };
    format!("<p class=\"link\">{link}{note}</p>\n")
}

/// The Story's section headings, in order, for a Travel Guide's contents.
fn headings(blocks: &[StoryBlock]) -> Vec<&str> {
    blocks
        .iter()
        .filter_map(|block| match &block.block {
            Block::Heading(text) => Some(text.trim()).filter(|text| !text.is_empty()),
            _ => None,
        })
        .collect()
}

/// What a template adds to [`STYLE`], the Photography Journal.
fn template_style(template: &str) -> &'static str {
    match template {
        "diary" => DIARY,
        "guide" => GUIDE,
        "essay" => ESSAY,
        _ => "",
    }
}

/// Travel Diary: a notebook page. Warm paper, a narrower column, photos
/// as prints with a white border, a handwritten-looking byline.
const DIARY: &str = "
body.diary{--bg:#f6efe1;--text:#2b2620;--muted:#7a6d5c;--rule:#d9cbb2;--accent:#9a4a2b;font-size:19px}
@media (prefers-color-scheme:dark){body.diary{--bg:#221d17;--text:#efe6d6;--muted:#b3a58f;--rule:#4a3f31;--accent:#e08a62}}
.diary article{max-width:38rem}
.diary h1,.diary h2{font-family:Georgia,'Iowan Old Style','Noto Serif',serif;font-style:italic;font-weight:normal}
.diary h2{border-bottom:1px dashed var(--rule);padding-bottom:.3rem}
.diary .byline{font-family:'Bradley Hand','Segoe Print','Comic Neue',cursive;font-size:1.1rem}
.diary figure.photo,.diary figure.cover{margin-left:0;margin-right:0}
.diary figure.photo img,.diary .gallery img{background:#fff;padding:.6rem .6rem 1.8rem;border-radius:2px;box-shadow:0 2px 8px rgba(0,0,0,.18)}
.diary figure.photo:nth-of-type(odd){transform:rotate(-.8deg)}
.diary figure.photo:nth-of-type(even){transform:rotate(.8deg)}
.diary figcaption{font-family:'Bradley Hand','Segoe Print','Comic Neue',cursive;font-size:1rem;text-align:center}
.diary hr{border-top-style:dashed}
@media (min-width:52rem){.diary figure.photo,.diary figure.cover,.diary figure.grid,.diary figure.route{margin-left:0;margin-right:0}}
@media (prefers-reduced-motion:reduce){.diary figure.photo{transform:none}}
";

/// Travel Guide: something to use on the way. Sans-serif, numbered
/// sections listed at the top, quotes as boxed tips, smaller photos.
const GUIDE: &str = "
body.guide{--bg:#ffffff;--text:#1c2430;--muted:#5d6878;--rule:#dfe4ea;--accent:#1d5fa8;font:17px/1.6 system-ui,-apple-system,'Segoe UI',sans-serif;counter-reset:section}
@media (prefers-color-scheme:dark){body.guide{--bg:#12161c;--text:#e6ebf1;--muted:#9aa5b4;--rule:#2b333d;--accent:#7fb2ee}}
.guide article{max-width:50rem}
.guide h1{font-size:2.2rem}
.guide article>h2{counter-increment:section;border-top:2px solid var(--accent);padding-top:.8rem}
.guide article>h2::before{content:counter(section) '. ';color:var(--muted)}
.guide .contents{background:var(--rule);border-radius:6px;padding:1rem 1.25rem;margin:0 0 2.5rem}
.guide .contents h2{margin:0 0 .5rem;font-size:1rem;color:var(--text);text-transform:uppercase;letter-spacing:.06em}
.guide .contents ol{margin:0;padding-left:1.4rem}
.guide .contents a{color:var(--accent)}
.guide blockquote{font-style:normal;border-left:0;background:color-mix(in srgb,var(--accent) 10%,var(--bg));border-radius:6px;padding:1rem 1.25rem}
.guide figure.photo,.guide figure.cover{margin-left:0;margin-right:0}
.guide .gallery{grid-template-columns:repeat(auto-fill,minmax(10rem,1fr))}
@media (min-width:52rem){.guide figure.photo,.guide figure.grid,.guide figure.route{margin-left:0;margin-right:0}}
";

/// Photo Essay: the photos lead. Dark page, photos edge to edge, one per
/// row, short text in a narrow centered column.
const ESSAY: &str = "
body.essay{--bg:#0e0e0e;--text:#e9e9e9;--muted:#9a9a9a;--rule:#2a2a2a;--accent:#d8c49a;color-scheme:dark}
.essay article{max-width:none;padding:0 0 5rem}
.essay article>*{max-width:36rem;margin-left:auto;margin-right:auto;padding-left:1.25rem;padding-right:1.25rem}
.essay header{max-width:none;padding:0;text-align:center}
.essay header>*:not(figure){max-width:40rem;margin-left:auto;margin-right:auto;padding:0 1.25rem}
.essay h1{font:300 3rem/1.1 Georgia,'Iowan Old Style','Noto Serif',serif;letter-spacing:.02em;margin-top:2.5rem}
.essay h2{font:300 1.6rem/1.3 Georgia,'Iowan Old Style','Noto Serif',serif;color:var(--text);text-align:center}
.essay article>figure.photo,.essay article>figure.grid,.essay article>figure.route,.essay header figure.cover,.essay article>.gallery{max-width:none;margin:3.5rem 0;padding:0}
.essay img{border-radius:0}
.essay figcaption{text-align:center;font-style:italic}
.essay .gallery{display:block}
.essay .gallery figure{margin:0 0 3.5rem}
.essay hr{width:3rem;border-top-color:var(--accent)}
@media (min-width:52rem){.essay figure.photo,.essay figure.cover,.essay figure.grid,.essay figure.route{margin-left:0;margin-right:0}}
";

/// Photography Journal: one wide column, large photos, quiet type. System
/// fonts only, so the page needs no network. The other templates add to it.
const STYLE: &str = "
:root{color-scheme:light dark;--bg:#fbfaf7;--text:#1f1f1f;--muted:#6b6b6b;--rule:#ddd8cf;--accent:#1f7a7a}
@media (prefers-color-scheme:dark){:root{--bg:#161616;--text:#ececec;--muted:#a0a0a0;--rule:#333;--accent:#5cc2c2}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:18px/1.65 Georgia,'Iowan Old Style','Noto Serif',serif}
article{max-width:46rem;margin:0 auto;padding:3rem 1.25rem 4rem}
header{margin-bottom:2.5rem}
h1{font:600 2.4rem/1.15 system-ui,-apple-system,'Segoe UI',sans-serif;margin:1.5rem 0 .4rem;letter-spacing:-.01em}
h2{font:600 1.45rem/1.3 system-ui,-apple-system,'Segoe UI',sans-serif;margin:3rem 0 1rem;color:var(--accent)}
.subtitle{font-size:1.2rem;color:var(--muted);margin:0 0 .6rem}
.byline{font:.9rem system-ui,-apple-system,'Segoe UI',sans-serif;color:var(--muted);margin:0}
p{margin:0 0 1.2rem}
figure{margin:2rem 0}
figure.photo,figure.cover{margin-left:-1.25rem;margin-right:-1.25rem}
img{display:block;width:100%;height:auto;border-radius:4px}
figcaption{font:.85rem/1.5 system-ui,-apple-system,'Segoe UI',sans-serif;color:var(--muted);margin-top:.5rem;padding:0 1.25rem}
.gallery{display:grid;grid-template-columns:repeat(auto-fill,minmax(14rem,1fr));gap:.75rem;margin:2rem 0}
.gallery figure{margin:0}
.gallery figcaption{padding:0}
figure.grid{margin-left:-1.25rem;margin-right:-1.25rem}
.grid .cells{display:grid;grid-template-columns:repeat(var(--cols),1fr);gap:.35rem}
.grid .cells img,.grid .cells .missing{width:100%;height:auto;aspect-ratio:var(--ratio);object-fit:cover}
.grid .cells img{display:block}
.missing{aspect-ratio:3/2;display:flex;align-items:center;justify-content:center;background:var(--rule);color:var(--muted);border-radius:4px;font:.9rem system-ui,sans-serif}
hr{border:0;border-top:1px solid var(--rule);margin:3rem auto;width:40%}
blockquote{margin:2rem 0;padding-left:1.25rem;border-left:3px solid var(--rule);font-style:italic}
blockquote cite{display:block;font-style:normal;font-size:.9rem;color:var(--muted)}
blockquote cite::before{content:'— '}
code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.9em}
figure.route svg{display:block;width:100%;height:auto;background:var(--rule);border-radius:4px}
.route polyline{fill:none;stroke:var(--accent);stroke-width:4;stroke-linecap:round;stroke-linejoin:round}
.route circle{stroke:var(--accent);stroke-width:3}
.route .start{fill:var(--accent)}
.route .end{fill:var(--bg)}
figure.route img{border-radius:4px}
.route .credit{display:block;font-size:.75rem}
.info{margin:2rem 0;padding:1rem 1.25rem;border:1px solid var(--rule);border-radius:6px;font:.95rem/1.5 system-ui,-apple-system,'Segoe UI',sans-serif}
.info h3{margin:0 0 .6rem;font-size:1rem}
.info dl{margin:0;display:grid;grid-template-columns:minmax(6rem,max-content) 1fr;gap:.35rem 1rem}
.info dt{font-weight:600}
.info dd{margin:0}
.link{padding-left:1rem;border-left:3px solid var(--accent)}
.link a{color:var(--accent);font-weight:600}
.link .note{color:var(--muted);font-size:.95rem}
.credits{margin-top:3rem;padding-top:1.5rem;border-top:1px solid var(--rule);font-size:.9rem;color:var(--muted)}
@media (min-width:52rem){figure.photo,figure.cover,figure.grid,figure.route{margin-left:-4rem;margin-right:-4rem}}
";

#[cfg(test)]
mod tests {
    use super::*;

    fn story() -> Story {
        Story {
            id: 1,
            trip_id: 1,
            title: "Kyoto <in> spring".into(),
            subtitle: String::new(),
            introduction: String::new(),
            conclusion: String::new(),
            author: "Ana".into(),
            show_date: true,
            cover: None,
            cover_crop: crate::trip::CoverCrop::default(),
            template: crate::trip::DEFAULT_TEMPLATE.into(),
            archived: false,
            finished: false,
            revision: 1,
            updated_at: 0,
        }
    }

    fn words() -> Words {
        Words { lang: "en", untitled: "Untitled story".into(), unavailable: "Photo unavailable".into(), contents: "Contents".into() }
    }

    #[test]
    fn practical_info_and_links_are_escaped_and_only_web_addresses_followed() {
        let blocks = vec![
            StoryBlock::new(Block::Info { title: "Getting <there>".into(), items: vec![("Train".into(), "2 h, *book ahead*".into()), (" ".into(), " ".into())] }),
            StoryBlock::new(Block::Info { title: " ".into(), items: vec![(String::new(), String::new())] }),
            StoryBlock::new(Block::Link { url: "https://example.com/?a=1&b=2".into(), title: "Ferry \"times\"".into(), note: "Summer only".into() }),
            StoryBlock::new(Block::Link { url: "javascript:alert(1)".into(), title: "Trick".into(), note: String::new() }),
        ];
        let (html, _) = page(&story(), None, &blocks, &HashMap::new(), &HashMap::new(), &words());
        assert!(html.contains("<aside class=\"info\">\n<h3>Getting &lt;there&gt;</h3>\n<dl>\n<dt>Train</dt><dd>2 h, <em>book ahead</em></dd>\n</dl>\n</aside>"), "{html}");
        assert_eq!(html.matches("<aside").count(), 1, "an empty block draws nothing");
        assert!(html.contains("<p class=\"link\"><a href=\"https://example.com/?a=1&amp;b=2\" rel=\"noopener\">Ferry &quot;times&quot;</a><br><span class=\"note\">Summer only</span></p>"));
        assert!(html.contains("<p class=\"link\">Trick</p>") && !html.contains("javascript:"));
        let (md, _) = markdown(&story(), None, &blocks, &HashMap::new(), &HashMap::new(), &words());
        assert!(md.contains("**Getting &lt;there&gt;**\n\n- **Train:** 2 h, *book ahead*\n\n"), "{md}");
        assert!(md.contains("[Ferry \"times\"](<https://example.com/?a=1&b=2>) — Summer only") && md.contains("Trick\n\n") && !md.contains("javascript:"), "{md}");
    }

    #[test]
    fn small_photos_and_a_missing_cover_are_only_warnings() {
        let folder = std::env::temp_dir().join(format!("picasso-small-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let small = folder.join("small.jpg");
        image::RgbImage::from_pixel(800, 600, image::Rgb([90, 120, 150])).save(&small).unwrap();
        let large = Path::new(env!("CARGO_MANIFEST_DIR")).join("images/banner.jpg");
        let sources = HashMap::from([(1, Some((small, Settings::default(), 0))), (2, Some((large, Settings::default(), 0)))]);
        let mut first = MediaUse::new(2);
        first.alt = "Harbor".into();
        let mut second = MediaUse::new(1);
        second.alt = "Boats".into();
        let blocks = [StoryBlock::new(Block::Photo(first)), StoryBlock::new(Block::Photo(second))];
        let mut story = story();
        let words = words();
        let check = preflight(&Page { story: &story, dates: None, blocks: &blocks, sources: &sources, words: &words, map: None, profiles: &HashMap::new() });
        assert_eq!((check.low_res, check.first[4], check.no_cover), (1, Some(1), true));
        assert!(!check.blocked(false), "neither stops an export");
        story.cover = Some(2);
        let check = preflight(&Page { story: &story, dates: None, blocks: &blocks, sources: &sources, words: &words, map: None, profiles: &HashMap::new() });
        assert!(!check.no_cover);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn each_template_changes_the_page_but_not_its_blocks() {
        let blocks = vec![
            StoryBlock::new(Block::Heading("Getting there".into())),
            StoryBlock::new(Block::Paragraph("Take the early train.".into())),
            StoryBlock::new(Block::Heading("Where to eat".into())),
        ];
        let render = |template: &str| {
            let story = Story { template: template.into(), ..story() };
            page(&story, None, &blocks, &HashMap::new(), &HashMap::new(), &words()).0
        };
        let pages: Vec<String> = crate::trip::TEMPLATES.iter().map(|code| render(code)).collect();
        for (n, html) in pages.iter().enumerate() {
            assert!(html.contains(&format!("<body class=\"{}\">", crate::trip::TEMPLATES[n])));
            assert!(html.contains("<h2 id=\"section-2\">Where to eat</h2>") && html.contains("Take the early train."));
            assert!(pages.iter().enumerate().all(|(m, other)| m == n || other != html), "every template looks different");
        }
        let guide = &pages[2];
        assert!(guide.contains("<nav class=\"contents\"") && guide.contains("<a href=\"#section-1\">Getting there</a>"));
        assert!(!pages[0].contains("<nav"), "only the guide lists its sections");
        assert_eq!(render("unknown"), pages[0], "an unknown template reads as the journal");

        let guide_story = Story { template: "guide".into(), ..story() };
        let (md, _) = markdown(&guide_story, None, &blocks, &HashMap::new(), &HashMap::new(), &words());
        assert!(md.contains("**Contents**\n\n1. Getting there\n2. Where to eat\n"));
    }

    #[test]
    fn a_route_on_its_map_is_a_picture_with_the_map_credit() {
        let route = vec![vec![[3_500_000, 13_500_000], [3_510_000, 13_510_000]]];
        let blocks = vec![StoryBlock::new(Block::Route { caption: "Walk <1>".into(), source: String::new(), route })];
        let image = WebImage { file: "images/r.jpg".into(), width: 1440, height: 800 };
        let maps = HashMap::from([(blocks[0].uid.clone(), RouteImage { image, credit: "© OpenStreetMap contributors" })]);
        let (html, _) = page(&story(), None, &blocks, &HashMap::new(), &maps, &words());
        assert!(html.contains("<img src=\"images/r.jpg\" width=\"1440\" height=\"800\" alt=\"Walk &lt;1&gt;\""));
        assert!(html.contains("<small class=\"credit\">© OpenStreetMap contributors</small>"));
        let (drawn, _) = page(&story(), None, &blocks, &HashMap::new(), &HashMap::new(), &words());
        assert!(drawn.contains("<svg") && !drawn.contains("<img"), "without its map, the route is drawn on its own");
        let (text, _) = markdown(&story(), None, &blocks, &HashMap::new(), &maps, &words());
        assert!(text.contains("![Walk <1>](images/r.jpg)") || text.contains("](images/r.jpg)"));
    }

    #[test]
    fn text_is_escaped_and_paragraphs_kept() {
        let blocks = [
            StoryBlock::new(Block::Heading("Day 1 & 2".into())),
            StoryBlock::new(Block::Paragraph("First \"line\"\nsecond\n\n<script>x</script>".into())),
        ];
        let (html, _) = page(&story(), Some("1 – 3 March"), &blocks, &HashMap::new(), &HashMap::new(), &words());
        assert!(html.contains("<title>Kyoto &lt;in&gt; spring</title>"));
        assert!(html.contains("<h2 id=\"section-1\">Day 1 &amp; 2</h2>"));
        assert!(html.contains("<p>First &quot;line&quot;<br>\nsecond</p>"));
        assert!(html.contains("<p>&lt;script&gt;x&lt;/script&gt;</p>"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("<p class=\"byline\">Ana · 1 – 3 March</p>"));
    }

    #[test]
    fn the_introduction_opens_and_the_conclusion_closes_the_story() {
        let mut story = story();
        story.introduction = "Why we went".into();
        story.conclusion = "We'll be back".into();
        let blocks = [StoryBlock::new(Block::Heading("Day 1".into()))];
        let (html, _) = page(&story, None, &blocks, &HashMap::new(), &HashMap::new(), &words());
        let at = |text: &str| html.find(text).unwrap();
        assert!(at("</header>") < at("Why we went") && at("Why we went") < at("<h2 id=\"section-1\">Day 1</h2>") && at("<h2 id=\"section-1\">Day 1</h2>") < at("We&#39;ll be back"));
        let (text, _) = markdown(&story, None, &blocks, &HashMap::new(), &HashMap::new(), &words());
        assert!(text.find("Why we went").unwrap() < text.find("## Day 1").unwrap() && text.find("## Day 1").unwrap() < text.find("We'll be back").unwrap());
        let (plain, _) = page(&self::story(), None, &blocks, &HashMap::new(), &HashMap::new(), &words());
        assert!(!plain.contains("<section"), "left empty, neither is written");
    }

    #[test]
    fn missing_photos_are_shown_and_counted_never_left_out() {
        let mut shown = MediaUse::new(1);
        shown.alt = "A \"red\" gate".into();
        shown.caption = "Fushimi".into();
        let blocks = [
            StoryBlock::new(Block::Photo(shown)),
            StoryBlock::new(Block::Gallery(vec![MediaUse::new(2), MediaUse::new(1)])),
            StoryBlock::new(Block::Unknown { kind: "map".into(), version: 9, payload: "{}".into(), media: Vec::new() }),
        ];
        let images = HashMap::from([(1, WebImage { file: "images/ab.jpg".into(), width: 1600, height: 1067 })]);
        let (html, outcome) = page(&story(), None, &blocks, &images, &HashMap::new(), &words());
        assert_eq!(outcome, Outcome { missing: 1, unknown: 1 });
        assert!(html.contains("<img src=\"images/ab.jpg\" width=\"1600\" height=\"1067\" alt=\"A &quot;red&quot; gate\""));
        assert!(html.contains("<figcaption>Fushimi</figcaption>"));
        assert!(html.contains("<div class=\"missing\">Photo unavailable</div>"));
        assert!(html.contains("<p class=\"byline\">Ana</p>"), "the author alone without dates");
    }

    #[test]
    fn a_page_is_written_with_rendered_photos_beside_it() {
        let folder = std::env::temp_dir().join(format!("picasso-story-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let photo = Path::new(env!("CARGO_MANIFEST_DIR")).join("images/banner.jpg");
        let sources = HashMap::from([(1, Some((photo, Settings::default(), 0))), (2, None)]);
        let blocks = [StoryBlock::new(Block::Photo(MediaUse::new(1))), StoryBlock::new(Block::Photo(MediaUse::new(2)))];
        let (story, words) = (story(), words());
        let page = Page { story: &story, dates: None, blocks: &blocks, sources: &sources, words: &words, map: None, profiles: &HashMap::new() };
        let check = preflight(&page);
        assert_eq!(check, Preflight { missing: 1, no_alt: 1, no_cover: true, first: [Some(1), Some(0), None, None, None], photos: 1, ..Preflight::default() });
        assert!(check.blocked(false) && !check.blocked(true), "missing photos wait for the user");
        let (index, outcome) = write_story(&folder, Format::Html, Profile::default(), &page, &Progress::default()).unwrap();
        assert_eq!(outcome.missing, 1);
        let html = std::fs::read_to_string(&index).unwrap();
        let file = html.split("src=\"").nth(1).unwrap().split('"').next().unwrap();
        assert!(file.starts_with("images/") && folder.join(file).is_file(), "{file}");
        let jpeg = std::fs::read(folder.join(file)).unwrap();
        assert!(!jpeg.windows(4).any(|w| w == b"Exif"), "no camera metadata in the page's photos");
        let text = std::fs::read_to_string(folder.join("manifest.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!((manifest["story"].as_i64(), manifest["revision"].as_i64()), (Some(1), Some(1)));
        assert_eq!(manifest["profile"]["version"].as_u64(), Some(u64::from(Profile::VERSION)));
        assert_eq!(manifest["photos"].as_array().unwrap().len(), 1, "only photos that were rendered");
        assert_eq!(manifest["photos"][0]["file"].as_str(), Some(file));
        assert!(!text.contains("banner"), "no source paths or names");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_cropped_cover_is_a_picture_of_its_own_and_the_photo_stays_whole() {
        let folder = std::env::temp_dir().join(format!("picasso-story-cover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let photo = Path::new(env!("CARGO_MANIFEST_DIR")).join("images/banner.jpg");
        let sources = HashMap::from([(1, Some((photo, Settings::default(), 0)))]);
        let blocks = [StoryBlock::new(Block::Photo(MediaUse::new(1)))];
        let mut story = story();
        story.cover = Some(1);
        story.cover_crop = crate::trip::CoverCrop([2_500, 0, 7_500, 5_000]);
        assert_eq!(story.cover_key(), Some(-1));
        let words = words();
        let page = Page { story: &story, dates: None, blocks: &blocks, sources: &sources, words: &words, map: None, profiles: &HashMap::new() };
        let (index, outcome) = write_story(&folder, Format::Html, Profile::default(), &page, &Progress::default()).unwrap();
        assert_eq!(outcome.missing, 0);
        let html = std::fs::read_to_string(&index).unwrap();
        let size = |class: &str| {
            let at = html.find(&format!("<figure class=\"{class}\">")).unwrap();
            let tag = &html[at..];
            let number = |name: &str| tag.split(&format!("{name}=\"")).nth(1).unwrap().split('"').next().unwrap().parse::<u32>().unwrap();
            (number("width"), number("height"))
        };
        let (cover, whole) = (size("cover"), size("photo"));
        assert_eq!(cover.0, whole.0 / 2, "half the width");
        assert_eq!(cover.1, whole.1 / 2, "half the height");
        let text = std::fs::read_to_string(folder.join("manifest.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(manifest["photos"].as_array().unwrap().len(), 1, "the source is listed once");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn unreadable_photos_wait_for_the_user_like_missing_ones() {
        let folder = std::env::temp_dir().join(format!("picasso-story-bad-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let bad = folder.join("bad.jpg");
        std::fs::write(&bad, b"not a photo").unwrap();
        let mut described = MediaUse::new(1);
        described.alt = "Something".into();
        let sources = HashMap::from([(1, Some((bad, Settings::default(), 0)))]);
        let blocks = [StoryBlock::new(Block::Heading("Day 1".into())), StoryBlock::new(Block::Photo(described))];
        let (story, words) = (story(), words());
        let page = Page { story: &story, dates: None, blocks: &blocks, sources: &sources, words: &words, map: None, profiles: &HashMap::new() };
        let check = preflight(&page);
        assert_eq!((check.unreadable, check.missing, check.photos, check.first[3]), (1, 0, 0, Some(1)));
        assert!(check.blocked(false) && !check.blocked(true));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_size_estimate_grows_with_photos_size_and_quality() {
        let small = Profile { edge: 1200, quality: 60 };
        let large = Profile { edge: 2400, quality: 95 };
        assert!(estimated_bytes(0, small) > 0, "the page itself");
        assert!(estimated_bytes(10, small) < estimated_bytes(20, small));
        assert!(estimated_bytes(10, small) < estimated_bytes(10, large));
        // A 1600 px JPEG at 85 is typically 300–500 KB; the estimate errs high.
        let one = estimated_bytes(1, Profile::default()) - estimated_bytes(0, Profile::default());
        assert!((400_000..1_000_000).contains(&one), "{one}");
        #[cfg(unix)]
        assert!(free_space(&std::env::temp_dir()).is_some_and(|free| free > 0));
        assert_eq!(free_space(Path::new("/no/such/folder/anywhere")), None);
    }

    #[test]
    fn a_grid_keeps_its_columns_and_one_description() {
        let mut first = MediaUse::new(1);
        first.alt = "A boat".into();
        first.caption = "hidden in a grid".into();
        let grid = Block::Grid { columns: 2, rows: 2, caption: "Morning <market>".into(), media: vec![first, MediaUse::new(2), MediaUse::new(3)] };
        let blocks = [StoryBlock::new(grid)];
        let images = HashMap::from([
            (1, WebImage { file: "images/a.jpg".into(), width: 10, height: 10 }),
            (2, WebImage { file: "images/b.jpg".into(), width: 10, height: 10 }),
        ]);
        let (html, outcome) = page(&story(), None, &blocks, &images, &HashMap::new(), &words());
        assert_eq!(outcome.missing, 1, "the third photo is shown as unavailable, not left out");
        assert!(html.contains("<figure class=\"grid\" style=\"--cols:2;--ratio:4/3\">"), "{html}");
        assert_eq!(html.matches("<img src=\"images/").count(), 2);
        assert!(html.contains("alt=\"A boat\""));
        assert!(html.contains("<figcaption>Morning &lt;market&gt;</figcaption>"));
        assert!(!html.contains("hidden in a grid"));
        let (text, _) = markdown(&story(), None, &blocks, &images, &HashMap::new(), &words());
        assert!(text.contains("![A boat](images/a.jpg)\n\n![](images/b.jpg)"), "{text}");
        assert!(text.contains("*Morning &lt;market&gt;*") && !text.contains("hidden"), "{text}");
    }

    #[test]
    fn markdown_escapes_its_marks_and_keeps_photos_in_place() {
        let mut photo = MediaUse::new(1);
        photo.alt = "gate [red]".into();
        photo.caption = "*Fushimi*".into();
        let blocks = [
            StoryBlock::new(Block::Heading("# Day 1".into())),
            StoryBlock::new(Block::Paragraph("- not a list\n1. not a number <b>".into())),
            StoryBlock::new(Block::Photo(photo)),
            StoryBlock::new(Block::Photo(MediaUse::new(2))),
            StoryBlock::new(Block::Separator),
        ];
        let images = HashMap::from([(1, WebImage { file: "images/ab.jpg".into(), width: 10, height: 10 })]);
        let (text, outcome) = markdown(&story(), None, &blocks, &images, &HashMap::new(), &words());
        assert_eq!(outcome.missing, 1);
        assert!(text.starts_with("# Kyoto &lt;in&gt; spring\n\nAna\n\n"), "{text}");
        assert!(text.contains("## \\# Day 1\n"));
        assert!(text.contains("\\- not a list  \n1\\. not a number &lt;b&gt;\n"), "{text}");
        assert!(text.contains("![gate \\[red\\]](images/ab.jpg)\n*\\*Fushimi\\**\n"), "{text}");
        assert!(text.contains("*Photo unavailable*\n"));
        assert!(text.contains("---\n"));
    }

    #[test]
    fn export_makes_a_new_folder_and_never_replaces_one() {
        let parent = std::env::temp_dir().join(format!("picasso-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(parent.join("Kyoto in spring")).unwrap();
        std::fs::write(parent.join("Kyoto in spring/mine.txt"), "keep").unwrap();
        let (story, words, sources) = (story(), words(), HashMap::new());
        let page = Page { story: &story, dates: None, blocks: &[], sources: &sources, words: &words, map: None, profiles: &HashMap::new() };
        let export = |format| export_story(&parent, "Kyoto: in/spring", format, Profile::default(), &page, &Progress::default());
        let (first, _) = export(Format::Html).unwrap();
        assert_eq!(first, parent.join("Kyoto in spring 2"));
        assert!(first.join("index.html").is_file());
        let (second, _) = export(Format::Markdown).unwrap();
        assert_eq!(second, parent.join("Kyoto in spring 3"));
        assert!(second.join("story.md").is_file());
        assert_eq!(std::fs::read_to_string(parent.join("Kyoto in spring/mine.txt")).unwrap(), "keep");
        let names: Vec<_> = std::fs::read_dir(&parent).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(names.len(), 3, "no staging folder is left: {names:?}");
        let stopped = Progress::default();
        stopped.cancel.store(true, Ordering::Relaxed);
        let photo = HashMap::from([(1, None)]);
        let page = Page { story: &story, dates: None, blocks: &[], sources: &photo, words: &words, map: None, profiles: &HashMap::new() };
        assert_eq!(export_story(&parent, "Stopped", Format::Html, Profile::default(), &page, &stopped), Err(CANCELLED.into()));
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 3, "a cancelled export leaves nothing");
        assert_eq!(folder_name(" ..hidden "), "hidden");
        assert_eq!(folder_name("/"), "Story");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn the_page_needs_nothing_from_the_network() {
        let (html, _) = page(&story(), None, &[], &HashMap::new(), &HashMap::new(), &words());
        assert!(!html.contains("http"), "no remote fonts, styles, or scripts");
        assert_eq!(key(b"same"), key(b"same"));
        assert_ne!(key(b"same"), key(b"other"));
    }
}
