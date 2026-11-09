//! A Story as WordPress block markup: the core blocks Gutenberg itself
//! saves, so a post reopens in the block editor as these blocks
//! (`docs/TRAVEL.md`).
//!
//! The Story stays free of anything from WordPress. The site's media IDs
//! and URLs come in through `media`, by media reference, at the time of
//! publishing; a photo without one, or a block with no core form, stops
//! the serialization rather than being left out.
//!
//! Publishing to a site uses it; until then only its tests do.
#![cfg_attr(not(test), allow(dead_code))]

use std::fmt::Write;

use serde_json::{json, Value};

use crate::inline;
use crate::trip::{Block, MediaUse, Story, StoryBlock};

/// A photo as uploaded to the site.
#[derive(Clone, Debug, PartialEq)]
pub struct Uploaded {
    pub id: i64,
    pub url: String,
}

/// Why a Story can't be written as blocks.
#[derive(Clone, Debug, PartialEq)]
pub enum Unsupported {
    /// Block `index` was made by a newer Picasso.
    Unknown(usize),
    /// The photo with this media reference has no upload.
    Photo(i64),
    /// The video with this media reference has no upload.
    Video(i64),
}

/// A video's part as uploaded to the site, and the picture shown before it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct UploadedVideo {
    pub video: Uploaded,
    pub poster: Uploaded,
}

/// A route drawn on its map and uploaded, with the credit its tiles ask for.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteMap {
    pub uploaded: Uploaded,
    pub credit: &'static str,
}

/// The post: its block markup, and the cover as the featured image, or
/// without a cover the Story's first photo, as themes open a post on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    pub title: String,
    /// The subtitle, as the post's excerpt.
    pub excerpt: String,
    pub content: String,
    pub featured: Option<i64>,
}

impl Post {
    /// Puts the Story dates at the start of the post in an editable core block.
    pub fn prepend_dates(&mut self, dates: &str) {
        let dates = esc(dates);
        self.content.insert_str(0, &format!("<!-- wp:paragraph {{\"className\":\"picasso-story-dates\"}} -->\n<p class=\"picasso-story-dates\">{dates}</p>\n<!-- /wp:paragraph -->\n\n"));
    }
}

