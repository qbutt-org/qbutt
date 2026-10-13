use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::model::canonical_decimal;
use crate::torrent::magnet;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, Transport, Variant,
    VariantRef, VariantSelector, Work,
};

use super::taxonomy::Categories;

pub(crate) struct Release {
    pub(crate) work: Work,
    magnet: Url,
}

impl Release {
    pub(crate) fn resolve(self, selected: &VariantRef) -> Result<ResolvedContent> {
        if self.work.variants[0].reference != *selected {
            return Err(Error::VariantUnavailable);
        }
        Ok(ResolvedContent::Torrent {
            reference: selected.clone(),
            title: self.work.title,
            magnet: self.magnet,
        })
    }
}

pub(crate) fn search(
    body: &str,
    origin: &Url,
    category: Category,
    categories: &Categories,
) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let Some(table) = optional(document.root_element(), "#tor-tbl")? else {
        let empty = unique(document.root_element(), "#search-results")?;
        if text(empty).to_lowercase().contains("не найден") {
            return Ok(Vec::new());
        }
        return Err(Error::InvalidResponse("missing RuTracker results"));
    };
    let mut seen = HashSet::new();
    let mut works = Vec::new();
    for (index, row) in table.select(&selector("tbody > tr")).enumerate() {
        if index >= 500 {
            return Err(Error::InvalidResponse("excessive RuTracker results"));
        }
        if !row
            .child_elements()
            .any(|child| child.value().name() == "td")
        {
            continue;
        }
        let title = unique(row, "td.t-title-col a.tLink")?;
        let reference = publication(required(title, "href")?, origin, "viewtopic.php")?;
        if required(title, "data-topic_id")? != reference.id
            || row.attr("id") != Some(format!("trs-tr-{}", reference.id).as_str())
            || !seen.insert(reference.clone())
        {
            return Err(Error::InvalidResponse("RuTracker topic identity differs"));
        }
        let forum = unique(row, "a[href*='tracker.php?f=']")?;
        let id = forum_id(required(forum, "href")?, origin, "tracker.php")?;
        let actual = categories
            .get(&id)
            .ok_or(Error::InvalidResponse("unknown RuTracker forum"))?;
        if *actual != Some(category) {
            continue;
        }
        works.push(work(
            reference,
            origin,
            category,
            text(title),
            Vec::new(),
            true,
        )?);
    }
    Ok(works)
}

pub(crate) fn release(
    body: &str,
    origin: &Url,
    expected: &PublicationRef,
    categories: &Categories,
) -> Result<Release> {
    expected.rutracker_id()?;
    let document = Html::parse_document(body);
    let title = unique(document.root_element(), "#topic-title")?;
    if publication(required(title, "href")?, origin, "viewtopic.php")? != *expected {
        return Err(Error::VariantUnavailable);
    }
    let attachment = unique(document.root_element(), "table.attach")?;
    let download = unique(attachment, "a.dl-stub")?;
    if publication(required(download, "href")?, origin, "dl.php")? != *expected {
        return Err(Error::VariantUnavailable);
    }
    let Some(link) = optional(attachment, "a.magnet-link")? else {
        return Err(Error::UnsupportedTorrent);
    };
    let (magnet, hash) = magnet(required(link, "href")?)?;
    let size = number(required(unique(attachment, "#tor-size-humn")?, "title")?)?;
    let seeders = optional(document.root_element(), "#t-tor-stats span.seed > b")?
        .map(|count| number(&text(count)))
        .transpose()?;
    let mut category = None;
    for link in document.select(&selector(".nav a[href], a.nav[href]")) {
        let href = required(link, "href")?;
        let url = site_url(href, origin)?;
        if url.path() != "/forum/viewforum.php" {
            continue;
        }
        let id = forum_id(href, origin, "viewforum.php")?;
        category = *categories
            .get(&id)
            .ok_or(Error::InvalidResponse("unknown RuTracker topic forum"))?;
    }
    let category = category.ok_or(Error::InvalidResponse("missing RuTracker topic category"))?;
    let title = text(title);
    let variant = Variant {
        reference: VariantRef {
            publication: expected.clone(),
            selector: VariantSelector::TorrentHash {
                info_hash: hash.clone(),
            },
        },
        title: Some(title.clone()),
        transport: Transport::Torrent,
        episode: None,
        quality: None,
        translation: None,
        languages: Vec::new(),
        player: None,
        size: Some(size),
        seeders,
        info_hash: Some(hash),
        reported_video_ads: false,
    };
    let mut work = work(
        expected.clone(),
        origin,
        category,
        title,
        vec![variant],
        false,
    )?;
    let post = document
        .select(&selector(".post_body"))
        .next()
        .ok_or(Error::InvalidResponse("missing RuTracker publication body"))?;
    let description = text(post);
    if !description.is_empty() {
        work.description = Some(description.chars().take(16_000).collect());
    }
    Ok(Release { work, magnet })
}

