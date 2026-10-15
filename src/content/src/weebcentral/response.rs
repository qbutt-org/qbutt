// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::validate_https_url;
use crate::{
    Category, Chapter, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result,
    Source, Transport, Variant, VariantRef, VariantSelector, Work,
};

use super::IMAGE_HOSTS;

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed Weeb Central selector")
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn source_id(origin: &Url, value: &str, kind: &str) -> Result<String> {
    let url = origin
        .join(value)
        .map_err(|_| Error::InvalidResponse("invalid Weeb Central URL"))?;
    validate_https_url(&url)?;
    let mut parts = url.path().trim_start_matches('/').split('/');
    if url.origin() != origin.origin() || url.query().is_some() || parts.next() != Some(kind) {
        return Err(Error::InvalidResponse("foreign Weeb Central identity"));
    }
    let id = parts
        .next()
        .ok_or(Error::InvalidResponse("missing Weeb Central identity"))?;
    if !crate::model::canonical_ulid(id) || (kind == "chapters" && parts.next().is_some()) {
        return Err(Error::InvalidResponse("invalid Weeb Central identity"));
    }
    Ok(id.into())
}

fn base_work(reference: PublicationRef, title: String, origin: &Url) -> Result<Work> {
    if title.is_empty() || title.len() > 2048 {
        return Err(Error::InvalidResponse("invalid Weeb Central title"));
    }
    let url = origin.join(&format!("/series/{}", reference.id)).unwrap();
    Ok(Work {
        reference,
        title,
        category: Category::Manga,
        url,
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants: Vec::new(),
        notes: Vec::new(),
        needs_inspection: true,
    })
}

pub(super) fn search_page(page: &str, origin: &Url) -> Result<Vec<Work>> {
    let document = Html::parse_fragment(page);
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for article in document
        .select(&selector("article"))
        .filter(|node| node.value().classes().any(|class| class == "bg-base-300"))
    {
        let link = article
            .select(&selector("a.line-clamp-1[href]"))
            .next()
            .ok_or(Error::InvalidResponse("missing Weeb Central search title"))?;
        let id = source_id(origin, link.attr("href").unwrap(), "series")?;
        if !seen.insert(id.clone()) || works.len() >= 100 {
            return Err(Error::InvalidResponse(
                "duplicate or excessive Weeb Central search results",
            ));
        }
        let mut work = base_work(
            PublicationRef {
                source: Source::WeebCentral,
                id,
            },
            text(link),
            origin,
        )?;
        work.cover = article
            .select(&selector("img[src]"))
            .next()
            .and_then(|img| img.attr("src"))
            .map(|src| {
                Url::parse(src).map_err(|_| Error::InvalidResponse("invalid Weeb Central cover"))
            })
            .transpose()?;
        if let Some(cover) = &work.cover {
            validate_image(cover)?;
        }
        works.push(work);
    }
    // Only the site's explicit empty-result state is a successful empty search.
    if works.is_empty()
        && !document
            .select(&selector("div[role=alert] span"))
            .any(|node| text(node) == "No results found")
    {
        return Err(Error::InvalidResponse(
            "unrecognized Weeb Central search response",
        ));
    }
    Ok(works)
}

pub(super) fn work_page(page: &str, origin: &Url, reference: &PublicationRef) -> Result<Work> {
    let document = Html::parse_document(page);
    let canonical = document
        .select(&selector("link[rel=canonical]"))
        .next()
        .and_then(|node| node.attr("href"))
        .ok_or(Error::InvalidResponse(
            "missing Weeb Central publication identity",
        ))?;
    if source_id(origin, canonical, "series")? != reference.id {
        return Err(Error::VariantUnavailable);
    }
    let title = document
        .select(&selector("h1"))
        .next()
        .map(text)
        .ok_or(Error::InvalidResponse(
            "missing Weeb Central publication title",
        ))?;
    let mut work = base_work(reference.clone(), title, origin)?;
    for item in document.select(&selector("li")) {
        let Some(label) = item.select(&selector("strong")).next().map(text) else {
            continue;
        };
        match label.as_str() {
            "Released:" => {
                work.year = item
                    .text()
                    .collect::<String>()
                    .split_whitespace()
                    .last()
                    .and_then(|year| year.parse().ok());
            }
            "Description" => work.description = item.select(&selector("p")).next().map(text),
            "Associated Name(s)" => {
                work.aliases = item.select(&selector("ul li")).map(text).collect()
            }
            _ => {}
        }
    }
    work.cover = document
        .select(&selector("meta[property='og:image']"))
        .next()
        .and_then(|node| node.attr("content"))
        .map(|src| {
            Url::parse(src).map_err(|_| Error::InvalidResponse("invalid Weeb Central cover"))
        })
        .transpose()?;
    if let Some(cover) = &work.cover {
        validate_image(cover)?;
    }
    work.variants.push(Variant {
        reference: VariantRef {
            publication: reference.clone(),
            selector: VariantSelector::WeebCentralPages,
        },
        title: None,
        transport: Transport::Manga,
        episode: None,
        quality: None,
        translation: None,
        languages: vec!["en".into()],
        player: None,
        size: None,
        seeders: None,
        info_hash: None,
        reported_video_ads: false,
    });
    work.needs_inspection = false;
    Ok(work)
}

