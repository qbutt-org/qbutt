use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::validate_https_url;
use crate::torrent::magnet;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, Transport, Variant,
    VariantRef, VariantSelector, Work,
};

pub(crate) struct Release {
    pub(crate) work: Work,
    magnet: Url,
}

struct TorrentMetadata {
    title: String,
    hash: String,
    size: u64,
    seeders: u64,
}

impl Release {
    pub(crate) fn resolve(self, selected: &VariantRef) -> Result<ResolvedContent> {
        if self.work.reference != selected.publication
            || self.work.variants[0].reference != *selected
        {
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
    query: &str,
    route: &str,
    category: Category,
) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let feed = unique(
        document.root_element(),
        "link[rel='alternate'][type='application/rss+xml']",
    )?;
    let feed_url = public_url(
        feed.attr("href")
            .ok_or(Error::InvalidResponse("missing catalog identity"))?,
        origin,
    )?;
    let fields = feed_url.query_pairs().collect::<Vec<_>>();
    for (name, expected) in [("page", "rss"), ("q", query), ("c", route), ("f", "0")] {
        let mut values = fields.iter().filter(|(key, _)| key == name);
        if values.next().map(|(_, value)| value.as_ref()) != Some(expected)
            || values.next().is_some()
        {
            return Err(Error::InvalidResponse("search identity was not confirmed"));
        }
    }
    if feed_url.path() != "/" {
        return Err(Error::InvalidResponse("invalid catalog identity"));
    }

    let table_selector = Selector::parse("table.torrent-list").expect("fixed results selector");
    let mut tables = document.select(&table_selector);
    let Some(table) = tables.next() else {
        let empty = Selector::parse(".container > h3").expect("fixed empty results selector");
        if document
            .select(&empty)
            .any(|heading| text(heading) == "No results found")
        {
            return Ok(Vec::new());
        }
        return Err(Error::InvalidResponse("missing search results"));
    };
    if tables.next().is_some()
        || text(unique(table, "thead th.hdr-name")?) != "Name"
        || text(unique(table, "thead th.hdr-size")?) != "Size"
        || unique(table, "thead th.hdr-seeders")?.attr("title") != Some("Seeders")
    {
        return Err(Error::InvalidResponse("invalid search results table"));
    }
    let rows = Selector::parse("tbody > tr").expect("fixed results row selector");
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for (index, row) in table.select(&rows).enumerate() {
        if index >= 100 {
            return Err(Error::InvalidResponse("excessive torrent rows"));
        }
        let cells = row.child_elements().collect::<Vec<_>>();
        if cells.len() != 8 || cells.iter().any(|cell| cell.value().name() != "td") {
            return Err(Error::InvalidResponse("invalid torrent row"));
        }
        let reported = category_code(unique(cells[0], "a[href]")?.attr("href").unwrap(), origin)?;
        if reported.split_once('_').map(|(main, _)| main)
            != route.split_once('_').map(|(main, _)| main)
            || (matches!(route, "6_1" | "6_2") && reported != route)
        {
            return Err(Error::InvalidResponse(
                "torrent category differs from search",
            ));
        }
        let (actual, language) = match site_category(&reported)? {
            Some(mapped) => mapped,
            // Nyaa explicitly identifies anime music videos.
            None if reported == "1_1" => continue,
            None => return Err(Error::InvalidResponse("unsupported search category")),
        };
        if actual != category {
            return Err(Error::InvalidResponse(
                "torrent category differs from search",
            ));
        }
        let title = unique(cells[1], "a[href^='/view/']:not(.comments)")?;
        let reference = publication(title.attr("href").unwrap(), origin, "view", "")?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate torrent publication"));
        }
        let download = unique(cells[2], "a[href^='/download/']")?;
        if publication(
            download.attr("href").unwrap(),
            origin,
            "download",
            ".torrent",
        )? != reference
        {
            return Err(Error::InvalidResponse("torrent download identity differs"));
        }
        let (_, hash) = magnet(
            unique(cells[2], "a[href^='magnet:']")?
                .attr("href")
                .unwrap(),
        )?;
        works.push(work(
            reference,
            origin,
            category,
            language,
            TorrentMetadata {
                title: text(title),
                hash,
                size: size_bytes(&text(cells[3]))?,
                seeders: number(&text(cells[5]))?,
            },
            true,
        )?);
    }
    Ok(works)
}

