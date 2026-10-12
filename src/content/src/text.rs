//! Original chapter text. Providers select the chapter body; this module keeps
//! only text, emphasis and ruby, with no executable markup or external assets.

use std::collections::HashMap;

use scraper::{ElementRef, Html, Node};
use serde::{Deserialize, Serialize};

use crate::{ChapterRef, Error, Result};

const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_BLOCKS: usize = 20_000;
const MAX_SPANS: usize = 40_000;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextBlockKind {
    Paragraph,
    Heading,
    Separator,
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
        if builder.blocks.is_empty() {
            return Err(Error::InvalidResponse("chapter text is empty"));
        }
        Ok(Self {
            reference,
            blocks: builder.blocks,
        })
    }

    pub fn text_bytes(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|block| &block.spans)
            .map(|span| match span {
                TextSpan::Text { text, .. } => text.len(),
                TextSpan::Ruby { text, reading } => text.len() + reading.len(),
            })
            .sum()
    }
}

#[derive(Default)]
struct ChapterBuilder {
    blocks: Vec<TextBlock>,
    spans: Vec<TextSpan>,
    occurrences: HashMap<u64, usize>,
    text_bytes: usize,
    span_count: usize,
}

impl ChapterBuilder {
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
                | "svg"
                | "canvas"
                | "video"
                | "audio"
                | "img"
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
        let block = heading || matches!(name, "p" | "div" | "section" | "blockquote" | "li");
        if block {
            self.finish(TextBlockKind::Paragraph)?;
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
            self.finish(if heading {
                TextBlockKind::Heading
            } else {
                TextBlockKind::Paragraph
            })?;
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
        let mut block = TextBlock {
            id: String::new(),
            kind,
            spans: std::mem::take(&mut self.spans),
        };
        let text = block.plain_text();
        if text.trim().is_empty() && kind != TextBlockKind::Separator {
            return Ok(());
        }
        if self.blocks.len() >= MAX_BLOCKS {
            return Err(Error::ResponseTooLarge);
        }
        // FNV-1a is only an anchor, never a trust or integrity boundary.
        let hash = text.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
        let occurrence = self.occurrences.entry(hash).or_default();
        block.id = format!("{hash:016x}-{occurrence}");
        *occurrence += 1;
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
