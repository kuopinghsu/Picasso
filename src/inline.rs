//! Styles inside a line of Story text, written as light Markdown marks:
//! `**bold**`, `*italic*`, `~~struck~~`, `` `code` ``, `[text](url)`, and
//! a colour as `{red}text{/red}`. A backslash keeps the next mark a plain
//! character. Marks without their closing half are plain text, so nothing
//! typed is ever lost (`docs/TRAVEL.md`).
//!
//! The text is cut into spans that together are the whole line, marks
//! included, so the editor can draw it styled where the caret is; the
//! page, the Markdown file, and WordPress each get it from the spans.

/// Characters a backslash keeps plain.
const SPECIAL: &[u8] = b"\\*~`[](){";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// The address of the link the text is in.
    pub link: Option<String>,
    pub color: Option<Color>,
}

/// A colour for text: one of a few, each with a shade for light pages and
/// a lighter one for dark pages, so it reads on both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Red,
    Orange,
    Green,
    Blue,
    Purple,
    Gray,
}

impl Color {
    pub const ALL: [Color; 6] = [Color::Red, Color::Orange, Color::Green, Color::Blue, Color::Purple, Color::Gray];

    /// Its name in the marks, and in the classes of the page and the post.
    pub fn name(self) -> &'static str {
        match self {
            Color::Red => "red",
            Color::Orange => "orange",
            Color::Green => "green",
            Color::Blue => "blue",
            Color::Purple => "purple",
            Color::Gray => "gray",
        }
    }

    fn open(self) -> &'static str {
        match self {
            Color::Red => "{red}",
            Color::Orange => "{orange}",
            Color::Green => "{green}",
            Color::Blue => "{blue}",
            Color::Purple => "{purple}",
            Color::Gray => "{gray}",
        }
    }

    fn close(self) -> &'static str {
        match self {
            Color::Red => "{/red}",
            Color::Orange => "{/orange}",
            Color::Green => "{/green}",
            Color::Blue => "{/blue}",
            Color::Purple => "{/purple}",
            Color::Gray => "{/gray}",
        }
    }

    /// As `#rrggbb`: on a light page, or on a dark one. Each reads at 4.5:1
    /// or more on every template's page, the Travel Diary's paper included.
    pub fn hex(self, dark: bool) -> &'static str {
        match (self, dark) {
            (Color::Red, false) => "#c62828",
            (Color::Red, true) => "#ff8a80",
            (Color::Orange, false) => "#a14a08",
            (Color::Orange, true) => "#fdba74",
            (Color::Green, false) => "#27702b",
            (Color::Green, true) => "#86d993",
            (Color::Blue, false) => "#1d5fbf",
            (Color::Blue, true) => "#8ab4f8",
            (Color::Purple, false) => "#7e3fa8",
            (Color::Purple, true) => "#d0a2f0",
            (Color::Gray, false) => "#6b6b6b",
            (Color::Gray, true) => "#a3a3a3",
        }
    }
}

/// The colour mark `rest` starts with: its colour, its length, and
/// whether it is the closing one.
fn color_mark(rest: &str) -> Option<(Color, usize, bool)> {
    let inner = &rest[1..rest.find('}')?];
    let (closing, name) = match inner.strip_prefix('/') {
        Some(name) => (true, name),
        None => (false, inner),
    };
    let color = Color::ALL.into_iter().find(|color| color.name() == name)?;
    Some((color, inner.len() + 2, closing))
}

/// A piece of a line: text in one style, or a mark that styles it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span<'a> {
    pub text: &'a str,
    pub style: Style,
    pub mark: bool,
}

/// What the format buttons and shortcuts put around a selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    /// The colour to give the selection, or `None` to take its colour away.
    Color(Option<Color>),
}

impl Mark {
    fn marks(self) -> (&'static str, &'static str) {
        match self {
            Mark::Bold => ("**", "**"),
            Mark::Italic => ("*", "*"),
            Mark::Strike => ("~~", "~~"),
            Mark::Code => ("`", "`"),
            Mark::Link => ("[", "](https://)"),
            Mark::Color(Some(color)) => (color.open(), color.close()),
            Mark::Color(None) => ("", ""),
        }
    }
}

