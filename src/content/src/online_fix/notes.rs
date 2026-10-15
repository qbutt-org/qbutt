use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::{Error, PublicationNote, Result, html_text};

pub(super) fn extract(document: &Html, publication_url: &Url) -> Result<Vec<PublicationNote>> {
    let bodies = Selector::parse("[id^='comm-id-']").expect("fixed comment body selector");
    let authors = Selector::parse(".group.author > a").expect("fixed comment author selector");
    let links = Selector::parse(".comment-link > a[href]").expect("fixed comment link selector");
    let mut notes = Vec::new();
    for body in document.select(&bodies) {
        if notes.len() >= 100 {
            return Err(Error::ResponseTooLarge);
        }
        let id = body.attr("id").unwrap().strip_prefix("comm-id-").unwrap();
        if !crate::model::canonical_decimal(id) {
            return Err(Error::InvalidResponse("invalid Online-Fix comment ID"));
        }
        let anchor = format!("comment-id-{id}");
        let container = body
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|element| element.attr("id") == Some(anchor.as_str()))
            .ok_or(Error::InvalidResponse("missing Online-Fix comment owner"))?;
        let Some(text) = html_text::extract(body, publication_url)? else {
            continue;
        };
        let author = container
            .select(&authors)
            .next()
            .map(|element| {
                element
                    .text()
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|author| !author.is_empty());
        if author.as_ref().is_some_and(|author| author.len() > 512) {
            return Err(Error::ResponseTooLarge);
        }
        let url = container
            .select(&links)
            .filter_map(|link| publication_url.join(link.attr("href").unwrap()).ok())
            .find(|url| {
                url.fragment() == Some(anchor.as_str()) && same_publication(url, publication_url)
            });
        notes.push(PublicationNote { text, author, url });
    }
    Ok(notes)
}

fn same_publication(url: &Url, publication_url: &Url) -> bool {
    if url.origin() != publication_url.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
    {
        return false;
    }
    if url.path() == publication_url.path() {
        return true;
    }
    let Some((directory, filename)) = publication_url.path().rsplit_once('/') else {
        return false;
    };
    let Some(page) = url.path().strip_prefix(&format!("{directory}/page,1,")) else {
        return false;
    };
    let Some((number, linked_filename)) = page.split_once(',') else {
        return false;
    };
    linked_filename == filename && crate::model::canonical_decimal(number)
}