pub(crate) fn topic_url(origin: &Url, publication: &PublicationRef) -> Url {
    origin
        .join(&format!("/forum/viewtopic.php?t={}", publication.id))
        .expect("canonical topic ID")
}

fn work(
    reference: PublicationRef,
    origin: &Url,
    category: Category,
    title: String,
    variants: Vec<Variant>,
    needs_inspection: bool,
) -> Result<Work> {
    if title.is_empty() || title.len() > 4096 {
        return Err(Error::InvalidResponse("invalid RuTracker topic title"));
    }
    Ok(Work {
        url: topic_url(origin, &reference),
        reference,
        title,
        category,
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants,
        needs_inspection,
    })
}

fn publication(href: &str, origin: &Url, path: &str) -> Result<PublicationRef> {
    let url = site_url(href, origin)?;
    if url.path() != format!("/forum/{path}") {
        return Err(Error::InvalidResponse("invalid RuTracker publication path"));
    }
    let mut fields = url.query_pairs();
    let Some((name, id)) = fields.next() else {
        return Err(Error::InvalidResponse("missing RuTracker topic ID"));
    };
    if name != "t" || fields.next().is_some() || !canonical_decimal(&id) {
        return Err(Error::InvalidResponse("invalid RuTracker topic ID"));
    }
    let reference = PublicationRef {
        source: Source::RuTracker,
        id: id.into_owned(),
    };
    reference.rutracker_id()?;
    Ok(reference)
}

pub(super) fn forum_id(href: &str, origin: &Url, path: &str) -> Result<u64> {
    let url = site_url(href, origin)?;
    let mut fields = url.query_pairs();
    let Some((name, id)) = fields.next() else {
        return Err(Error::InvalidResponse("missing RuTracker forum ID"));
    };
    if url.path() != format!("/forum/{path}") || name != "f" {
        return Err(Error::InvalidResponse("invalid RuTracker forum URL"));
    }
    if let Some((name, _)) = fields.next()
        && (path != "tracker.php" || name != "nm" || fields.next().is_some())
    {
        return Err(Error::InvalidResponse("invalid RuTracker forum URL"));
    }
    decimal(&id)
}

fn site_url(href: &str, origin: &Url) -> Result<Url> {
    let base = origin.join("/forum/").unwrap();
    let url = base
        .join(href)
        .map_err(|_| Error::InvalidResponse("invalid RuTracker link"))?;
    if url.origin() != origin.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidResponse("foreign RuTracker link"));
    }
    Ok(url)
}

pub(super) fn decimal(value: &str) -> Result<u64> {
    if !canonical_decimal(value) {
        return Err(Error::InvalidResponse("invalid RuTracker numeric identity"));
    }
    value
        .parse()
        .map_err(|_| Error::InvalidResponse("RuTracker numeric identity overflow"))
}

fn number(value: &str) -> Result<u64> {
    value
        .parse()
        .map_err(|_| Error::InvalidResponse("invalid RuTracker count"))
}

pub(super) fn required<'a>(element: ElementRef<'a>, attribute: &str) -> Result<&'a str> {
    element
        .attr(attribute)
        .ok_or(Error::InvalidResponse("missing RuTracker attribute"))
}

pub(super) fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed RuTracker selector")
}

pub(super) fn unique<'a>(parent: ElementRef<'a>, value: &str) -> Result<ElementRef<'a>> {
    optional(parent, value)?.ok_or(Error::InvalidResponse("missing RuTracker metadata"))
}

fn optional<'a>(parent: ElementRef<'a>, value: &str) -> Result<Option<ElementRef<'a>>> {
    let selector = selector(value);
    let mut elements = parent.select(&selector);
    let first = elements.next();
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous RuTracker metadata"));
    }
    Ok(first)
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}