pub(super) fn chapter_list(
    page: &str,
    origin: &Url,
    reference: &VariantRef,
) -> Result<Vec<Chapter>> {
    let document = Html::parse_fragment(page);
    let mut chapters = Vec::new();
    let mut seen = HashSet::new();
    for link in document.select(&selector("a.flex-1[href]")) {
        let id = source_id(origin, link.attr("href").unwrap(), "chapters")?;
        if !seen.insert(id.clone()) || chapters.len() >= 10_000 {
            return Err(Error::InvalidResponse(
                "duplicate or excessive Weeb Central chapters",
            ));
        }
        let label = link
            .select(&selector("span.grow > span"))
            .next()
            .map(text)
            .ok_or(Error::InvalidResponse("missing Weeb Central chapter title"))?;
        if label.is_empty() || label.len() > 2048 {
            return Err(Error::InvalidResponse("invalid Weeb Central chapter title"));
        }
        let number = label.strip_prefix("Chapter ").map(str::to_owned);
        let title = number.is_none().then_some(label);
        chapters.push(Chapter {
            reference: ChapterRef {
                variant: reference.clone(),
                id,
            },
            title,
            volume: None,
            number,
            pages: 0,
        });
    }
    if chapters.is_empty() {
        return Err(Error::VariantUnavailable);
    }
    // Weeb Central's full list is newest first; preserve its ordering for
    // nonnumeric specials instead of guessing a chapter number comparator.
    chapters.reverse();
    Ok(chapters)
}

pub(super) fn reader_identity(page: &str, origin: &Url, selected: &ChapterRef) -> Result<()> {
    let document = Html::parse_document(page);
    let canonical = document
        .select(&selector("link[rel=canonical]"))
        .next()
        .and_then(|node| node.attr("href"))
        .ok_or(Error::InvalidResponse(
            "missing Weeb Central chapter identity",
        ))?;
    if source_id(origin, canonical, "chapters")? != selected.id {
        return Err(Error::VariantUnavailable);
    }
    let mut found = false;
    for link in document.select(&selector("a.col-span-6[href]")) {
        if source_id(origin, link.attr("href").unwrap(), "series")?
            != selected.variant.publication.id
        {
            return Err(Error::VariantUnavailable);
        }
        found = true;
    }
    if !found {
        return Err(Error::InvalidResponse(
            "missing Weeb Central chapter publication",
        ));
    }
    Ok(())
}

pub(super) fn image_list(page: &str, reference: &ChapterRef) -> Result<ChapterPages> {
    let document = Html::parse_fragment(page);
    let section = document
        .select(&selector("section#chapter-images"))
        .next()
        .ok_or(Error::InvalidResponse(
            "missing Weeb Central chapter images",
        ))?;
    let mut images = Vec::new();
    let mut seen = HashSet::new();
    for img in section.select(&selector("img")) {
        if img.attr("alt") != Some(format!("Page {}", images.len() + 1).as_str())
            || images.len() >= 2000
        {
            return Err(Error::InvalidResponse("invalid Weeb Central page order"));
        }
        let url = img
            .attr("src")
            .and_then(|src| Url::parse(src).ok())
            .ok_or(Error::InvalidResponse("invalid Weeb Central page URL"))?;
        validate_image(&url)?;
        if !seen.insert(url.clone()) {
            return Err(Error::InvalidResponse("duplicate Weeb Central page"));
        }
        images.push(ChapterImage { url });
    }
    if images.is_empty() {
        return Err(Error::VariantUnavailable);
    }
    Ok(ChapterPages {
        reference: reference.clone(),
        images,
    })
}

pub(super) fn validate_image(url: &Url) -> Result<()> {
    validate_https_url(url)?;
    if !url
        .host_str()
        .is_some_and(|host| IMAGE_HOSTS.contains(&host))
        || url.port().is_some()
    {
        return Err(Error::UnsafeAddress);
    }
    Ok(())
}
