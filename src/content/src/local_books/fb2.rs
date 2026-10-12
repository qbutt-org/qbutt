//! FB2 sections, embedded images and XML rendering.

use std::collections::HashMap;

use base64::Engine;
use roxmltree::{Document, Node, NodeId, ParsingOptions};
use sha2::{Digest, Sha256};

use super::{
    LocalBook, LocalBookChapter, MAX_ARCHIVE_ENTRIES, MAX_IMAGE_BYTES, check_html_size,
    decode_text, escape_html, image_media_type, valid_image_bytes,
};
use crate::{ChapterRef, Error, Result, TextChapter, TextImageAsset};

impl LocalBook {
    pub(super) fn inspect_fb2(&mut self) -> Result<()> {
        let xml = decode_text(&self.read_verified_bytes()?)?;
        let document = parse_fb2(&xml)?;
        let title = document
            .descendants()
            .find(|node| node.has_tag_name("book-title"))
            .and_then(|node| node.text())
            .map(str::trim)
            .filter(|title| !title.is_empty());
        if let Some(title) = title {
            self.title = title.chars().take(300).collect();
        }
        let body = document
            .descendants()
            .find(|node| node.has_tag_name("body") && node.attribute("name").is_none())
            .ok_or(Error::InvalidLocalBook("The FB2 has no main body"))?;
        for (index, section) in body
            .descendants()
            .filter(|node| node.has_tag_name("section"))
            .enumerate()
        {
            if index >= MAX_ARCHIVE_ENTRIES {
                return Err(Error::ResponseTooLarge);
            }
            let title = section
                .children()
                .find(|node| node.has_tag_name("title"))
                .map(|node| {
                    node.descendants()
                        .filter(|child| child.is_text())
                        .filter_map(|child| child.text())
                        .collect::<String>()
                })
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| format!("Chapter {}", index + 1));
            self.chapters.push(LocalBookChapter {
                id: (index + 1).to_string(),
                title: title.trim().chars().take(300).collect(),
                resource: section.id().get().to_string(),
                anchor: None,
                end_anchor: None,
            });
        }
        if self.chapters.is_empty() {
            self.chapters.push(LocalBookChapter {
                id: "1".into(),
                title: self.title.clone(),
                resource: "body".into(),
                anchor: None,
                end_anchor: None,
            });
        }
        Ok(())
    }

    pub(super) fn read_fb2_chapter(
        &self,
        reference: &ChapterRef,
        chapter: &LocalBookChapter,
    ) -> Result<TextChapter> {
        let xml = decode_text(&self.read_verified_bytes()?)?;
        let document = parse_fb2(&xml)?;
        let body = document
            .descendants()
            .find(|node| node.has_tag_name("body") && node.attribute("name").is_none())
            .ok_or(Error::InvalidLocalBook("The FB2 has no main body"))?;
        let section = if chapter.resource == "body" {
            body
        } else {
            document
                .get_node(NodeId::new(
                    chapter
                        .resource
                        .parse::<u32>()
                        .map_err(|_| Error::InvalidReference)?,
                ))
                .filter(|node| node.has_tag_name("section"))
                .ok_or(Error::InvalidReference)?
        };
        let mut html = String::new();
        if section == body {
            render_fb2(section, &mut html, 0)?;
        } else {
            html.push_str("<section>");
            for child in section
                .children()
                .filter(|child| !child.has_tag_name("section"))
            {
                render_fb2(child, &mut html, 1)?;
            }
            html.push_str("</section>");
        }
        let binary = document
            .descendants()
            .filter(|node| node.has_tag_name("binary"))
            .filter_map(|node| node.attribute("id").map(|id| (id, node)))
            .collect::<HashMap<_, _>>();
        let mut resolve_image = |source: &str| -> Result<Option<TextImageAsset>> {
            let Some(id) = source.strip_prefix('#') else {
                return Ok(None);
            };
            let Some(node) = binary.get(id) else {
                return Ok(None);
            };
            let Some(media_type) = node.attribute("content-type").and_then(image_media_type) else {
                return Ok(None);
            };
            let encoded = node.text().unwrap_or_default();
            if encoded.len() > MAX_IMAGE_BYTES * 2 {
                return Err(Error::ResponseTooLarge);
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded.split_ascii_whitespace().collect::<String>())
                .map_err(|_| Error::InvalidLocalBook("An FB2 image is corrupt"))?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(Error::ResponseTooLarge);
            }
            if !valid_image_bytes(media_type, &bytes) {
                return Err(Error::InvalidLocalBook("An FB2 image is invalid"));
            }
            Ok(Some(TextImageAsset {
                id: format!("{:x}", Sha256::digest(&bytes)),
                media_type: media_type.into(),
                bytes: bytes.into(),
            }))
        };
        let chapter =
            TextChapter::from_local_html(reference.clone(), &html, &mut resolve_image, None, None)?;
        Ok(chapter)
    }
}

fn parse_fb2(xml: &str) -> Result<Document<'_>> {
    Document::parse_with_options(
        xml,
        ParsingOptions {
            nodes_limit: 100_000,
            ..ParsingOptions::default()
        },
    )
    .map_err(|_| Error::InvalidLocalBook("The FB2 XML is invalid"))
}

fn render_fb2(node: Node<'_, '_>, output: &mut String, depth: usize) -> Result<()> {
    if depth > 128 {
        return Err(Error::ResponseTooLarge);
    }
    if node.is_text() {
        return escape_html(node.text().unwrap_or_default(), output);
    }
    if !node.is_element() {
        return Ok(());
    }
    if node.has_tag_name("title") {
        output.push_str("<h2>");
        for child in node.descendants().filter(|child| child.is_text()) {
            escape_html(child.text().unwrap_or_default(), output)?;
        }
        output.push_str("</h2>");
        return check_html_size(output);
    }
    let tag = match node.tag_name().name() {
        "subtitle" => "h2",
        "p" | "v" | "text-author" => "p",
        "strong" => "strong",
        "emphasis" => "em",
        "epigraph" | "cite" => "blockquote",
        "section" | "body" | "poem" | "stanza" => "section",
        "image" => {
            if let Some(source) = node
                .attributes()
                .find(|attribute| attribute.name() == "href")
            {
                output.push_str("<img src=\"");
                escape_html(source.value(), output)?;
                output.push_str("\">");
            }
            return check_html_size(output);
        }
        "empty-line" => {
            output.push_str("<br>");
            return Ok(());
        }
        "binary" | "script" | "style" => return Ok(()),
        _ => {
            for child in node.children() {
                render_fb2(child, output, depth + 1)?;
            }
            return Ok(());
        }
    };
    output.push('<');
    output.push_str(tag);
    output.push('>');
    for child in node.children() {
        render_fb2(child, output, depth + 1)?;
    }
    output.push_str("</");
    output.push_str(tag);
    output.push('>');
    check_html_size(output)
}