/// The Story as a post. The introduction comes before the blocks and the
/// conclusion after them, as paragraphs; the title and the subtitle, as
/// its excerpt, are the post's own, and the byline is the site's author. `maps` gives each Route block's map picture by the
/// block's uid; a route without one is drawn on its own. `videos` gives
/// each Video block's part as uploaded, by the block's uid.
pub fn post(story: &Story, blocks: &[StoryBlock], media: &dyn Fn(i64) -> Option<Uploaded>, maps: &dyn Fn(&str) -> Option<RouteMap>, videos: &dyn Fn(&str) -> Option<UploadedVideo>) -> Result<Post, Unsupported> {
    let mut out = String::new();
    paragraphs(&mut out, &story.introduction, None);
    for (index, block) in blocks.iter().enumerate() {
        match &block.block {
            Block::Heading(text) => {
                if !text.trim().is_empty() {
                    let _ = write!(out, "<!-- wp:heading -->\n<h2 class=\"wp-block-heading\">{}</h2>\n<!-- /wp:heading -->\n\n", inline::to_post_html(text.trim()));
                }
            }
            Block::Subheading(text) => {
                if !text.trim().is_empty() {
                    let _ = write!(
                        out,
                        "<!-- wp:heading {{\"level\":3}} -->\n<h3 class=\"wp-block-heading\">{}</h3>\n<!-- /wp:heading -->\n\n",
                        inline::to_post_html(text.trim())
                    );
                }
            }
            Block::List { ordered, items } => {
                let items: Vec<String> = items
                    .iter()
                    .map(|item| item.trim())
                    .filter(|item| !item.is_empty())
                    .map(|item| format!("<!-- wp:list-item -->\n<li>{}</li>\n<!-- /wp:list-item -->", inline::to_post_html(item)))
                    .collect();
                if !items.is_empty() {
                    let (attrs, tag) = if *ordered { (" {\"ordered\":true}", "ol") } else { ("", "ul") };
                    let _ = write!(out, "<!-- wp:list{attrs} -->\n<{tag} class=\"wp-block-list\">{}</{tag}>\n<!-- /wp:list -->\n\n", items.join("\n\n"));
                }
            }
            Block::Quote { text, citation } => {
                let mut inner = String::new();
                paragraphs(&mut inner, text, None);
                let cite = if citation.trim().is_empty() { String::new() } else { format!("<cite>{}</cite>", inline::to_post_html(citation.trim())) };
                let _ = write!(out, "<!-- wp:quote -->\n<blockquote class=\"wp-block-quote\">{}{cite}</blockquote>\n<!-- /wp:quote -->\n\n", inner.trim_end());
            }
            Block::Info { title, items } => {
                let items: Vec<String> = items
                    .iter()
                    .map(|(label, value)| (label.trim(), value.trim()))
                    .filter(|(label, value)| !label.is_empty() || !value.is_empty())
                    .map(|(label, value)| {
                        let label = if label.is_empty() { String::new() } else { format!("<strong>{}:</strong> ", esc(label)) };
                        format!("<!-- wp:list-item -->\n<li>{label}{}</li>\n<!-- /wp:list-item -->", inline::to_post_html(value))
                    })
                    .collect();
                if !title.trim().is_empty() || !items.is_empty() {
                    out.push_str("<!-- wp:group {\"className\":\"picasso-info\",\"layout\":{\"type\":\"constrained\"}} -->\n<div class=\"wp-block-group picasso-info\">");
                    if !title.trim().is_empty() {
                        let _ = write!(out, "<!-- wp:heading {{\"level\":3}} -->\n<h3 class=\"wp-block-heading\">{}</h3>\n<!-- /wp:heading -->\n\n", esc(title.trim()));
                    }
                    if !items.is_empty() {
                        let _ = write!(out, "<!-- wp:list -->\n<ul class=\"wp-block-list\">{}</ul>\n<!-- /wp:list -->", items.join("\n\n"));
                    }
                    out.push_str("</div>\n<!-- /wp:group -->\n\n");
                }
            }
            Block::Link { url, title, note } => {
                let shown = if title.trim().is_empty() { url.trim() } else { title.trim() };
                if !shown.is_empty() || !note.trim().is_empty() {
                    let link = match crate::trip::web_address(url) {
                        Some(url) => format!("<a href=\"{}\" rel=\"noopener\">{}</a>", esc(url), esc(shown)),
                        None => esc(shown),
                    };
                    let note = if note.trim().is_empty() { String::new() } else { format!("<br>{}", inline::to_post_html(note.trim())) };
                    let _ = write!(out, "<!-- wp:paragraph {{\"className\":\"picasso-link\"}} -->\n<p class=\"picasso-link\">{link}{note}</p>\n<!-- /wp:paragraph -->\n\n");
                }
            }
            // WordPress asks YouTube for the player of the address on its
            // own line, so the post plays it as the site's theme sizes it.
            // That player starts at the beginning; the VoyageLight theme and
            // the Picasso Places plugin start it where the address says.
            Block::YouTube { url, caption } => {
                let caption = caption.trim();
                match crate::trip::youtube(url) {
                    Some(video) => {
                        let url = video.watch();
                        // Escaped as Gutenberg writes them; a video's id can hold "--", which would end the comment.
                        let attrs = json!({ "url": url, "type": "video", "providerNameSlug": "youtube", "responsive": true, "className": "wp-embed-aspect-16-9 wp-has-aspect-ratio" })
                            .to_string()
                            .replace("--", "\\u002d\\u002d")
                            .replace('&', "\\u0026");
                        let caption = if caption.is_empty() { String::new() } else { format!("<figcaption class=\"wp-element-caption\">{}</figcaption>", esc(caption)) };
                        let _ = write!(
                            out,
                            "<!-- wp:embed {attrs} -->\n<figure class=\"wp-block-embed is-type-video is-provider-youtube wp-block-embed-youtube wp-embed-aspect-16-9 wp-has-aspect-ratio\"><div class=\"wp-block-embed__wrapper\">\n{}\n</div>{caption}</figure>\n<!-- /wp:embed -->\n\n",
                            esc(&url)
                        );
                    }
                    None if !caption.is_empty() => paragraphs(&mut out, caption, None),
                    None => {}
                }
            }
            // The columns fit their text unless given widths. A core table's
            // markup has no place for those, so they go in an attribute of
            // their own, which the Picasso Places plugin and the VoyageLight
            // theme draw as a <colgroup>.
            Block::Table { header, rows, widths, caption } => {
                let (sections, widths) = crate::story_html::table_sections(*header, rows, widths, false, inline::to_post_html);
                if !sections.is_empty() {
                    let caption = if caption.trim().is_empty() { String::new() } else { format!("<figcaption class=\"wp-element-caption\">{}</figcaption>", esc(caption.trim())) };
                    let attrs = if widths.is_empty() { json!({ "hasFixedLayout": false }) } else { json!({ "hasFixedLayout": false, "picassoColumnWidths": widths }) };
                    let _ = write!(out, "<!-- wp:table {attrs} -->\n<figure class=\"wp-block-table\"><table>{sections}</table>{caption}</figure>\n<!-- /wp:table -->\n\n");
                }
            }
            Block::Paragraph(text) => paragraphs(&mut out, text, None),
            Block::Credits(text) => paragraphs(&mut out, text, Some("picasso-credits")),
            Block::Photo(item) => {
                let uploaded = media(item.media_id).ok_or(Unsupported::Photo(item.media_id))?;
                out.push_str(&image(item, &uploaded));
                out.push('\n');
            }
            // The core block has no place for a description, which only
            // the site's media library keeps.
            Block::Video { media: item, .. } => {
                let uploaded = videos(&block.uid).ok_or(Unsupported::Video(item.media_id))?;
                let caption = if item.caption.trim().is_empty() {
                    String::new()
                } else {
                    format!("<figcaption class=\"wp-element-caption\">{}</figcaption>", esc(item.caption.trim()))
                };
                let _ = write!(
                    out,
                    "<!-- wp:video {} -->\n<figure class=\"wp-block-video\"><video controls poster=\"{}\" src=\"{}\"></video>{caption}</figure>\n<!-- /wp:video -->\n\n",
                    json!({ "id": uploaded.video.id }),
                    esc(&uploaded.poster.url),
                    esc(&uploaded.video.url)
                );
            }
            Block::Gallery(items) => out.push_str(&gallery(items, None, "", media)?),
            Block::Grid { columns, caption, media: items, .. } => out.push_str(&gallery(items, Some(*columns), caption, media)?),
            Block::Separator => out.push_str("<!-- wp:separator -->\n<hr class=\"wp-block-separator has-alpha-channel-opacity\"/>\n<!-- /wp:separator -->\n\n"),
            Block::Route { caption, route, .. } if !route.is_empty() && maps(&block.uid).is_some() => {
                let Some(map) = maps(&block.uid) else { continue };
                let attrs: Value = json!({ "id": map.uploaded.id, "sizeSlug": "large", "linkDestination": "none", "className": "picasso-route" });
                let _ = write!(
                    out,
                    "<!-- wp:image {attrs} -->\n<figure class=\"wp-block-image size-large picasso-route\"><img src=\"{}\" alt=\"{}\" class=\"wp-image-{}\"/><figcaption class=\"wp-element-caption\">{}</figcaption></figure>\n<!-- /wp:image -->\n\n",
                    esc(&map.uploaded.url),
                    esc(caption.trim()),
                    map.uploaded.id,
                    crate::story_html::route_words(route, caption, map.credit)
                );
            }
            // Without its map, a drawing WordPress keeps as it is, coloured by the theme's text.
            Block::Route { caption, route, .. } => {
                let figure = crate::story_html::route_figure(route, caption);
                if !figure.is_empty() {
                    let styled = figure.replacen("<svg ", "<svg style=\"width:100%;height:auto\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"4\" ", 1);
                    let _ = write!(out, "<!-- wp:html -->\n{}<!-- /wp:html -->\n\n", styled);
                }
            }
            Block::Unknown { .. } => return Err(Unsupported::Unknown(index)),
        }
    }
    paragraphs(&mut out, &story.conclusion, None);
    let first_photo = || blocks.iter().filter(|block| !matches!(block.block, Block::Video { .. })).find_map(|block| block.block.media().first().map(|first| first.media_id));
    let featured = match story.cover_key().or_else(first_photo) {
        Some(cover) => Some(media(cover).ok_or(Unsupported::Photo(cover.abs()))?.id),
        None => None,
    };
    Ok(Post { title: story.title.trim().to_string(), excerpt: story.subtitle.trim().to_string(), content: out.trim_end().to_string(), featured })
}