pub(crate) fn release(body: &str, origin: &Url, expected: &PublicationRef) -> Result<Release> {
    expected.nyaa_id()?;
    let document = Html::parse_document(body);
    let footer = unique(
        document.root_element(),
        ".container > .panel > .panel-footer",
    )?;
    let panel = footer
        .parent()
        .and_then(ElementRef::wrap)
        .ok_or(Error::InvalidResponse("missing publication panel"))?;
    let title = text(unique(panel, ".panel-heading > h3.panel-title")?);
    let download = unique(footer, "a[href^='/download/']")?;
    if publication(
        download.attr("href").unwrap(),
        origin,
        "download",
        ".torrent",
    )? != *expected
    {
        return Err(Error::VariantUnavailable);
    }
    let (magnet, hash) = magnet(unique(footer, "a[href^='magnet:']")?.attr("href").unwrap())?;
    let rows = Selector::parse(".panel-body > .row").expect("fixed metadata rows selector");
    let mut category = None;
    let mut size = None;
    let mut seeders = None;
    let mut reported_hash = None;
    for row in panel.select(&rows) {
        let cells = row.child_elements().collect::<Vec<_>>();
        if cells.len() % 2 != 0 {
            return Err(Error::InvalidResponse("invalid publication metadata row"));
        }
        for pair in cells.chunks_exact(2) {
            match text(pair[0]).as_str() {
                "Category:" => {
                    let links = Selector::parse("a[href]").expect("fixed category links selector");
                    let codes = pair[1]
                        .select(&links)
                        .map(|link| category_code(link.attr("href").unwrap(), origin))
                        .collect::<Result<Vec<_>>>()?;
                    if codes.len() != 2
                        || codes[0].split_once('_').map(|(main, _)| main)
                            != codes[1].split_once('_').map(|(main, _)| main)
                        || !codes[0].ends_with("_0")
                    {
                        return Err(Error::InvalidResponse("ambiguous publication category"));
                    }
                    let actual = site_category(&codes[1])?.ok_or(Error::VariantUnavailable)?;
                    if category.replace(actual).is_some() {
                        return Err(Error::InvalidResponse("ambiguous publication category"));
                    }
                }
                "File size:" => {
                    let previous = size.replace(size_bytes(&text(pair[1]))?);
                    if previous.is_some() {
                        return Err(Error::InvalidResponse("ambiguous publication size"));
                    }
                }
                "Seeders:" => {
                    let previous = seeders.replace(number(&text(pair[1]))?);
                    if previous.is_some() {
                        return Err(Error::InvalidResponse("ambiguous seed count"));
                    }
                }
                "Info hash:" => {
                    let previous =
                        reported_hash.replace(text(unique(pair[1], "kbd")?).to_ascii_lowercase());
                    if previous.is_some() {
                        return Err(Error::InvalidResponse("ambiguous publication hash"));
                    }
                }
                _ => {}
            }
        }
    }
    if reported_hash.as_deref() != Some(&hash) {
        return Err(Error::InvalidResponse(
            "publication hash differs from magnet",
        ));
    }
    let (category, language) =
        category.ok_or(Error::InvalidResponse("missing publication category"))?;
    let mut work = work(
        expected.clone(),
        origin,
        category,
        language,
        TorrentMetadata {
            title,
            hash,
            size: size.ok_or(Error::InvalidResponse("missing publication size"))?,
            seeders: seeders.ok_or(Error::InvalidResponse("missing seed count"))?,
        },
        false,
    )?;
    let description = unique(document.root_element(), "#torrent-description")?
        .text()
        .collect::<String>();
    work.description = super::description::plain_text(&description, &work.url)?;
    Ok(Release { work, magnet })
}

fn public_url(input: &str, origin: &Url) -> Result<Url> {
    let url = origin.join(input).map_err(|_| Error::UnsafeAddress)?;
    validate_https_url(&url)?;
    if url.origin() != origin.origin() {
        return Err(Error::UnsafeAddress);
    }
    Ok(url)
}

fn publication(input: &str, origin: &Url, prefix: &str, suffix: &str) -> Result<PublicationRef> {
    let url = public_url(input, origin)?;
    let id = url
        .path()
        .strip_prefix(&format!("/{prefix}/"))
        .and_then(|id| id.strip_suffix(suffix))
        .ok_or(Error::InvalidReference)?;
    let reference = PublicationRef {
        source: Source::Nyaa,
        id: id.into(),
    };
    reference.validate()?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(Error::InvalidReference);
    }
    Ok(reference)
}

fn category_code(input: &str, origin: &Url) -> Result<String> {
    let url = public_url(input, origin)?;
    let fields = url.query_pairs().collect::<Vec<_>>();
    if url.path() != "/" || fields.len() != 1 || fields[0].0 != "c" {
        return Err(Error::InvalidResponse("invalid category identity"));
    }
    Ok(fields[0].1.to_string())
}