/// The line cut into spans, in order, covering all of it.
pub fn spans(line: &str) -> Vec<Span<'_>> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut style = Style::default();
    // Where the link's text ends, and where its address does.
    let mut link: Option<(usize, usize)> = None;
    let mut start = 0;
    let mut i = 0;
    fn push<'a>(out: &mut Vec<Span<'a>>, line: &'a str, from: usize, to: usize, style: &Style, mark: bool) {
        if to > from {
            out.push(Span { text: &line[from..to], style: style.clone(), mark });
        }
    }
    while i < bytes.len() {
        if let Some((close, after)) = link.filter(|&(close, _)| close == i) {
            push(&mut out, line, start, i, &style, false);
            push(&mut out, line, close, after, &style, true);
            style.link = None;
            link = None;
            i = after;
            start = i;
            continue;
        }
        let rest = &line[i..];
        let closes = |mark: &str| line[i + mark.len()..link.map_or(bytes.len(), |(close, _)| close)].contains(mark);
        let toggle = match bytes[i] {
            b'\\' if bytes.get(i + 1).is_some_and(|c| SPECIAL.contains(c)) => {
                push(&mut out, line, start, i, &style, false);
                push(&mut out, line, i, i + 1, &style, true);
                push(&mut out, line, i + 1, i + 2, &style, false);
                i += 2;
                start = i;
                continue;
            }
            b'`' => match rest[1..].find('`') {
                Some(end) => {
                    push(&mut out, line, start, i, &style, false);
                    push(&mut out, line, i, i + 1, &style, true);
                    let mut code = style.clone();
                    code.code = true;
                    push(&mut out, line, i + 1, i + 1 + end, &code, false);
                    push(&mut out, line, i + 1 + end, i + 2 + end, &style, true);
                    i += end + 2;
                    start = i;
                    continue;
                }
                None => None,
            },
            // One colour at a time: another opened inside it stays text.
            b'{' => match color_mark(rest) {
                Some((color, len, true)) if style.color == Some(color) => {
                    push(&mut out, line, start, i, &style, false);
                    push(&mut out, line, i, i + len, &style, true);
                    style.color = None;
                    i += len;
                    start = i;
                    continue;
                }
                Some((color, len, false)) if style.color.is_none() && line[i + len..link.map_or(bytes.len(), |(close, _)| close)].contains(color.close()) => {
                    push(&mut out, line, start, i, &style, false);
                    style.color = Some(color);
                    push(&mut out, line, i, i + len, &style, true);
                    i += len;
                    start = i;
                    continue;
                }
                _ => None,
            },
            b'*' if rest.starts_with("**") && (style.bold || closes("**")) => Some(("**", 0)),
            b'~' if rest.starts_with("~~") && (style.strike || closes("~~")) => Some(("~~", 1)),
            b'*' if !rest.starts_with("**") && (style.italic || closes("*")) => Some(("*", 2)),
            b'[' if link.is_none() => match link_at(line, i) {
                Some((close, after, address)) => {
                    push(&mut out, line, start, i, &style, false);
                    push(&mut out, line, i, i + 1, &style, true);
                    style.link = Some(address);
                    link = Some((close, after));
                    i += 1;
                    start = i;
                    continue;
                }
                None => None,
            },
            _ => None,
        };
        match toggle {
            Some((mark, which)) => {
                push(&mut out, line, start, i, &style, false);
                let opening = !match which {
                    0 => style.bold,
                    1 => style.strike,
                    _ => style.italic,
                };
                if !opening {
                    push(&mut out, line, i, i + mark.len(), &style, true);
                }
                match which {
                    0 => style.bold = opening,
                    1 => style.strike = opening,
                    _ => style.italic = opening,
                }
                if opening {
                    push(&mut out, line, i, i + mark.len(), &style, true);
                }
                i += mark.len();
                start = i;
            }
            None => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    push(&mut out, line, start, bytes.len(), &style, false);
    out
}

/// A link starting with the `[` at `at`: where its text ends, where the
/// whole link ends, and its address.
fn link_at(line: &str, at: usize) -> Option<(usize, usize, String)> {
    let close = at + line[at..].find("](")?;
    let open = close + 2;
    let end = open + line[open..].find(')')?;
    let address = &line[open..end];
    (!address.is_empty() && !address.contains(char::is_whitespace)).then(|| (close, end + 1, address.to_string()))
}

/// Only these addresses become links; any other stays text.
pub fn safe_link(address: &str) -> bool {
    let lower = address.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

/// The line without its marks.
pub fn plain(line: &str) -> String {
    spans(line).into_iter().filter(|span| !span.mark).map(|span| span.text).collect()
}

/// The line as HTML for the page, escaped, with its styles as `strong`,
/// `em`, `s`, `code`, and `a`, and a colour as a class the page's style
/// gives its shade for light and dark pages.
pub fn to_html(line: &str) -> String {
    html(line, &|color| (format!("<span class=\"color-{}\">", color.name()), "</span>"))
}

/// The line as HTML for a WordPress post: a colour as Gutenberg's own
/// inline colour, so the block editor shows and keeps it, with a class the
/// VoyageLight theme gives the lighter shade in dark mode.
pub fn to_post_html(line: &str) -> String {
    html(line, &|color| {
        let open = format!("<mark style=\"background-color:transparent;color:{}\" class=\"has-inline-color picasso-color-{}\">", color.hex(false), color.name());
        (open, "</mark>")
    })
}

fn html(line: &str, colored: &dyn Fn(Color) -> (String, &'static str)) -> String {
    let mut out = String::new();
    for span in spans(line).into_iter().filter(|span| !span.mark) {
        let style = &span.style;
        let link = style.link.as_deref().filter(|address| safe_link(address));
        let tags: Vec<&str> = [(style.bold, "strong"), (style.italic, "em"), (style.strike, "s"), (style.code, "code")]
            .into_iter()
            .filter_map(|(on, tag)| on.then_some(tag))
            .collect();
        if let Some(address) = link {
            out.push_str(&format!("<a href=\"{}\">", crate::story_html::esc(address)));
        }
        let color = style.color.map(colored);
        if let Some((open, _)) = &color {
            out.push_str(open);
        }
        for tag in &tags {
            out.push_str(&format!("<{tag}>"));
        }
        out.push_str(&crate::story_html::esc(span.text));
        for tag in tags.iter().rev() {
            out.push_str(&format!("</{tag}>"));
        }
        if let Some((_, close)) = color {
            out.push_str(close);
        }
        if link.is_some() {
            out.push_str("</a>");
        }
    }
    out
}

/// The line as Markdown: the marks kept, the text between them escaped,
/// and a colour, which Markdown has no mark for, as an HTML `span` in the
/// shade for light pages.
pub fn to_markdown(line: &str) -> String {
    let spans = spans(line);
    let mut out = String::new();
    for span in &spans {
        match (span.mark, span.style.code) {
            // A backslash's character is escaped again below.
            (true, _) if span.text == "\\" => {}
            (true, _) if span.text.starts_with("{/") => out.push_str("</span>"),
            (true, _) if span.text.starts_with('{') => {
                let color = span.style.color.map_or("inherit", |color| color.hex(false));
                out.push_str(&format!("<span style=\"color:{color}\">"));
            }
            (true, _) => out.push_str(span.text),
            (false, true) => out.push_str(span.text),
            (false, false) => out.push_str(&crate::story_html::md(span.text)),
        }
    }
    out
}

/// Puts `mark` around the characters `start..end` of `text`, or takes it
/// away when they already have it right around them. Returns the
/// characters to select afterwards: the same text, or for a link its
/// address, ready to be typed over.
pub fn wrap(text: &mut String, start: usize, end: usize, mark: Mark) -> (usize, usize) {
    let (start, end) = (start.min(end), start.max(end));
    if let Mark::Color(color) = mark {
        return recolor(text, start, end, color);
    }
    let byte = |text: &str, n: usize| text.char_indices().nth(n).map_or(text.len(), |(at, _)| at);
    let (open, close) = mark.marks();
    let (from, to) = (byte(text, start), byte(text, end));
    let open_chars = open.chars().count();
    if mark != Mark::Link && text[..from].ends_with(open) && text[to..].starts_with(close) {
        text.replace_range(to..to + close.len(), "");
        text.replace_range(from - open.len()..from, "");
        return (start - open_chars, end - open_chars);
    }
    text.insert_str(to, close);
    text.insert_str(from, open);
    match mark {
        // The address after the text, "https://" selected.
        Mark::Link => {
            let address = end + open_chars + 2;
            (address, address + "https://".len())
        }
        _ => (start + open_chars, end + open_chars),
    }
}

/// Gives the characters `start..end` of `text` colour `color`, or none.
/// A colour can't be inside another, so the one they had is taken away
/// from them first, and the text around them keeps its own, its marks
/// moved to end before them and start again after. Each line is coloured
/// on its own, as marks don't reach over lines. Returns the characters to
/// select afterwards: the same text.
fn recolor(text: &mut String, start: usize, end: usize, color: Option<Color>) -> (usize, usize) {
    let byte = |n: usize| text.char_indices().nth(n).map_or(text.len(), |(at, _)| at);
    // An end inside a colour's mark moves out to take all of the mark.
    let mut from = byte(start);
    if let (_, Some(mark)) = color_state(text, from) {
        from = mark.start;
    }
    let mut to = byte(end).max(from);
    if let (_, Some(mark)) = color_state(text, to) {
        to = mark.end;
    }
    let (first, last) = (color_state(text, from).0, color_state(text, to).0);
    let (mut before, mut after) = (text[..from].to_string(), text[to..].to_string());
    if let Some(color) = first {
        match before.strip_suffix(color.open()) {
            Some(rest) => before = rest.to_string(),
            None => before.push_str(color.close()),
        }
    }
    if let Some(color) = last {
        match after.strip_prefix(color.close()) {
            Some(rest) => after = rest.to_string(),
            None => after.insert_str(0, color.open()),
        }
    }
    let mut middle = String::new();
    let mut offset = text[..from].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[to..].find('\n').map_or(text.len(), |n| to + n);
    for line in text[offset..line_end].split_inclusive('\n') {
        for span in spans(line) {
            let at = offset;
            offset += span.text.len();
            let (left, right) = (from.max(at), to.min(offset));
            if left >= right || (span.mark && span.text.starts_with('{')) {
                continue;
            }
            let selected = &text[left..right];
            // A literal marker must remain literal under the new colour.
            // Code spans and existing escapes already keep their marks plain.
            if !span.mark && !span.style.code {
                for (i, ch) in selected.char_indices() {
                    if ch == '{' && color_mark(&selected[i..]).is_some() {
                        middle.push('\\');
                    }
                    middle.push(ch);
                }
            } else {
                middle.push_str(selected);
            }
        }
    }
    let colored = match color {
        Some(color) => middle.split('\n').map(|line| if line.is_empty() && middle.contains('\n') { String::new() } else { format!("{}{line}{}", color.open(), color.close()) }).collect::<Vec<_>>().join("\n"),
        None => middle.clone(),
    };
    let at = before.chars().count();
    *text = format!("{before}{colored}{after}");
    match color {
        Some(color) if !middle.contains('\n') => {
            let open = color.open().chars().count();
            (at + open, at + open + middle.chars().count())
        }
        _ => (at, at + colored.chars().count()),
    }
}

/// The colour text typed at byte `at` of `text` would have, and the bytes
/// of the colour mark `at` is inside of, if any.
fn color_state(text: &str, at: usize) -> (Option<Color>, Option<std::ops::Range<usize>>) {
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[at..].find('\n').map_or(text.len(), |n| at + n);
    let mut offset = line_start;
    let mut color = None;
    for span in spans(&text[line_start..line_end]) {
        let range = offset..offset + span.text.len();
        offset = range.end;
        let color_mark = span.mark && span.text.starts_with('{');
        if range.end <= at {
            color = if color_mark && span.text.starts_with("{/") { None } else { span.style.color };
            continue;
        }
        if range.start < at {
            return if color_mark { (color, Some(range)) } else { (span.style.color, None) };
        }
        break;
    }
    (color, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_style_the_text_between_them_and_lone_marks_stay_text() {
        assert_eq!(
            to_html("a **b** *c* ~~d~~ `*e*` [f](https://x.test/?a=1&b) 5 * 3"),
            "a <strong>b</strong> <em>c</em> <s>d</s> <code>*e*</code> <a href=\"https://x.test/?a=1&amp;b\">f</a> 5 * 3"
        );
        assert_eq!(to_html("**bold *both***"), "<strong>bold </strong><strong><em>both</em></strong>");
        assert_eq!(to_html(r"\*not\* <b>"), "*not* &lt;b&gt;");
        assert_eq!(to_html("[bad](javascript:alert(1))"), "bad)", "only web and mail addresses link");
        assert_eq!(plain("**Day** [one](https://x.test)"), "Day one");
    }

    #[test]
    fn a_colour_is_a_class_on_the_page_gutenbergs_own_in_a_post_and_a_span_in_markdown() {
        assert_eq!(
            to_html("a {red}b **c**{/red} {blue}d"),
            "a <span class=\"color-red\">b </span><span class=\"color-red\"><strong>c</strong></span> {blue}d",
            "a colour without its closing mark stays text"
        );
        assert_eq!(to_html("{red}a {blue}b{/blue}{/red}"), "<span class=\"color-red\">a {blue}b{/blue}</span>", "one colour at a time");
        assert_eq!(to_html(r"\{red}a{/red}"), "{red}a{/red}");
        assert_eq!(to_html("{pink}a{/pink}"), "{pink}a{/pink}", "only the colours there are");
        assert_eq!(
            to_post_html("{green}x{/green}"),
            "<mark style=\"background-color:transparent;color:#27702b\" class=\"has-inline-color picasso-color-green\">x</mark>"
        );
        assert_eq!(to_markdown("{red}**a_b**{/red}"), "<span style=\"color:#c62828\">**a\\_b**</span>");
        assert_eq!(plain("{gray}quiet{/gray}"), "quiet");
    }

    #[test]
    fn every_colour_reads_on_every_page_it_can_be_on() {
        fn luminance(hex: &str) -> f64 {
            let channel = |at: usize| {
                let c = f64::from(u8::from_str_radix(&hex[at..at + 2], 16).unwrap()) / 255.0;
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
        }
        // The pages of the templates and of the VoyageLight theme, light and dark.
        let light = ["#fbfaf7", "#f6efe1", "#ffffff", "#f6f5f2"];
        let dark = ["#161616", "#221d17", "#12161c", "#0e0e0e", "#0b141b", "#121f29"];
        for color in Color::ALL {
            for (shade, pages) in [(color.hex(false), &light[..]), (color.hex(true), &dark[..])] {
                for page in pages {
                    let (a, b) = (luminance(shade), luminance(page));
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(contrast >= 4.5, "{color:?} {shade} on {page}: {contrast:.2}:1");
                }
            }
        }
    }

    #[test]
    fn the_voyagelight_theme_gives_each_colour_its_dark_shade() {
        let css = include_str!("../wordpress/voyagelight/assets/css/theme.css");
        for color in Color::ALL {
            let rule = format!("html[data-theme=\"dark\"] .picasso-color-{}{{color:{}!important}}", color.name(), color.hex(true));
            assert!(css.contains(&rule), "the theme lacks {rule}");
        }
    }

    #[test]
    fn a_colour_replaces_the_one_the_selection_had_and_the_text_around_keeps_its_own() {
        let recolored = |text: &str, start: usize, end: usize, color: Option<Color>| {
            let mut text = text.to_string();
            let (from, to) = wrap(&mut text, start, end, Mark::Color(color));
            (text.clone(), text.chars().skip(from).take(to - from).collect::<String>())
        };
        assert_eq!(recolored("one two", 4, 7, Some(Color::Red)), ("one {red}two{/red}".into(), "two".into()));
        assert_eq!(recolored("one {red}two{/red}", 9, 12, Some(Color::Blue)), ("one {blue}two{/blue}".into(), "two".into()));
        assert_eq!(recolored("one {red}two{/red}", 9, 12, None), ("one two".into(), "two".into()));
        assert_eq!(recolored("{red}abc{/red}", 6, 7, Some(Color::Blue)), ("{red}a{/red}{blue}b{/blue}{red}c{/red}".into(), "b".into()), "a word in the middle");
        assert_eq!(recolored("{red}abc{/red}", 5, 5, None), ("{red}abc{/red}".into(), String::new()), "a caret alone changes nothing");
        assert_eq!(recolored("{red}abc{/red}", 0, 14, Some(Color::Green)).0, "{green}abc{/green}", "the marks selected too");
        assert_eq!(recolored("{red}abc{/red}", 2, 8, Some(Color::Green)).0, "{green}abc{/green}", "an end inside a mark takes all of it");
        assert_eq!(recolored("x {red}ab{/red}", 0, 8, None).0, "x a{red}b{/red}");
        assert_eq!(recolored("a\n\nb", 0, 4, Some(Color::Green)).0, "{green}a{/green}\n\n{green}b{/green}", "each line on its own");
        for original in ["The {red} label", "The {/blue} label", r"The \{red} label", "The `{red}` label", "{red}{blue}label{/blue}{/red}"] {
            let visible = |text: &str| spans(text).into_iter().filter(|span| !span.mark).map(|span| span.text).collect::<String>();
            for color in [None, Some(Color::Blue)] {
                let (out, _) = recolored(original, 0, original.len(), color);
                assert_eq!(visible(&out), visible(original), "{original} became {out}");
            }
        }
        for text in ["one {red}two{/red}", "{red}abc{/red}", "x {red}ab{/red}"] {
            for start in 0..=text.len() {
                for end in start..=text.len() {
                    for color in [None, Some(Color::Blue)] {
                        let (out, _) = recolored(text, start, end, color);
                        let left = spans(&out).iter().filter(|span| !span.mark).map(|span| span.text).collect::<String>();
                        assert!(!left.contains('{'), "{text:?} {start}..{end} {color:?} left a mark as text: {out:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_spans_are_the_whole_line_so_the_editor_keeps_every_character() {
        for line in ["a **b** *c", "x [y](https://z) `q` \\* ~~", "日本 **語**", "**", "[a](b c)", "{red}x{/red} {blue}", "{/red}{red}"] {
            assert_eq!(spans(line).iter().map(|span| span.text).collect::<String>(), line);
        }
    }

    #[test]
    fn markdown_keeps_the_marks_and_escapes_the_rest() {
        assert_eq!(to_markdown("**a_b** 5 * 3"), "**a\\_b** 5 \\* 3");
        assert_eq!(to_markdown(r"\*x"), "\\*x");
    }

    #[test]
    fn a_selection_is_wrapped_or_unwrapped() {
        let mut text = "one two".to_string();
        assert_eq!(wrap(&mut text, 4, 7, Mark::Bold), (6, 9));
        assert_eq!(text, "one **two**");
        assert_eq!(wrap(&mut text, 6, 9, Mark::Bold), (4, 7));
        assert_eq!(text, "one two");
        let (from, to) = wrap(&mut text, 0, 3, Mark::Link);
        assert_eq!(text, "[one](https://) two");
        assert_eq!(text.chars().skip(from).take(to - from).collect::<String>(), "https://");
    }
}