/// The places of a post, for a theme without a map of its own: under
/// `heading`, the map picture with their numbered pins when there is one,
/// then each place by its number, linked to it on OpenStreetMap.
pub fn places(heading: &str, alt: &str, places: &[&crate::publish::MapPlace], map: Option<&RouteMap>) -> String {
    if places.is_empty() {
        return String::new();
    }
    let mut out = String::from("<!-- wp:group {\"className\":\"picasso-places\",\"layout\":{\"type\":\"constrained\"}} -->\n<div class=\"wp-block-group picasso-places\">");
    let _ = write!(out, "<!-- wp:heading -->\n<h2 class=\"wp-block-heading\">{}</h2>\n<!-- /wp:heading -->\n\n", esc(heading));
    if let Some(map) = map {
        let attrs: Value = json!({ "id": map.uploaded.id, "sizeSlug": "large", "linkDestination": "none", "className": "picasso-places-map" });
        let _ = write!(
            out,
            "<!-- wp:image {attrs} -->\n<figure class=\"wp-block-image size-large picasso-places-map\"><img src=\"{}\" alt=\"{}\" class=\"wp-image-{}\"/><figcaption class=\"wp-element-caption\"><small class=\"credit\">{}</small></figcaption></figure>\n<!-- /wp:image -->\n\n",
            esc(&map.uploaded.url),
            esc(alt),
            map.uploaded.id,
            esc(map.credit)
        );
    }
    let items: Vec<String> = places
        .iter()
        .map(|place| {
            let link = format!("https://www.openstreetmap.org/?mlat={0:.5}&mlon={1:.5}#map=15/{0:.5}/{1:.5}", place.lat, place.lon);
            format!("<!-- wp:list-item -->\n<li><a href=\"{}\">{}</a></li>\n<!-- /wp:list-item -->", esc(&link), esc(place.name.trim()))
        })
        .collect();
    let _ = write!(out, "<!-- wp:list {{\"ordered\":true}} -->\n<ol class=\"wp-block-list\">{}</ol>\n<!-- /wp:list -->", items.join("\n\n"));
    out.push_str("</div>\n<!-- /wp:group -->");
    out
}

