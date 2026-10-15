use scraper::{ElementRef, Node};
use url::Url;

use crate::{Error, Result};

const MAX_BYTES: usize = 32 * 1024;
const MAX_DEPTH: usize = 64;

/// Convert a selected source document fragment to bounded, inert plain text.
pub(crate) fn extract(content: ElementRef<'_>, publication_url: &Url) -> Result<Option<String>> {
    let mut description = Description {
        text: String::new(),
        list_numbers: Vec::new(),
        publication_url,
    };
    description.element(content, 0)?;
    let mut lines = Vec::new();
    for line in description.text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() || lines.last().is_some_and(|line: &String| !line.is_empty()) {
            lines.push(line);
        }
    }
    let text = lines.join("\n").trim().to_owned();
    Ok((!text.is_empty()).then_some(text))
}

struct Description<'a> {
    text: String,
    list_numbers: Vec<Option<u64>>,
    publication_url: &'a Url,
}

impl Description<'_> {
    fn push(&mut self, value: &str) -> Result<()> {
        if value.len() > MAX_BYTES - self.text.len() {
            return Err(Error::ResponseTooLarge);
        }
        self.text.push_str(value);
        Ok(())
    }

    fn element(&mut self, element: ElementRef<'_>, depth: usize) -> Result<()> {
        if depth >= MAX_DEPTH {
            return Err(Error::ResponseTooLarge);
        }
        let name = element.value().name();
        if matches!(
            name,
            "script"
                | "style"
                | "noscript"
                | "iframe"
                | "object"
                | "embed"
                | "template"
                | "img"
                | "svg"
                | "video"
                | "audio"
                | "form"
                | "button"
        ) || element.attr("hidden").is_some()
            || element.attr("aria-hidden") == Some("true")
            || element
                .value()
                .classes()
                .any(|class| matches!(class, "ad" | "ads" | "advertisement"))
        {
            return Ok(());
        }
        if element.value().has_class(
            "fa-arrow-right",
            scraper::CaseSensitivity::AsciiCaseInsensitive,
        ) {
            return self.push(" → ");
        }
        if name == "br" {
            return self.push("\n");
        }
        let block = matches!(
            name,
            "div"
                | "p"
                | "center"
                | "blockquote"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "ul"
                | "ol"
                | "li"
                | "table"
                | "tr"
                | "hr"
        );
        if block {
            self.push("\n")?;
        }
        if matches!(name, "ul" | "ol") {
            self.list_numbers.push((name == "ol").then(|| {
                element
                    .attr("start")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1)
            }));
        }
        if name == "li" {
            if let Some(Some(number)) = self.list_numbers.last_mut() {
                let ordinal = element
                    .attr("value")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(*number);
                *number = ordinal.checked_add(1).ok_or(Error::ResponseTooLarge)?;
                self.push(&format!("{ordinal}. "))?;
            } else {
                self.push("- ")?;
            }
        }
        let label_start = self.text.len();
        for child in element.children() {
            match child.value() {
                Node::Text(value) => {
                    let normalized = value
                        .chars()
                        .map(|character| {
                            if character.is_whitespace() {
                                ' '
                            } else {
                                character
                            }
                        })
                        .collect::<String>();
                    self.push(&normalized)?;
                }
                Node::Element(_) => self.element(ElementRef::wrap(child).unwrap(), depth + 1)?,
                _ => {}
            }
        }
        if name == "a"
            && let Some(href) = element.attr("href")
            && let Ok(url) = self.publication_url.join(href)
            && matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
        {
            let label = self.text[label_start..].trim();
            if label != url.as_str() {
                let address = if label.is_empty() {
                    url.to_string()
                } else {
                    format!(" ({url})")
                };
                self.push(&address)?;
            }
        }
        if matches!(name, "ul" | "ol") {
            self.list_numbers.pop();
        }
        if block {
            self.push("\n")?;
        }
        Ok(())
    }
}
