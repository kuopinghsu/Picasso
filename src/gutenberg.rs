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
    pub content: String,
    pub featured: Option<i64>,
}

/// The Story as a post. The introduction comes before the blocks and the
/// conclusion after them, as paragraphs; the title, subtitle, and byline
/// are the post's own. `maps` gives each Route block's map picture by the
/// block's uid; a route without one is drawn on its own.
pub fn post(story: &Story, blocks: &[StoryBlock], media: &dyn Fn(i64) -> Option<Uploaded>, maps: &dyn Fn(&str) -> Option<RouteMap>) -> Result<Post, Unsupported> {
    let mut out = String::new();
    paragraphs(&mut out, &story.introduction, None);
    for (index, block) in blocks.iter().enumerate() {
        match &block.block {
            Block::Heading(text) => {
                if !text.trim().is_empty() {
                    let _ = write!(out, "<!-- wp:heading -->\n<h2 class=\"wp-block-heading\">{}</h2>\n<!-- /wp:heading -->\n\n", inline::to_html(text.trim()));
                }
            }
            Block::Subheading(text) => {
                if !text.trim().is_empty() {
                    let _ = write!(
                        out,
                        "<!-- wp:heading {{\"level\":3}} -->\n<h3 class=\"wp-block-heading\">{}</h3>\n<!-- /wp:heading -->\n\n",
                        inline::to_html(text.trim())
                    );
                }
            }
            Block::List { ordered, items } => {
                let items: Vec<String> = items
                    .iter()
                    .map(|item| item.trim())
                    .filter(|item| !item.is_empty())
                    .map(|item| format!("<!-- wp:list-item -->\n<li>{}</li>\n<!-- /wp:list-item -->", inline::to_html(item)))
                    .collect();
                if !items.is_empty() {
                    let (attrs, tag) = if *ordered { (" {\"ordered\":true}", "ol") } else { ("", "ul") };
                    let _ = write!(out, "<!-- wp:list{attrs} -->\n<{tag} class=\"wp-block-list\">{}</{tag}>\n<!-- /wp:list -->\n\n", items.join("\n\n"));
                }
            }
            Block::Quote { text, citation } => {
                let mut inner = String::new();
                paragraphs(&mut inner, text, None);
                let cite = if citation.trim().is_empty() { String::new() } else { format!("<cite>{}</cite>", inline::to_html(citation.trim())) };
                let _ = write!(out, "<!-- wp:quote -->\n<blockquote class=\"wp-block-quote\">{}{cite}</blockquote>\n<!-- /wp:quote -->\n\n", inner.trim_end());
            }
            Block::Paragraph(text) => paragraphs(&mut out, text, None),
            Block::Credits(text) => paragraphs(&mut out, text, Some("picasso-credits")),
            Block::Photo(item) => {
                let uploaded = media(item.media_id).ok_or(Unsupported::Photo(item.media_id))?;
                out.push_str(&image(item, &uploaded));
                out.push('\n');
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
    let featured = match story.cover.or_else(|| blocks.iter().find_map(|block| block.block.media().first().map(|first| first.media_id))) {
        Some(cover) => Some(media(cover).ok_or(Unsupported::Photo(cover))?.id),
        None => None,
    };
    Ok(Post { title: story.title.trim().to_string(), content: out.trim_end().to_string(), featured })
}

/// Each part between blank lines as its own paragraph block, its lines
/// kept with line breaks.
fn paragraphs(out: &mut String, text: &str, class: Option<&str>) {
    for part in text.split("\n\n").map(str::trim).filter(|part| !part.is_empty()) {
        let lines: Vec<String> = part.lines().map(|line| inline::to_html(line.trim_end())).collect();
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
            cover: Some(3),
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
        let post = post(&story(), &blocks, &uploads, &|_| None).unwrap();
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
        assert_eq!(post(&story, &blocks, &uploads, &|_| None).unwrap().featured, Some(102));
        assert_eq!(post(&story, &blocks[..1], &uploads, &|_| None).unwrap().featured, None, "no photo, nothing featured");
    }

    #[test]
    fn a_block_without_a_core_form_or_a_photo_without_an_upload_stops_it() {
        let unknown = StoryBlock::new(Block::Unknown { kind: "map".into(), version: 9, payload: "{}".into(), media: Vec::new() });
        assert_eq!(post(&story(), &[StoryBlock::new(Block::Separator), unknown], &uploads, &|_| None), Err(Unsupported::Unknown(1)));
        let missing = StoryBlock::new(Block::Photo(MediaUse::new(42)));
        assert_eq!(post(&story(), &[missing], &uploads, &|_| None), Err(Unsupported::Photo(42)));
    }

    #[test]
    fn a_route_with_its_map_is_an_image_and_without_one_a_drawing() {
        let route = StoryBlock::new(Block::Route { caption: "Walk".into(), source: String::new(), route: vec![vec![[0, 0], [100_000, 100_000]]] });
        let uid = route.uid.clone();
        let map = |key: &str| (key == uid).then(|| RouteMap { uploaded: Uploaded { id: 9, url: "https://x.test/m.jpg".into() }, credit: "© Maps" });
        let with = post(&story(), std::slice::from_ref(&route), &uploads, &map).unwrap().content;
        assert!(with.contains("<!-- wp:image {") && with.contains("wp-image-9") && with.contains("© Maps") && !with.contains("<svg"), "{with}");
        let without = post(&story(), &[route], &uploads, &|_| None).unwrap().content;
        assert!(without.contains("<!-- wp:html -->") && without.contains("<svg"), "{without}");
    }
}
