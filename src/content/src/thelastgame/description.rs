use scraper::ElementRef;

use crate::{Error, Result};

use super::{selector, text};

const MAX_BYTES: usize = 16_000;
const MAX_DEPTH: usize = 32;

/// Article reports and instructions remain publication-wide plain text. They
/// do not prove that every attachment has the same edition or languages.
pub(super) fn extract(entry: ElementRef<'_>) -> Result<Option<String>> {
    let mut sections = Vec::new();
    if let Some(introduction) = entry
        .children()
        .filter_map(ElementRef::wrap)
        .find(|element| element.value().name() == "p")
    {
        sections.push(plain_text(introduction)?);
    }
    for block in entry.children().filter_map(ElementRef::wrap) {
        if block.value().name() != "div" {
            continue;
        }
        let heading = block.select(&selector("strong")).next().map(text);
        if !matches!(heading.as_deref(), Some("Информация о игре" | "Важно")) {
            continue;
        }
        let content = plain_text(block)?;
        if Some(content.as_str()) != heading.as_deref() {
            sections.push(content);
        }
    }
    sections.retain(|section| !section.is_empty());
    let description = sections.join("\n\n");
    if description.len() > MAX_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    Ok((!description.is_empty()).then_some(description))
}

fn plain_text(element: ElementRef<'_>) -> Result<String> {
    let mut output = String::new();
    append(element, &mut output, 0)?;
    Ok(output
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n"))
}

fn append(element: ElementRef<'_>, output: &mut String, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::InvalidResponse(
            "TheLastGame instructions are too deeply nested",
        ));
    }
    let name = element.value().name();
    if matches!(
        name,
        "script"
            | "style"
            | "noscript"
            | "iframe"
            | "img"
            | "svg"
            | "video"
            | "audio"
            | "nav"
            | "form"
            | "button"
    ) || element.value().classes().any(|class| {
        matches!(
            class,
            "ad" | "ads" | "advertisement" | "navigation" | "comments" | "btn_green" | "btn_blue"
        )
    }) || element.attr("hidden").is_some()
        || element.attr("aria-hidden") == Some("true")
    {
        return Ok(());
    }
    let block = matches!(
        name,
        "div" | "p" | "center" | "h2" | "h3" | "h4" | "ol" | "ul" | "li"
    );
    if block || name == "br" {
        output.push('\n');
    }
    if name == "li" {
        let ordered = element
            .parent()
            .and_then(ElementRef::wrap)
            .is_some_and(|parent| parent.value().name() == "ol");
        if ordered {
            let index = element
                .prev_siblings()
                .filter_map(ElementRef::wrap)
                .filter(|sibling| sibling.value().name() == "li")
                .count()
                + 1;
            output.push_str(&format!("{index}. "));
        } else {
            output.push_str("- ");
        }
    }
    for child in element.children() {
        if let Some(child) = ElementRef::wrap(child) {
            append(child, output, depth + 1)?;
        } else if let Some(content) = child.value().as_text() {
            for character in content.chars() {
                output.push(if character.is_whitespace() {
                    ' '
                } else {
                    character
                });
                if output.len() > MAX_BYTES {
                    return Err(Error::ResponseTooLarge);
                }
            }
        }
    }
    if block {
        output.push('\n');
    }
    if output.len() > MAX_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    Ok(())
}