fn site_category(code: &str) -> Result<Option<(Category, Option<&'static str>)>> {
    let mapped = match code {
        "1_2" => (Category::Anime, Some("en")),
        "1_3" | "1_4" => (Category::Anime, None),
        "3_1" => (Category::Books, Some("en")),
        "3_2" | "3_3" => (Category::Books, None),
        "6_1" => (Category::Software, None),
        "6_2" => (Category::Games, None),
        "1_1" | "4_1" | "4_2" | "4_3" | "4_4" => return Ok(None),
        _ => return Err(Error::InvalidResponse("invalid category identity")),
    };
    Ok(Some(mapped))
}

fn work(
    reference: PublicationRef,
    origin: &Url,
    category: Category,
    language: Option<&str>,
    metadata: TorrentMetadata,
    needs_inspection: bool,
) -> Result<Work> {
    let TorrentMetadata {
        title,
        hash,
        size,
        seeders,
    } = metadata;
    if title.is_empty() || title.len() > 2048 {
        return Err(Error::InvalidResponse("invalid publication title"));
    }
    let url = origin
        .join(&format!("/view/{}", reference.id))
        .expect("canonical publication path");
    let variants = vec![Variant {
        reference: VariantRef {
            publication: reference.clone(),
            selector: VariantSelector::TorrentHash {
                info_hash: hash.clone(),
            },
        },
        episode: (category == Category::Anime)
            .then(|| episode(&title))
            .flatten(),
        quality: (category == Category::Anime)
            .then(|| quality(&title))
            .flatten(),
        title: Some(title.clone()),
        transport: Transport::Torrent,
        translation: None,
        languages: language.into_iter().map(str::to_owned).collect(),
        player: None,
        size: Some(size),
        seeders: Some(seeders),
        info_hash: Some(hash),
        reported_video_ads: false,
    }];
    Ok(Work {
        reference,
        title,
        category,
        url,
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants,
        notes: Vec::new(),
        needs_inspection,
    })
}

fn quality(title: &str) -> Option<String> {
    let values = title
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter_map(|token| token.strip_suffix('p').or_else(|| token.strip_suffix('P')))
        .filter(|height| {
            matches!(
                *height,
                "360" | "480" | "576" | "720" | "1080" | "1440" | "2160"
            )
        })
        .collect::<HashSet<_>>();
    (values.len() == 1).then(|| format!("{}p", values.into_iter().next().unwrap()))
}

fn episode(title: &str) -> Option<String> {
    // A common explicit single-episode release suffix, without guessing seasons,
    // batches, ranges, volume numbers or unrelated numeric title fragments.
    let (_, suffix) = title.rsplit_once(" - ")?;
    let token = suffix.split_whitespace().next()?;
    let number = token.split_once('v').map_or(token, |(number, version)| {
        if !version.is_empty() && version.bytes().all(|value| value.is_ascii_digit()) {
            number
        } else {
            token
        }
    });
    if number.len() > 4 || number.is_empty() || !number.bytes().all(|value| value.is_ascii_digit())
    {
        return None;
    }
    Some(number.parse::<u16>().ok()?.to_string())
}

fn unique<'a>(root: ElementRef<'a>, selector: &str) -> Result<ElementRef<'a>> {
    let selector = Selector::parse(selector).expect("fixed metadata selector");
    let mut elements = root.select(&selector);
    let first = elements
        .next()
        .ok_or(Error::InvalidResponse("missing publication metadata"))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous publication metadata"));
    }
    Ok(first)
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn number(value: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|value| value.is_ascii_digit()) {
        return Err(Error::InvalidResponse("invalid torrent number"));
    }
    value
        .parse()
        .map_err(|_| Error::InvalidResponse("torrent number exceeds its limit"))
}

fn size_bytes(value: &str) -> Result<u64> {
    let (amount, unit) = value
        .split_once(' ')
        .ok_or(Error::InvalidResponse("invalid torrent size"))?;
    let factor = match unit {
        "Bytes" => 1u128,
        "KiB" => 1024,
        "MiB" => 1024u128.pow(2),
        "GiB" => 1024u128.pow(3),
        "TiB" => 1024u128.pow(4),
        _ => return Err(Error::InvalidResponse("invalid torrent size unit")),
    };
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    if whole.is_empty()
        || !whole.bytes().all(|value| value.is_ascii_digit())
        || fraction.len() > 9
        || !fraction.bytes().all(|value| value.is_ascii_digit())
    {
        return Err(Error::InvalidResponse("invalid torrent size"));
    }
    let denominator = 10u128.pow(fraction.len() as u32);
    let whole = whole
        .parse::<u128>()
        .map_err(|_| Error::InvalidResponse("torrent size exceeds its limit"))?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u128>().unwrap()
    };
    let bytes = whole
        .checked_mul(denominator)
        .and_then(|value| value.checked_add(fraction))
        .and_then(|value| value.checked_mul(factor))
        .map(|value| value / denominator)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(Error::InvalidResponse("torrent size exceeds its limit"))?;
    Ok(bytes)
}
