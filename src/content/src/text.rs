//! Original chapter content. Providers select the chapter body; local books
//! may attach bounded images, but no executable markup or external resources.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use scraper::{ElementRef, Html, Node};
use serde::{Deserialize, Serialize};

use crate::{ChapterRef, Error, Result};

const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_BLOCKS: usize = 20_000;
const MAX_SPANS: usize = 40_000;
const MAX_DEPTH: usize = 128;
const MAX_ASSET_BYTES: usize = 24 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextBlockKind {
    Paragraph,
    Heading,
    Separator,
    ListItem { ordinal: Option<u32> },
    Quote,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TextSpan {
    Text {
        text: String,
        bold: bool,
        italic: bool,
    },
    Ruby {
        text: String,
        reading: String,
    },
    Image {
        asset_id: String,
        alt: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextImageAsset {
    pub id: String,
    pub media_type: String,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextBlock {
    pub id: String,
    pub kind: TextBlockKind,
    pub spans: Vec<TextSpan>,
}

impl TextBlock {
    pub fn plain_text(&self) -> String {
        let mut output = String::new();
        for span in &self.spans {
            match span {
                TextSpan::Text { text, .. } => output.push_str(text),
                TextSpan::Ruby { text, reading } => {
                    output.push_str(text);
                    if !reading.is_empty() {
                        output.push('(');
                        output.push_str(reading);
                        output.push(')');
                    }
                }
                TextSpan::Image { alt, .. } => output.push_str(alt),
            }
        }
        output
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextChapter {
    pub reference: ChapterRef,
    pub blocks: Vec<TextBlock>,
    #[serde(skip)]
    pub assets: Vec<TextImageAsset>,
    #[serde(skip)]
    pub entry_anchor: Option<String>,
}

impl TextChapter {
    /// Accept only the selected chapter body, never the whole source page.
    /// Block IDs depend on content rather than layout or preceding paragraphs.
    pub fn from_html(reference: ChapterRef, body: &str) -> Result<Self> {
        reference.validate()?;
        if body.len() > MAX_HTML_BYTES {
            return Err(Error::ResponseTooLarge);
        }
        let fragment = Html::parse_fragment(body);
        let mut builder = ChapterBuilder::default();
        builder.visit(fragment.root_element(), false, false, 0)?;
        builder.finish(TextBlockKind::Paragraph)?;
        Self::from_blocks(reference, builder.blocks)
    }

    pub fn from_local_html(
        reference: ChapterRef,
        body: &str,
        resolve_image: &mut dyn FnMut(&str) -> Result<Option<TextImageAsset>>,
        entry_fragment: Option<&str>,
        end_fragment: Option<&str>,
    ) -> Result<Self> {
        reference.validate()?;
        if body.len() > MAX_HTML_BYTES {
            return Err(Error::ResponseTooLarge);
        }
        let fragment = Html::parse_fragment(body);
        let mut builder = ChapterBuilder {
            image_resolver: Some(resolve_image),
            entry_fragment,
            end_fragment,
            ..ChapterBuilder::default()
        };
        builder.visit(fragment.root_element(), false, false, 0)?;
        builder.finish(TextBlockKind::Paragraph)?;
        let total = builder.blocks.len();
        let entry_found = builder.entry_index.is_some();
        let start = builder.entry_index.unwrap_or(0);
        let end = builder.end_index.unwrap_or(total);
        let range = if (entry_fragment.is_some() && !entry_found) || end <= start {
            0..total
        } else {
            start..end
        };
        let blocks = builder.blocks.drain(range).collect::<Vec<_>>();
        let image_ids = blocks
            .iter()
            .flat_map(|block| &block.spans)
            .filter_map(|span| match span {
                TextSpan::Image { asset_id, .. } => Some(asset_id.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let assets = builder
            .assets
            .into_iter()
            .filter(|asset| image_ids.contains(asset.id.as_str()))
            .collect();
        let mut chapter = Self::from_blocks_with_assets(reference, blocks, assets)?;
        chapter.entry_anchor = if entry_found {
            chapter.blocks.first().map(|block| block.id.clone())
        } else {
            None
        };
        Ok(chapter)
    }

    /// Native structured sources share the same content anchors as HTML sources.
    /// Supplied IDs are replaced; layout and source DOM IDs are not persisted.
    pub fn from_blocks(reference: ChapterRef, blocks: Vec<TextBlock>) -> Result<Self> {
        Self::from_blocks_with_assets(reference, blocks, Vec::new())
    }

    pub fn from_blocks_with_assets(
        reference: ChapterRef,
        mut blocks: Vec<TextBlock>,
        assets: Vec<TextImageAsset>,
    ) -> Result<Self> {
        let mut occurrences = HashMap::new();
        for block in &mut blocks {
            let mut text = block.plain_text();
            for span in &block.spans {
                if let TextSpan::Image { asset_id, .. } = span {
                    text.push_str(asset_id);
                }
            }
            // FNV-1a is only an anchor, never a trust or integrity boundary.
            let hash = text.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
            let occurrence = occurrences.entry(hash).or_insert(0usize);
            block.id = format!("{hash:016x}-{occurrence}");
            *occurrence += 1;
        }
        let chapter = Self {
            reference,
            blocks,
            assets,
            entry_anchor: None,
        };
        chapter.validate()?;
        Ok(chapter)
    }

    pub fn text_bytes(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|block| &block.spans)
            .map(|span| match span {
                TextSpan::Text { text, .. } => text.len(),
                TextSpan::Ruby { text, reading } => text.len() + reading.len(),
                TextSpan::Image { alt, .. } => alt.len(),
            })
            .sum()
    }

    pub fn validate(&self) -> Result<()> {
        self.reference.validate()?;
        if self.blocks.is_empty() {
            return Err(Error::InvalidResponse("chapter text is empty"));
        }
        if self.blocks.len() > MAX_BLOCKS
            || self
                .blocks
                .iter()
                .map(|block| block.spans.len())
                .sum::<usize>()
                > MAX_SPANS
            || self.text_bytes() > MAX_TEXT_BYTES
        {
            return Err(Error::ResponseTooLarge);
        }
        let mut ids = HashSet::new();
        let asset_ids = self
            .assets
            .iter()
            .map(|asset| asset.id.as_str())
            .collect::<HashSet<_>>();
        if asset_ids.len() != self.assets.len()
            || self
                .assets
                .iter()
                .map(|asset| asset.bytes.len())
                .sum::<usize>()
                > MAX_ASSET_BYTES
        {
            return Err(Error::ResponseTooLarge);
        }
        for block in &self.blocks {
            if block.id.is_empty()
                || block.id.len() > 128
                || block.id.chars().any(char::is_control)
                || !ids.insert(&block.id)
            {
                return Err(Error::InvalidResponse("chapter text anchor is invalid"));
            }
            if block.spans.iter().any(|span| {
                matches!(span, TextSpan::Image { asset_id, .. } if !asset_ids.contains(asset_id.as_str()))
            }) {
                return Err(Error::InvalidResponse("chapter image is missing"));
            }
        }
        Ok(())
    }
}

/// A reader-session cache of original content. It has no translation state,
/// persistence, network tasks or ownership outside the caller's reader session.
#[derive(Default)]
pub struct TextChapterCache {
    chapters: VecDeque<Arc<TextChapter>>,
}

impl TextChapterCache {
    pub fn get(&mut self, reference: &ChapterRef) -> Option<Arc<TextChapter>> {
        let index = self
            .chapters
            .iter()
            .position(|chapter| &chapter.reference == reference)?;
        let chapter = self.chapters.remove(index)?;
        self.chapters.push_back(Arc::clone(&chapter));
        Some(chapter)
    }

    pub fn insert(&mut self, chapter: TextChapter) -> Result<Arc<TextChapter>> {
        chapter.validate()?;
        self.chapters
            .retain(|cached| cached.reference != chapter.reference);
        let chapter = Arc::new(chapter);
        self.chapters.push_back(Arc::clone(&chapter));
        // Each validated chapter holds at most 2 MiB of original text.
        while self.chapters.len() > 3 {
            self.chapters.pop_front();
        }
        Ok(chapter)
    }

    pub fn clear(&mut self) {
        self.chapters.clear();
    }
}

#[derive(Default)]
struct ChapterBuilder<'a> {
    blocks: Vec<TextBlock>,
    spans: Vec<TextSpan>,
    assets: Vec<TextImageAsset>,
    entry_fragment: Option<&'a str>,
    entry_index: Option<usize>,
    end_fragment: Option<&'a str>,
    end_index: Option<usize>,
    image_resolver: Option<&'a mut dyn FnMut(&str) -> Result<Option<TextImageAsset>>>,
    asset_bytes: usize,
    text_bytes: usize,
    span_count: usize,
}

impl ChapterBuilder<'_> {
    fn visit(
        &mut self,
        element: ElementRef<'_>,
        bold: bool,
        italic: bool,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(Error::ResponseTooLarge);
        }
        let name = element.value().name();
        if matches!(
            name,
            "script"
                | "style"
                | "iframe"
                | "object"
                | "embed"
                | "form"
                | "input"
                | "button"
                | "link"
                | "meta"
                | "base"
                | "canvas"
                | "video"
                | "audio"
                | "rt"
                | "rp"
                | "noscript"
                | "template"
        ) || element.value().attr("hidden").is_some()
        {
            return Ok(());
        }
        if element.value().attr("style").is_some_and(hidden_style) {
            return Ok(());
        }
        if name == "svg" {
            for node in element.descendants().filter_map(ElementRef::wrap) {
                if node.value().name() == "image" {
                    if let Some(source) = node
                        .value()
                        .attr("href")
                        .or_else(|| node.value().attr("xlink:href"))
                    {
                        self.image(source, node.value().attr("aria-label").unwrap_or_default())?;
                    }
                }
            }
            return Ok(());
        }
        if name == "img" {
            if let Some(source) = element.value().attr("src") {
                self.image(source, element.value().attr("alt").unwrap_or_default())?;
            }
            return Ok(());
        }
        if name == "br" {
            return self.text("\n", bold, italic);
        }
        if name == "hr" {
            self.finish(TextBlockKind::Paragraph)?;
            return self.finish(TextBlockKind::Separator);
        }
        if name == "ruby" {
            let mut text = String::new();
            let mut reading = String::new();
            for child in element.children() {
                match child.value() {
                    Node::Text(value) => text.push_str(value),
                    Node::Element(value) if value.name() == "rt" => {
                        reading.extend(ElementRef::wrap(child).unwrap().text());
                    }
                    Node::Element(value) if value.name() != "rp" => {
                        text.extend(ElementRef::wrap(child).unwrap().text());
                    }
                    _ => {}
                }
            }
            let text = normalize_text(&text);
            let reading = normalize_text(&reading);
            self.account(text.len() + reading.len())?;
            self.spans.push(TextSpan::Ruby { text, reading });
            return Ok(());
        }

        let heading = matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
        let inline_paragraph = name == "p"
            && element
                .parent()
                .and_then(ElementRef::wrap)
                .is_some_and(|parent| {
                    matches!(
                        parent.value().name(),
                        "li" | "blockquote" | "h1" | "h2" | "h3"
                    )
                });
        let block = !inline_paragraph
            && (heading || matches!(name, "p" | "div" | "section" | "blockquote" | "li"));
        if block {
            self.finish(TextBlockKind::Paragraph)?;
        }
        if self.entry_index.is_none()
            && self
                .entry_fragment
                .is_some_and(|fragment| element.value().attr("id") == Some(fragment))
        {
            self.entry_index = Some(self.blocks.len());
        }
        if self.end_index.is_none()
            && self
                .end_fragment
                .is_some_and(|fragment| element.value().attr("id") == Some(fragment))
        {
            self.end_index = Some(self.blocks.len());
        }
        let bold = bold || matches!(name, "b" | "strong");
        let italic = italic || matches!(name, "i" | "em");
        for child in element.children() {
            match child.value() {
                Node::Text(value) => self.text(&normalize_text(value), bold, italic)?,
                Node::Element(_) => {
                    self.visit(ElementRef::wrap(child).unwrap(), bold, italic, depth + 1)?
                }
                _ => {}
            }
        }
        if block {
            let kind = if heading {
                TextBlockKind::Heading
            } else if name == "blockquote" {
                TextBlockKind::Quote
            } else if name == "li" {
                let ordered = element
                    .parent()
                    .and_then(ElementRef::wrap)
                    .is_some_and(|parent| parent.value().name() == "ol");
                let ordinal = ordered.then(|| {
                    element
                        .prev_siblings()
                        .filter_map(ElementRef::wrap)
                        .filter(|sibling| sibling.value().name() == "li")
                        .count() as u32
                        + 1
                });
                TextBlockKind::ListItem { ordinal }
            } else {
                TextBlockKind::Paragraph
            };
            self.finish(kind)?;
        }
        Ok(())
    }

    fn text(&mut self, text: &str, bold: bool, italic: bool) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.account(text.len())?;
        if let Some(TextSpan::Text {
            text: previous,
            bold: was_bold,
            italic: was_italic,
        }) = self.spans.last_mut()
            && bold == *was_bold
            && italic == *was_italic
        {
            previous.push_str(text);
        } else {
            self.spans.push(TextSpan::Text {
                text: text.into(),
                bold,
                italic,
            });
        }
        Ok(())
    }

    fn image(&mut self, source: &str, alt: &str) -> Result<()> {
        let Some(resolve) = self.image_resolver.as_mut() else {
            return Ok(());
        };
        let Some(asset) = resolve(source)? else {
            return Ok(());
        };
        self.finish(TextBlockKind::Paragraph)?;
        self.account(asset.id.len())?;
        self.spans.push(TextSpan::Image {
            asset_id: asset.id.clone(),
            alt: alt.chars().take(200).collect(),
        });
        if !self.assets.iter().any(|existing| existing.id == asset.id) {
            self.asset_bytes = self
                .asset_bytes
                .checked_add(asset.bytes.len())
                .filter(|total| *total <= MAX_ASSET_BYTES)
                .ok_or(Error::ResponseTooLarge)?;
            self.assets.push(asset);
        }
        self.finish(TextBlockKind::Image)
    }

    fn account(&mut self, bytes: usize) -> Result<()> {
        self.text_bytes += bytes;
        self.span_count += 1;
        if self.text_bytes > MAX_TEXT_BYTES || self.span_count > MAX_SPANS {
            return Err(Error::ResponseTooLarge);
        }
        Ok(())
    }

    fn finish(&mut self, kind: TextBlockKind) -> Result<()> {
        if self.spans.is_empty() && kind != TextBlockKind::Separator {
            return Ok(());
        }
        let block = TextBlock {
            id: String::new(),
            kind,
            spans: std::mem::take(&mut self.spans),
        };
        let text = block.plain_text();
        if text.trim().is_empty()
            && !matches!(kind, TextBlockKind::Separator | TextBlockKind::Image)
        {
            return Ok(());
        }
        if self.blocks.len() >= MAX_BLOCKS {
            return Err(Error::ResponseTooLarge);
        }
        self.blocks.push(block);
        Ok(())
    }
}

fn normalize_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut whitespace = false;
    for character in text.chars() {
        if character.is_ascii_whitespace() {
            if !whitespace {
                output.push(' ');
            }
            whitespace = true;
        } else if !character.is_control() {
            output.push(character);
            whitespace = false;
        }
    }
    output
}

fn hidden_style(style: &str) -> bool {
    style.split(';').any(|declaration| {
        declaration.split_once(':').is_some_and(|(name, value)| {
            let value = value.split('!').next().unwrap_or_default().trim();
            (name.trim().eq_ignore_ascii_case("display") && value.eq_ignore_ascii_case("none"))
                || (name.trim().eq_ignore_ascii_case("visibility")
                    && value.eq_ignore_ascii_case("hidden"))
        })
    })
}
