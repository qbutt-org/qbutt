use scraper::{ElementRef, Selector};
use url::Url;

use crate::{Error, Result, html_text};

pub(super) fn description(
    content: ElementRef<'_>,
    publication_url: &Url,
) -> Result<Option<String>> {
    let mut sections = Vec::new();
    for element in content.children().filter_map(ElementRef::wrap) {
        let heading = text(element);
        if element.value().name() == "h3" && heading == "Repack Features" {
            if let Some(list) = element.next_siblings().find_map(ElementRef::wrap)
                && list.value().name() == "ul"
                && let Some(body) = html_text::extract(list, publication_url)?
            {
                sections.push(format!("{heading}\n{body}"));
            }
        } else if element
            .value()
            .has_class("su-spoiler", scraper::CaseSensitivity::CaseSensitive)
        {
            let titles =
                Selector::parse(".su-spoiler-title").expect("fixed spoiler title selector");
            let bodies =
                Selector::parse(".su-spoiler-content").expect("fixed spoiler body selector");
            let title = element.select(&titles).next().map(text).unwrap_or_default();
            if (title == "Game Description" || title.starts_with("Selective Download"))
                && let Some(body) = element.select(&bodies).next()
                && let Some(body) = html_text::extract(body, publication_url)?
            {
                sections.push(format!("{title}\n{body}"));
            }
        } else if element.value().name() == "p" && heading == "Backwards Compatibility" {
            let mut section = vec![heading];
            for sibling in element.next_siblings().filter_map(ElementRef::wrap) {
                if !matches!(sibling.value().name(), "p" | "ul" | "ol") {
                    break;
                }
                if let Some(body) = html_text::extract(sibling, publication_url)? {
                    section.push(body);
                }
            }
            sections.push(section.join("\n"));
        }
    }
    let description = sections.join("\n\n");
    if description.len() > 32 * 1024 {
        return Err(Error::ResponseTooLarge);
    }
    Ok((!description.is_empty()).then_some(description))
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
