use std::fmt;

use pulldown_cmark::{Event, Options, Parser, html};
use scraper::{ElementRef, Html, Node};
use url::Url;

use crate::{Error, Result};

const MAX_DESCRIPTION_BYTES: usize = 32 * 1024;
const MAX_MARKUP_BYTES: usize = 128 * 1024;
const MAX_DEPTH: usize = 64;

/// The intermediate DOM is never displayed or executed. Work.description and
/// WorkDetails remain plain text, including image captions and link addresses.
pub(super) fn plain_text(markdown: &str, publication_url: &Url) -> Result<Option<String>> {
    if markdown.len() > MAX_DESCRIPTION_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    let markdown = markdown.trim();
    if markdown.is_empty() || markdown == "#### No description." {
        return Ok(None);
    }

    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(markdown, options).map(|event| match event {
        Event::TaskListMarker(true) => Event::Text("✓ ".into()),
        Event::TaskListMarker(false) => Event::Text("□ ".into()),
        event => event,
    });
    let mut markup = BoundedText::new(MAX_MARKUP_BYTES);
    html::write_html_fmt(&mut markup, events).map_err(|_| Error::ResponseTooLarge)?;

    let document = Html::parse_fragment(&markup.value);
    let mut description = PlainDescription {
        text: BoundedText::new(MAX_DESCRIPTION_BYTES),
        list_numbers: Vec::new(),
        publication_url,
    };
    description.element(document.root_element(), 0, false)?;
    let text = description.text.value.trim();
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

struct BoundedText {
    value: String,
    limit: usize,
}

impl BoundedText {
    fn new(limit: usize) -> Self {
        Self {
            value: String::new(),
            limit,
        }
    }

    fn push(&mut self, value: &str) -> Result<()> {
        if value.len() > self.limit - self.value.len() {
            return Err(Error::ResponseTooLarge);
        }
        self.value.push_str(value);
        Ok(())
    }

    fn separate(&mut self, lines: usize) -> Result<()> {
        if self.value.is_empty() {
            return Ok(());
        }
        let existing = self
            .value
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\n')
            .count();
        for _ in existing..lines {
            self.push("\n")?;
        }
        Ok(())
    }
}

impl fmt::Write for BoundedText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.push(value).map_err(|_| fmt::Error)
    }
}

struct PlainDescription<'a> {
    text: BoundedText,
    list_numbers: Vec<Option<u64>>,
    publication_url: &'a Url,
}

impl PlainDescription<'_> {
    fn element(&mut self, element: ElementRef<'_>, depth: usize, preformatted: bool) -> Result<()> {
        if depth >= MAX_DEPTH {
            return Err(Error::ResponseTooLarge);
        }
        match element.value().name() {
            "script" | "style" | "iframe" | "object" | "embed" | "template" => {}
            "br" => self.text.separate(1)?,
            "hr" => self.text.separate(2)?,
            "img" => {
                let start = self.text.value.len();
                self.text.push(element.attr("alt").unwrap_or_default())?;
                if let Some(source) = element.attr("src") {
                    self.destination(source, start)?;
                }
            }
            "a" => {
                let start = self.text.value.len();
                self.children(element, depth, preformatted)?;
                if let Some(destination) = element.attr("href") {
                    self.destination(destination, start)?;
                }
            }
            "ul" | "ol" => {
                self.text
                    .separate(if self.list_numbers.is_empty() { 2 } else { 1 })?;
                let number = (element.value().name() == "ol").then(|| {
                    element
                        .attr("start")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(1)
                });
                self.list_numbers.push(number);
                self.children(element, depth, preformatted)?;
                self.list_numbers.pop();
                self.text
                    .separate(if self.list_numbers.is_empty() { 2 } else { 1 })?;
            }
            "li" => {
                self.text.separate(1)?;
                self.text
                    .push(&"  ".repeat(self.list_numbers.len().saturating_sub(1)))?;
                if let Some(Some(number)) = self.list_numbers.last_mut() {
                    self.text.push(&format!("{number}. "))?;
                    *number = number.checked_add(1).ok_or(Error::ResponseTooLarge)?;
                } else {
                    self.text.push("- ")?;
                }
                self.children(element, depth, preformatted)?;
                self.text.separate(1)?;
            }
            "tr" => {
                self.text.separate(1)?;
                let mut first = true;
                for cell in element.child_elements() {
                    if matches!(cell.value().name(), "td" | "th") {
                        if !first {
                            self.text.push("\t")?;
                        }
                        self.children(cell, depth + 1, preformatted)?;
                        first = false;
                    }
                }
                self.text.separate(1)?;
            }
            "p" => {
                self.children(element, depth, preformatted)?;
                self.text
                    .separate(if self.list_numbers.is_empty() { 2 } else { 1 })?;
            }
            "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "div" | "table" => {
                self.text.separate(2)?;
                self.children(
                    element,
                    depth,
                    preformatted || element.value().name() == "pre",
                )?;
                self.text.separate(2)?;
            }
            _ => self.children(element, depth, preformatted)?,
        }
        Ok(())
    }

    fn children(
        &mut self,
        element: ElementRef<'_>,
        depth: usize,
        preformatted: bool,
    ) -> Result<()> {
        for child in element.children() {
            match child.value() {
                Node::Text(value) => {
                    if preformatted
                        || !value.trim().is_empty()
                        || (!self.text.value.is_empty() && !self.text.value.ends_with('\n'))
                    {
                        self.text.push(value)?;
                    }
                }
                Node::Element(_) => {
                    self.element(ElementRef::wrap(child).unwrap(), depth + 1, preformatted)?
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn destination(&mut self, value: &str, label_start: usize) -> Result<()> {
        let Ok(url) = self.publication_url.join(value) else {
            return Ok(());
        };
        if !matches!(url.scheme(), "http" | "https" | "mailto" | "magnet") {
            return Ok(());
        }
        let label = self.text.value[label_start..].trim();
        if label == url.as_str() {
            return Ok(());
        }
        if label.is_empty() {
            self.text.push(url.as_str())?;
        } else {
            self.text.push(" (")?;
            self.text.push(url.as_str())?;
            self.text.push(")")?;
        }
        Ok(())
    }
}