/// Each part between blank lines as its own paragraph block, its lines
/// kept with line breaks.
fn paragraphs(out: &mut String, text: &str, class: Option<&str>) {
    for part in text.split("\n\n").map(str::trim).filter(|part| !part.is_empty()) {
        let lines: Vec<String> = part.lines().map(|line| inline::to_post_html(line.trim_end())).collect();
        match class {
            Some(class) => {
                let _ = write!(
                    out,
                    "<!-- wp:paragraph {} -->\n<p class=\"{class}\">{}</p>\n<!-- /wp:paragraph -->\n\n",
                    json!({ "className": class }),
                    lines.join("<br>")
                );
            }
            None => {
                let _ = write!(out, "<!-- wp:paragraph -->\n<p>{}</p>\n<!-- /wp:paragraph -->\n\n", lines.join("<br>"));
            }
        }
    }
}

fn image(item: &MediaUse, uploaded: &Uploaded) -> String {
    let attrs: Value = json!({ "id": uploaded.id, "sizeSlug": "large", "linkDestination": "none" });
    let caption = if item.caption.trim().is_empty() {
        String::new()
    } else {
        format!("<figcaption class=\"wp-element-caption\">{}</figcaption>", esc(item.caption.trim()))
    };
    format!(
        "<!-- wp:image {attrs} -->\n<figure class=\"wp-block-image size-large\"><img src=\"{}\" alt=\"{}\" class=\"wp-image-{}\"/>{caption}</figure>\n<!-- /wp:image -->\n",
        esc(&uploaded.url),
        esc(item.alt.trim()),
        uploaded.id
    )
}

