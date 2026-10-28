//! A Story as a web page: one self-contained HTML file with its styles
//! inline and its photos beside it, used for the preview and for export.
//! Photos are rendered through the edit pipeline; originals are never copied.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ring::digest;

use crate::edit::Settings;
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
}

/// Where a Story photo comes from: its file, edit, and catalog rotation,
/// or `None` while it is unavailable.
pub type Source = Option<(PathBuf, Settings, i32)>;

/// Everything a page of a Story is made from.
pub struct Page<'a> {
    pub story: &'a Story,
    /// The Trip's dates as shown, if wanted.
    pub dates: Option<&'a str>,
    pub blocks: &'a [StoryBlock],
    pub sources: &'a HashMap<i64, Source>,
    pub words: &'a Words,
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
    /// The first block with each of those, to show it: missing, no
    /// description, newer, unreadable.
    pub first: [Option<usize>; 4],
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
    let mut check = Preflight { empty: page.blocks.is_empty(), ..Preflight::default() };
    // Some(true) readable, Some(false) there but unreadable, None missing.
    let state: HashMap<i64, Option<bool>> = page
        .sources
        .iter()
        .map(|(&media_id, source)| {
            let state = match source {
                Some((path, _, _)) if path.is_file() => Some(crate::image_io::probe(path, 0).is_ok()),
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
            } else if media.alt.trim().is_empty() {
                check.no_alt += 1;
                check.first[1].get_or_insert(index);
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
    let Page { story, dates, blocks, sources, words } = *input;
    let images_dir = folder.join("images");
    std::fs::create_dir_all(&images_dir).map_err(|err| err.to_string())?;
    let mut images = HashMap::new();
    let mut order: Vec<_> = sources.iter().collect();
    order.sort_by_key(|(id, _)| **id);
    progress.total.store(order.len(), Ordering::Relaxed);
    let mut read = Vec::new();
    for (&media_id, source) in order {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        progress.done.fetch_add(1, Ordering::Relaxed);
        let Some((path, settings, rotation)) = source else { continue };
        let before = stamp(path);
        // A photo that can't be read is shown as unavailable, never left out.
        let Ok(rgba) = crate::edit::render_edited(path, settings, *rotation, Some(profile.edge)) else { continue };
        let bytes = crate::image_io::encode_jpeg(&rgba, profile.quality.clamp(1, 100))?;
        let file = format!("images/{}.jpg", key(&bytes));
        crate::image_io::write_whole(&folder.join(&file), &bytes)?;
        read.push((media_id, path, before));
        images.insert(media_id, WebImage { file, width: rgba.width(), height: rgba.height() });
    }
    // The page is what the files were when read: one replaced or changed
    // meanwhile stops it, rather than mixing two versions.
    if read.iter().any(|(_, path, before)| stamp(path) != *before) {
        return Err(SOURCE_CHANGED.into());
    }
    let (text, outcome, name) = match format {
        Format::Html => {
            let (html, outcome) = page(story, dates, blocks, &images, words);
            (html, outcome, "index.html")
        }
        Format::Markdown => {
            let (markdown, outcome) = markdown(story, dates, blocks, &images, words);
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
pub fn markdown(story: &Story, dates: Option<&str>, blocks: &[StoryBlock], images: &HashMap<i64, WebImage>, words: &Words) -> (String, Outcome) {
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
    if let Some(cover) = story.cover {
        out.push_str(&md_image(&MediaUse::new(cover), images, words, &mut outcome));
    }
    for block in blocks {
        match &block.block {
            Block::Heading(text) => {
                let _ = write!(out, "## {}\n\n", md(text.trim()));
            }
            Block::Paragraph(text) | Block::Credits(text) => {
                for part in text.split("\n\n").map(str::trim).filter(|part| !part.is_empty()) {
                    let lines: Vec<String> = part.lines().map(|line| md(line.trim_end())).collect();
                    let _ = write!(out, "{}\n\n", lines.join("  \n"));
                }
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
            Block::Unknown { .. } => outcome.unknown += 1,
        }
    }
    (out, outcome)
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
pub fn page(story: &Story, dates: Option<&str>, blocks: &[StoryBlock], images: &HashMap<i64, WebImage>, words: &Words) -> (String, Outcome) {
    let mut outcome = Outcome::default();
    let title = if story.title.trim().is_empty() { words.untitled.as_str() } else { story.title.trim() };
    let mut html = String::new();
    let _ = write!(
        html,
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<article>\n<header>\n",
        esc(words.lang),
        esc(title)
    );
    if let Some(cover) = story.cover {
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
    for block in blocks {
        match &block.block {
            Block::Heading(text) => {
                let _ = writeln!(html, "<h2>{}</h2>", esc(text.trim()));
            }
            Block::Paragraph(text) => html.push_str(&paragraphs(text)),
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
            Block::Credits(text) => {
                let _ = write!(html, "<footer class=\"credits\">\n{}</footer>\n", paragraphs(text));
            }
            Block::Unknown { .. } => outcome.unknown += 1,
        }
    }
    html.push_str("</article>\n</body>\n</html>\n");
    (html, outcome)
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

/// Text as paragraphs: a blank line starts a new one, a line break stays.
fn paragraphs(text: &str) -> String {
    let mut html = String::new();
    for part in text.split("\n\n").map(str::trim).filter(|part| !part.is_empty()) {
        let lines: Vec<String> = part.lines().map(esc).collect();
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

/// Photography Journal: one wide column, large photos, quiet type. System
/// fonts only, so the page needs no network.
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
.credits{margin-top:3rem;padding-top:1.5rem;border-top:1px solid var(--rule);font-size:.9rem;color:var(--muted)}
@media (min-width:52rem){figure.photo,figure.cover,figure.grid{margin-left:-4rem;margin-right:-4rem}}
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
            author: "Ana".into(),
            show_date: true,
            cover: None,
            template: crate::trip::DEFAULT_TEMPLATE.into(),
            archived: false,
            finished: false,
            revision: 1,
            updated_at: 0,
        }
    }

    fn words() -> Words {
        Words { lang: "en", untitled: "Untitled story".into(), unavailable: "Photo unavailable".into() }
    }

    #[test]
    fn text_is_escaped_and_paragraphs_kept() {
        let blocks = [
            StoryBlock::new(Block::Heading("Day 1 & 2".into())),
            StoryBlock::new(Block::Paragraph("First \"line\"\nsecond\n\n<script>x</script>".into())),
        ];
        let (html, _) = page(&story(), Some("1 – 3 March"), &blocks, &HashMap::new(), &words());
        assert!(html.contains("<title>Kyoto &lt;in&gt; spring</title>"));
        assert!(html.contains("<h2>Day 1 &amp; 2</h2>"));
        assert!(html.contains("<p>First &quot;line&quot;<br>\nsecond</p>"));
        assert!(html.contains("<p>&lt;script&gt;x&lt;/script&gt;</p>"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("<p class=\"byline\">Ana · 1 – 3 March</p>"));
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
        let (html, outcome) = page(&story(), None, &blocks, &images, &words());
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
        let page = Page { story: &story, dates: None, blocks: &blocks, sources: &sources, words: &words };
        let check = preflight(&page);
        assert_eq!(check, Preflight { missing: 1, no_alt: 1, first: [Some(1), Some(0), None, None], photos: 1, ..Preflight::default() });
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
        let page = Page { story: &story, dates: None, blocks: &blocks, sources: &sources, words: &words };
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
        let (html, outcome) = page(&story(), None, &blocks, &images, &words());
        assert_eq!(outcome.missing, 1, "the third photo is shown as unavailable, not left out");
        assert!(html.contains("<figure class=\"grid\" style=\"--cols:2;--ratio:4/3\">"), "{html}");
        assert_eq!(html.matches("<img src=\"images/").count(), 2);
        assert!(html.contains("alt=\"A boat\""));
        assert!(html.contains("<figcaption>Morning &lt;market&gt;</figcaption>"));
        assert!(!html.contains("hidden in a grid"));
        let (text, _) = markdown(&story(), None, &blocks, &images, &words());
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
        let (text, outcome) = markdown(&story(), None, &blocks, &images, &words());
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
        let page = Page { story: &story, dates: None, blocks: &[], sources: &sources, words: &words };
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
        let page = Page { story: &story, dates: None, blocks: &[], sources: &photo, words: &words };
        assert_eq!(export_story(&parent, "Stopped", Format::Html, Profile::default(), &page, &stopped), Err(CANCELLED.into()));
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 3, "a cancelled export leaves nothing");
        assert_eq!(folder_name(" ..hidden "), "hidden");
        assert_eq!(folder_name("/"), "Story");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn the_page_needs_nothing_from_the_network() {
        let (html, _) = page(&story(), None, &[], &HashMap::new(), &words());
        assert!(!html.contains("http"), "no remote fonts, styles, or scripts");
        assert_eq!(key(b"same"), key(b"same"));
        assert_ne!(key(b"same"), key(b"other"));
    }
}
