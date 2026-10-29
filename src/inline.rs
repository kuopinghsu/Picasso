//! Styles inside a line of Story text, written as light Markdown marks:
//! `**bold**`, `*italic*`, `~~struck~~`, `` `code` ``, and `[text](url)`.
//! A backslash keeps the next mark a plain character. Marks without their
//! closing half are plain text, so nothing typed is ever lost
//! (`docs/TRAVEL.md`).
//!
//! The text is cut into spans that together are the whole line, marks
//! included, so the editor can draw it styled where the caret is; the
//! page, the Markdown file, and WordPress each get it from the spans.

/// Characters a backslash keeps plain.
const SPECIAL: &[u8] = b"\\*~`[]()";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// The address of the link the text is in.
    pub link: Option<String>,
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
}

impl Mark {
    fn marks(self) -> (&'static str, &'static str) {
        match self {
            Mark::Bold => ("**", "**"),
            Mark::Italic => ("*", "*"),
            Mark::Strike => ("~~", "~~"),
            Mark::Code => ("`", "`"),
            Mark::Link => ("[", "](https://)"),
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

/// The line as HTML, escaped, with its styles as `strong`, `em`, `s`,
/// `code`, and `a`.
pub fn to_html(line: &str) -> String {
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
        for tag in &tags {
            out.push_str(&format!("<{tag}>"));
        }
        out.push_str(&crate::story_html::esc(span.text));
        for tag in tags.iter().rev() {
            out.push_str(&format!("</{tag}>"));
        }
        if link.is_some() {
            out.push_str("</a>");
        }
    }
    out
}

/// The line as Markdown: the marks kept, the text between them escaped.
pub fn to_markdown(line: &str) -> String {
    let spans = spans(line);
    let mut out = String::new();
    for span in &spans {
        match (span.mark, span.style.code) {
            // A backslash's character is escaped again below.
            (true, _) if span.text == "\\" => {}
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
    fn the_spans_are_the_whole_line_so_the_editor_keeps_every_character() {
        for line in ["a **b** *c", "x [y](https://z) `q` \\* ~~", "日本 **語**", "**", "[a](b c)"] {
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
