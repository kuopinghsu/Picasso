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

/// How the page's photos are made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Longest side, in pixels; smaller photos are not enlarged.
    pub edge: u32,
    pub quality: u8,
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
    /// Photos without a description for people who can't see them.
    pub no_alt: usize,
    /// Blocks from a newer Picasso, which can't be exported.
    pub unknown: usize,
}

impl Preflight {
    /// Whether export must wait for the user to decide.
    pub fn blocked(&self, skip_missing: bool) -> bool {
        self.empty || (self.missing > 0 && !skip_missing)
    }
}

/// Checks a Story before it is exported. Reads only whether files exist.
pub fn preflight(page: &Page) -> Preflight {
    let mut check = Preflight { empty: page.blocks.is_empty(), ..Preflight::default() };
    let available = |media_id: i64| matches!(page.sources.get(&media_id), Some(Some((path, _, _))) if path.is_file());
    if page.story.cover.is_some_and(|cover| !available(cover)) {
        check.missing += 1;
    }
    for block in page.blocks {
        if matches!(block.block, Block::Unknown { .. }) {
            check.unknown += 1;
            continue;
        }
        for media in block.block.media() {
            if !available(media.media_id) {
                check.missing += 1;
            } else if media.alt.trim().is_empty() {
                check.no_alt += 1;
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
    for (&media_id, source) in order {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        progress.done.fetch_add(1, Ordering::Relaxed);
        let Some((path, settings, rotation)) = source else { continue };
        // A photo that can't be read is shown as unavailable, never left out.
        let Ok(rgba) = crate::edit::render_edited(path, settings, *rotation, Some(profile.edge)) else { continue };
        let bytes = crate::image_io::encode_jpeg(&rgba, profile.quality.clamp(1, 100))?;
        let file = format!("images/{}.jpg", key(&bytes));
        crate::image_io::write_whole(&folder.join(&file), &bytes)?;
        images.insert(media_id, WebImage { file, width: rgba.width(), height: rgba.height() });
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
    Ok((index, outcome))
}

/// The reason [`write_story`] gives when it was cancelled.
pub const CANCELLED: &str = "cancelled";

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
.missing{aspect-ratio:3/2;display:flex;align-items:center;justify-content:center;background:var(--rule);color:var(--muted);border-radius:4px;font:.9rem system-ui,sans-serif}
hr{border:0;border-top:1px solid var(--rule);margin:3rem auto;width:40%}
.credits{margin-top:3rem;padding-top:1.5rem;border-top:1px solid var(--rule);font-size:.9rem;color:var(--muted)}
@media (min-width:52rem){figure.photo,figure.cover{margin-left:-4rem;margin-right:-4rem}}
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
        assert_eq!(check, Preflight { missing: 1, no_alt: 1, ..Preflight::default() });
        assert!(check.blocked(false) && !check.blocked(true), "missing photos wait for the user");
        let (index, outcome) = write_story(&folder, Format::Html, Profile::default(), &page, &Progress::default()).unwrap();
        assert_eq!(outcome.missing, 1);
        let html = std::fs::read_to_string(&index).unwrap();
        let file = html.split("src=\"").nth(1).unwrap().split('"').next().unwrap();
        assert!(file.starts_with("images/") && folder.join(file).is_file(), "{file}");
        let jpeg = std::fs::read(folder.join(file)).unwrap();
        assert!(!jpeg.windows(4).any(|w| w == b"Exif"), "no camera metadata in the page's photos");
        let _ = std::fs::remove_dir_all(&folder);
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