/// A gallery of image blocks; a grid is a gallery of its columns, with its
/// description as the gallery's caption.
fn gallery(items: &[MediaUse], columns: Option<u8>, caption: &str, media: &dyn Fn(i64) -> Option<Uploaded>) -> Result<String, Unsupported> {
    let attrs = match columns {
        Some(columns) => json!({ "columns": columns, "linkTo": "none" }),
        None => json!({ "linkTo": "none" }),
    };
    let class = match columns {
        Some(columns) => format!("columns-{columns}"),
        None => "columns-default".into(),
    };
    let mut out = format!("<!-- wp:gallery {attrs} -->\n<figure class=\"wp-block-gallery has-nested-images {class} is-cropped\">");
    for item in items {
        let uploaded = media(item.media_id).ok_or(Unsupported::Photo(item.media_id))?;
        out.push_str(&image(item, &uploaded));
    }
    if !caption.trim().is_empty() {
        let _ = write!(out, "<figcaption class=\"blocks-gallery-caption wp-element-caption\">{}</figcaption>", esc(caption.trim()));
    }
    out.push_str("</figure>\n<!-- /wp:gallery -->\n\n");
    Ok(out)
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trip::DEFAULT_TEMPLATE;

    fn story() -> Story {
        Story {
            id: 1,
            trip_id: 1,
            title: "Kyoto".into(),
            subtitle: String::new(),
            introduction: "Why we went".into(),
            conclusion: "Back & again".into(),
            author: String::new(),
            show_date: false,
            dates: None,
            cover: Some(3),
            cover_crop: crate::trip::CoverCrop::default(),
            template: DEFAULT_TEMPLATE.into(),
            archived: false,
            finished: true,
            revision: 1,
            updated_at: 0,
        }
    }

    fn uploads(media_id: i64) -> Option<Uploaded> {
        (media_id < 10).then(|| Uploaded { id: media_id + 100, url: format!("https://example.test/{media_id}.jpg") })
    }

    #[test]
    fn every_block_kind_has_its_core_block_in_order() {
        let mut photo = MediaUse::new(1);
        photo.caption = "The gate".into();
        photo.alt = "A \"red\" gate".into();
        let blocks = vec![
            StoryBlock::new(Block::Heading("Day 1 <arrival>".into())),
            StoryBlock::new(Block::Paragraph("One\ntwo\n\nThree".into())),
            StoryBlock::new(Block::Photo(photo)),
            StoryBlock::new(Block::Gallery(vec![MediaUse::new(2), MediaUse::new(3)])),
            StoryBlock::new(Block::Grid { columns: 3, rows: 1, caption: "Temples".into(), media: vec![MediaUse::new(4), MediaUse::new(5)] }),
            StoryBlock::new(Block::Separator),
            StoryBlock::new(Block::Credits("Photos by Ana".into())),
            StoryBlock::new(Block::Subheading("Evening".into())),
            StoryBlock::new(Block::List { ordered: true, items: vec!["**Tea**".into(), String::new(), "[Map](https://x.test)".into()] }),
            StoryBlock::new(Block::Quote { text: "Go *slowly*".into(), citation: "Bashō".into() }),
        ];
        let post = post(&story(), &blocks, &uploads, &|_| None, &|_| None).unwrap();
        assert_eq!((post.title.as_str(), post.featured), ("Kyoto", Some(103)));
        let content = &post.content;
        let order = [
            "<p>Why we went</p>",
            "<h2 class=\"wp-block-heading\">Day 1 &lt;arrival&gt;</h2>",
            "<p>One<br>two</p>",
            "<p>Three</p>",
            "<!-- wp:image {\"id\":101,\"linkDestination\":\"none\",\"sizeSlug\":\"large\"} -->",
            "alt=\"A &quot;red&quot; gate\" class=\"wp-image-101\"/><figcaption class=\"wp-element-caption\">The gate</figcaption>",
            "<!-- wp:gallery {\"linkTo\":\"none\"} -->",
            "class=\"wp-block-gallery has-nested-images columns-default is-cropped\"",
            "<!-- wp:gallery {\"columns\":3,\"linkTo\":\"none\"} -->",
            "<figcaption class=\"blocks-gallery-caption wp-element-caption\">Temples</figcaption>",
            "<hr class=\"wp-block-separator has-alpha-channel-opacity\"/>",
            "<p class=\"picasso-credits\">Photos by Ana</p>",
            "<!-- wp:heading {\"level\":3} -->\n<h3 class=\"wp-block-heading\">Evening</h3>",
            "<!-- wp:list {\"ordered\":true} -->\n<ol class=\"wp-block-list\"><!-- wp:list-item -->\n<li><strong>Tea</strong></li>\n<!-- /wp:list-item -->\n\n<!-- wp:list-item -->\n<li><a href=\"https://x.test\">Map</a></li>",
            "<blockquote class=\"wp-block-quote\"><!-- wp:paragraph -->\n<p>Go <em>slowly</em></p>\n<!-- /wp:paragraph --><cite>Bashō</cite></blockquote>",
            "<p>Back &amp; again</p>",
        ];
        let mut from = 0;
        for part in order {
            let at = content[from..].find(part).unwrap_or_else(|| panic!("{part} missing or out of order in\n{content}"));
            from += at + part.len();
        }
        // Every block opened is closed, and nothing from WordPress was in the Story.
        assert_eq!(content.matches("<!-- wp:").count(), content.matches("<!-- /wp:").count());
    }

    #[test]
    fn without_a_cover_the_first_photo_is_featured() {
        let mut story = story();
        story.cover = None;
        let blocks = [StoryBlock::new(Block::Paragraph("Hola".into())), StoryBlock::new(Block::Gallery(vec![MediaUse::new(2), MediaUse::new(1)]))];
        assert_eq!(post(&story, &blocks, &uploads, &|_| None, &|_| None).unwrap().featured, Some(102));
        assert_eq!(post(&story, &blocks[..1], &uploads, &|_| None, &|_| None).unwrap().featured, None, "no photo, nothing featured");
    }

    #[test]
    fn a_block_without_a_core_form_or_a_photo_without_an_upload_stops_it() {
        let unknown = StoryBlock::new(Block::Unknown { kind: "map".into(), version: 9, payload: "{}".into(), media: Vec::new() });
        assert_eq!(post(&story(), &[StoryBlock::new(Block::Separator), unknown], &uploads, &|_| None, &|_| None), Err(Unsupported::Unknown(1)));
        let missing = StoryBlock::new(Block::Photo(MediaUse::new(42)));
        assert_eq!(post(&story(), &[missing], &uploads, &|_| None, &|_| None), Err(Unsupported::Photo(42)));
    }

    #[test]
    fn practical_info_is_a_group_and_a_link_a_paragraph() {
        let blocks = [
            StoryBlock::new(Block::Info { title: "Getting there".into(), items: vec![("Train".into(), "2 h".into()), (String::new(), "Book <ahead>".into())] }),
            StoryBlock::new(Block::Link { url: "https://example.com/?a=1&b=2".into(), title: String::new(), note: "Timetable".into() }),
            StoryBlock::new(Block::Link { url: "javascript:alert(1)".into(), title: "Trick".into(), note: String::new() }),
        ];
        let content = post(&story(), &blocks, &uploads, &|_| None, &|_| None).unwrap().content;
        assert!(content.contains("<!-- wp:group {\"className\":\"picasso-info\",\"layout\":{\"type\":\"constrained\"}} -->"), "{content}");
        assert!(content.contains("<h3 class=\"wp-block-heading\">Getting there</h3>") && content.contains("<li><strong>Train:</strong> 2 h</li>") && content.contains("<li>Book &lt;ahead&gt;</li>"), "{content}");
        assert!(content.contains("<a href=\"https://example.com/?a=1&amp;b=2\" rel=\"noopener\">https://example.com/?a=1&amp;b=2</a><br>Timetable"), "{content}");
        assert!(content.contains("<p class=\"picasso-link\">Trick</p>") && !content.contains("javascript:"), "{content}");
    }

    #[test]
    fn a_youtube_video_is_a_core_embed() {
        let blocks = [
            StoryBlock::new(Block::YouTube { url: "https://youtu.be/ab--DEF_123?t=90".into(), caption: "The <ferry>".into() }),
            StoryBlock::new(Block::YouTube { url: "https://www.youtube.com/watch?v=xyz-UVW_789".into(), caption: String::new() }),
            StoryBlock::new(Block::YouTube { url: "https://vimeo.com/123456789".into(), caption: "Elsewhere".into() }),
            StoryBlock::new(Block::YouTube { url: String::new(), caption: " ".into() }),
        ];
        let content = post(&story(), &blocks, &uploads, &|_| None, &|_| None).unwrap().content;
        let started = "<!-- wp:embed {\"className\":\"wp-embed-aspect-16-9 wp-has-aspect-ratio\",\"providerNameSlug\":\"youtube\",\"responsive\":true,\"type\":\"video\",\"url\":\"https://www.youtube.com/watch?v=ab\\u002d\\u002dDEF_123\\u0026t=90s\"} -->\n\
                       <figure class=\"wp-block-embed is-type-video is-provider-youtube wp-block-embed-youtube wp-embed-aspect-16-9 wp-has-aspect-ratio\"><div class=\"wp-block-embed__wrapper\">\n\
                       https://www.youtube.com/watch?v=ab--DEF_123&amp;t=90s\n</div><figcaption class=\"wp-element-caption\">The &lt;ferry&gt;</figcaption></figure>\n<!-- /wp:embed -->";
        assert!(content.contains(started), "{content}");
        assert!(content.contains("<div class=\"wp-block-embed__wrapper\">\nhttps://www.youtube.com/watch?v=xyz-UVW_789\n</div></figure>"), "{content}");
        assert!(content.contains("<p>Elsewhere</p>") && !content.contains("vimeo"), "{content}");
        assert_eq!(content.matches("<!-- wp:embed").count(), 2);
    }

    #[test]
    fn a_video_is_a_core_video_with_its_poster_and_caption() {
        let mut item = MediaUse::new(7);
        item.caption = "The <ferry>".into();
        item.alt = "Waves".into();
        let video = StoryBlock::new(Block::Video { media: item, start: 1_000, end: 5_000 });
        let uid = video.uid.clone();
        let uploaded = |key: &str| {
            (key == uid).then(|| UploadedVideo { video: Uploaded { id: 31, url: "https://x.test/v.mp4".into() }, poster: Uploaded { id: 32, url: "https://x.test/v.jpg".into() } })
        };
        let mut story = story();
        story.cover = None;
        let made = post(&story, std::slice::from_ref(&video), &uploads, &|_| None, &uploaded).unwrap();
        let block = "<!-- wp:video {\"id\":31} -->\n<figure class=\"wp-block-video\"><video controls poster=\"https://x.test/v.jpg\" src=\"https://x.test/v.mp4\"></video>\
                     <figcaption class=\"wp-element-caption\">The &lt;ferry&gt;</figcaption></figure>\n<!-- /wp:video -->";
        assert!(made.content.contains(block), "{}", made.content);
        assert_eq!(made.featured, None, "a video is never the featured image");
        assert_eq!(post(&story, &[video], &uploads, &|_| None, &|_| None), Err(Unsupported::Video(7)));
    }

    #[test]
    fn a_table_is_a_core_table_with_its_header_row_and_caption() {
        let rows = vec![vec!["Ferry".to_string(), "Leaves".to_string()], vec!["<Lisbon>".to_string(), "9:00".to_string()]];
        let blocks = [
            StoryBlock::new(Block::Table { header: true, rows: rows.clone(), widths: Vec::new(), caption: "Summer".into() }),
            StoryBlock::new(Block::Table { header: false, rows, widths: vec![30, 70], caption: String::new() }),
            StoryBlock::new(Block::table(2, 2)),
        ];
        let content = post(&story(), &blocks, &uploads, &|_| None, &|_| None).unwrap().content;
        assert!(content.contains("<!-- wp:table {\"hasFixedLayout\":false,\"picassoColumnWidths\":[30,70]} -->\n<figure class=\"wp-block-table\"><table><tbody>"), "{content}");
        let head = "<!-- wp:table {\"hasFixedLayout\":false} -->\n<figure class=\"wp-block-table\"><table><thead><tr><th>Ferry</th><th>Leaves</th></tr></thead>\
                    <tbody><tr><td>&lt;Lisbon&gt;</td><td>9:00</td></tr></tbody></table><figcaption class=\"wp-element-caption\">Summer</figcaption></figure>\n<!-- /wp:table -->";
        assert!(content.contains(head), "{content}");
        assert!(content.contains("<table><tbody><tr><td>Ferry</td><td>Leaves</td></tr><tr><td>&lt;Lisbon&gt;</td><td>9:00</td></tr></tbody></table></figure>"), "{content}");
        assert_eq!(content.matches("<!-- wp:table").count(), 2, "an empty table is left out");
    }

    #[test]
    fn coloured_text_is_gutenbergs_inline_colour_in_every_block_that_has_text() {
        let red = "**{red}Dinner: Botín 8:00 PM{/red}**";
        let blocks = [
            Block::Heading(red.into()),
            Block::Subheading(red.into()),
            Block::Paragraph(red.into()),
            Block::List { ordered: false, items: vec![red.into()] },
            Block::Quote { text: red.into(), citation: red.into() },
            Block::Info { title: "Evening".into(), items: vec![("Where".into(), red.into())] },
            Block::Link { url: "https://example.com".into(), title: "Botín".into(), note: red.into() },
            Block::Table { header: true, rows: vec![vec![red.into()], vec![red.into()]], widths: Vec::new(), caption: String::new() },
            Block::Credits(red.into()),
        ];
        let blocks: Vec<StoryBlock> = blocks.into_iter().map(StoryBlock::new).collect();
        let content = post(&story(), &blocks, &uploads, &|_| None, &|_| None).unwrap().content;
        let colored = "<mark style=\"background-color:transparent;color:#c62828\" class=\"has-inline-color picasso-color-red\"><strong>Dinner: Botín 8:00 PM</strong></mark>";
        assert_eq!(content.matches(colored).count(), 11, "{content}");
        assert!(!content.contains("class=\"color-") && !content.contains("{red}"), "the page's classes and the marks stay out of the post: {content}");
    }

    #[test]
    fn a_route_with_its_map_is_an_image_and_without_one_a_drawing() {
        let route = StoryBlock::new(Block::Route { caption: "Walk".into(), source: String::new(), route: vec![vec![[0, 0], [100_000, 100_000]]] });
        let uid = route.uid.clone();
        let map = |key: &str| (key == uid).then(|| RouteMap { uploaded: Uploaded { id: 9, url: "https://x.test/m.jpg".into() }, credit: "© Maps" });
        let with = post(&story(), std::slice::from_ref(&route), &uploads, &map, &|_| None).unwrap().content;
        assert!(with.contains("<!-- wp:image {") && with.contains("wp-image-9") && with.contains("© Maps") && !with.contains("<svg"), "{with}");
        let without = post(&story(), &[route], &uploads, &|_| None, &|_| None).unwrap().content;
        assert!(without.contains("<!-- wp:html -->") && without.contains("<svg"), "{without}");
    }
}
