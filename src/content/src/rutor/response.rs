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
    size: Option<u64>,
    seeders: Option<u64>,
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

pub(crate) fn search(body: &str, origin: &Url, route: u8, category: Category) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let script_selector = Selector::parse("script").expect("fixed script selector");
    let mut confirmed = None;
    for script in document.select(&script_selector) {
        for line in script.text().collect::<String>().lines() {
            let Some(rest) = line.trim().strip_prefix("var ") else {
                continue;
            };
            let Some(rest) = rest.trim_start().strip_prefix("search_category") else {
                continue;
            };
            let value = rest
                .trim_start()
                .strip_prefix('=')
                .and_then(|value| value.trim().strip_suffix(';'))
                .and_then(|value| value.trim().parse::<u8>().ok())
                .ok_or(Error::InvalidResponse(
                    "invalid search category acknowledgement",
                ))?;
            if confirmed.replace(value).is_some() {
                return Err(Error::InvalidResponse("ambiguous search category"));
            }
        }
    }
    if confirmed != Some(route) {
        return Err(Error::InvalidResponse("search category was not confirmed"));
    }

    let index_selector = Selector::parse("#index").expect("fixed results selector");
    let mut indexes = document.select(&index_selector);
    let index = indexes
        .next()
        .ok_or(Error::InvalidResponse("missing search results table"))?;
    if indexes.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous search results table"));
    }
    let summary = index
        .children()
        .filter_map(|node| node.value().as_text())
        .map(|value| &**value)
        .collect::<String>();
    let total = summary
        .trim()
        .strip_prefix("Результатов поиска ")
        .and_then(|value| value.split_whitespace().next())
        .map(number)
        .transpose()?
        .filter(|value| *value <= 2000)
        .ok_or(Error::InvalidResponse("missing search result count"))?;
    let header_selector = Selector::parse("tr.backgr").expect("fixed results header selector");
    let header = index
        .select(&header_selector)
        .next()
        .ok_or(Error::InvalidResponse("missing search results header"))?;
    if !text(header).contains("Название") {
        return Err(Error::InvalidResponse("invalid search results header"));
    }

    let row_selector = Selector::parse("tr").expect("fixed results row selector");
    let title_selector =
        Selector::parse("a[href^='/torrent/']").expect("fixed publication selector");
    let magnet_selector = Selector::parse("a[href^='magnet:']").expect("fixed magnet selector");
    let seed_selector = Selector::parse("span.green").expect("fixed seed selector");
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for row in index.select(&row_selector) {
        let mut titles = row.select(&title_selector);
        let title = titles.next();
        let mut magnets = row.select(&magnet_selector);
        let magnet_link = magnets.next();
        if title.is_none() && magnet_link.is_none() {
            continue;
        }
        let title = title.ok_or(Error::InvalidResponse("missing torrent publication"))?;
        let magnet_link = magnet_link.ok_or(Error::InvalidResponse("missing torrent magnet"))?;
        if titles.next().is_some() || magnets.next().is_some() || works.len() >= 500 {
            return Err(Error::InvalidResponse(
                "ambiguous or excessive torrent rows",
            ));
        }
        let (reference, url) = publication_url(title.attr("href").unwrap(), origin)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate torrent publication"));
        }
        let (_, hash) = magnet(magnet_link.attr("href").unwrap())?;
        let mut seeds = row.select(&seed_selector);
        let seeders = seeds
            .next()
            .map(text)
            .map(|value| number(&value))
            .transpose()?;
        if seeds.next().is_some() {
            return Err(Error::InvalidResponse("ambiguous seed count"));
        }
        let mut size = None;
        for cell in row
            .child_elements()
            .filter(|element| element.value().name() == "td")
        {
            if cell.child_elements().next().is_some() {
                continue;
            }
            let value = text(cell);
            if let Some(parsed) = size_bytes(&value)?
                && size.replace(parsed).is_some()
            {
                return Err(Error::InvalidResponse("ambiguous torrent size"));
            }
        }
        works.push(work(
            reference,
            category,
            url,
            TorrentMetadata {
                title: text(title),
                hash,
                size,
                seeders,
            },
            true,
        )?);
    }
    if (total == 0) != works.is_empty() || works.len() as u64 > total {
        return Err(Error::InvalidResponse(
            "search rows do not match their count",
        ));
    }
    Ok(works)
}

pub(crate) fn release(body: &str, origin: &Url, expected: &PublicationRef) -> Result<Release> {
    expected.rutor_id()?;
    let document = Html::parse_document(body);
    let title_selector = Selector::parse("h1").expect("fixed title selector");
    let mut titles = document.select(&title_selector);
    let title = text(
        titles
            .next()
            .ok_or(Error::InvalidResponse("missing publication title"))?,
    );
    if titles.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous publication title"));
    }

    let download_selector = Selector::parse("#download a[href]").expect("fixed download selector");
    let mut primary_magnet = None;
    let mut identity = None;
    for link in document.select(&download_selector) {
        let href = link.attr("href").unwrap();
        if href.starts_with("magnet:") {
            if primary_magnet.replace(magnet(href)?).is_some() {
                return Err(Error::InvalidResponse("ambiguous primary magnet"));
            }
            continue;
        }
        let url = origin.join(href).map_err(|_| Error::UnsafeAddress)?;
        if !url.path().starts_with("/download/") {
            continue;
        }
        validate_https_url(&url)?;
        if url.host_str() != Some("d.rutor.info") || url.query().is_some() {
            return Err(Error::UnsafeAddress);
        }
        let reference = PublicationRef {
            source: Source::RuTor,
            id: url.path().trim_start_matches("/download/").into(),
        };
        reference.validate()?;
        if identity.replace(reference).is_some() {
            return Err(Error::InvalidResponse("ambiguous publication identity"));
        }
    }
    if identity.as_ref() != Some(expected) {
        return Err(Error::VariantUnavailable);
    }
    let (magnet, hash) = primary_magnet.ok_or(Error::VariantUnavailable)?;

    let row_selector =
        Selector::parse("#details > tbody > tr, #details > tr").expect("fixed details selector");
    let mut category = None;
    let mut size = None;
    let mut seeders = None;
    for row in document.select(&row_selector) {
        let cells = row.child_elements().collect::<Vec<_>>();
        if cells.len() != 2 || !cells[0].value().classes().any(|value| value == "header") {
            continue;
        }
        let value = text(cells[1]);
        match text(cells[0]).as_str() {
            "Категория" => {
                let actual = match value.as_str() {
                    "Зарубежные фильмы" | "Наши фильмы" | "Научно-популярные фильмы" => {
                        Category::Movies
                    }
                    "Зарубежные сериалы" | "Наши сериалы" | "Телевизор" => {
                        Category::Tv
                    }
                    "Аниме" => Category::Anime,
                    "Игры" => Category::Games,
                    "Софт" => Category::Software,
                    "Книги" => Category::Books,
                    _ => return Err(Error::InvalidResponse("unsupported publication category")),
                };
                if category.replace(actual).is_some() {
                    return Err(Error::InvalidResponse("ambiguous publication category"));
                }
            }
            "Размер" => {
                let bytes = value
                    .rsplit_once('(')
                    .and_then(|(_, exact)| exact.strip_suffix(" Bytes)"))
                    .map(number)
                    .transpose()?;
                if size
                    .replace(bytes.ok_or(Error::InvalidResponse("missing exact torrent size"))?)
                    .is_some()
                {
                    return Err(Error::InvalidResponse("ambiguous torrent size"));
                }
            }
            "Раздают" if seeders.replace(number(&value)?).is_some() => {
                return Err(Error::InvalidResponse("ambiguous seed count"));
            }
            _ => {}
        }
    }
    let category = category.ok_or(Error::InvalidResponse("missing publication category"))?;
    let url = origin
        .join(&format!("/torrent/{}", expected.id))
        .expect("canonical publication path");
    Ok(Release {
        work: work(
            expected.clone(),
            category,
            url,
            TorrentMetadata {
                title,
                hash,
                size,
                seeders,
            },
            false,
        )?,
        magnet,
    })
}

fn publication_url(input: &str, origin: &Url) -> Result<(PublicationRef, Url)> {
    let url = origin.join(input).map_err(|_| Error::UnsafeAddress)?;
    validate_https_url(&url)?;
    if url.origin() != origin.origin() || url.query().is_some() {
        return Err(Error::UnsafeAddress);
    }
    let id = url
        .path_segments()
        .and_then(|mut segments| {
            (segments.next() == Some("torrent"))
                .then(|| segments.next())
                .flatten()
        })
        .ok_or(Error::InvalidReference)?;
    let reference = PublicationRef {
        source: Source::RuTor,
        id: id.into(),
    };
    reference.validate()?;
    Ok((reference, url))
}

fn work(
    reference: PublicationRef,
    category: Category,
    url: Url,
    torrent: TorrentMetadata,
    needs_inspection: bool,
) -> Result<Work> {
    let TorrentMetadata {
        title,
        hash,
        size,
        seeders,
    } = torrent;
    if title.is_empty() || title.len() > 2048 {
        return Err(Error::InvalidResponse("invalid publication title"));
    }
    let variant = Variant {
        reference: VariantRef {
            publication: reference.clone(),
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
        size,
        seeders,
        info_hash: Some(hash),
        reported_video_ads: false,
    };
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
        variants: vec![variant],
        needs_inspection,
    })
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
    let digits = value
        .chars()
        .filter(|value| !value.is_whitespace())
        .collect::<String>();
    if digits.is_empty() || !digits.bytes().all(|value| value.is_ascii_digit()) {
        return Err(Error::InvalidResponse("invalid torrent number"));
    }
    digits
        .parse()
        .map_err(|_| Error::InvalidResponse("torrent number exceeds its limit"))
}

fn size_bytes(value: &str) -> Result<Option<u64>> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    let Some(unit) = parts.last().copied() else {
        return Ok(None);
    };
    let factor: u128 = match unit {
        "B" => 1,
        "kB" | "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        "TB" => 1024u128.pow(4),
        _ => return Ok(None),
    };
    if parts.len() != 2 {
        return Err(Error::InvalidResponse("invalid torrent size"));
    }
    let value = parts[0].replace(',', ".");
    let (whole, fractional) = value.split_once('.').unwrap_or((&value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.len() > 9
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Error::InvalidResponse("invalid torrent size"));
    }
    let denominator = 10u128.pow(fractional.len() as u32);
    let whole = whole
        .parse::<u64>()
        .map_err(|_| Error::InvalidResponse("torrent size exceeds its limit"))?;
    let fraction = fractional.parse::<u64>().unwrap_or(0);
    let bytes = (u128::from(whole) * denominator + u128::from(fraction))
        .checked_mul(factor)
        .ok_or(Error::InvalidResponse("torrent size exceeds its limit"))?
        / denominator;
    Ok(Some(u64::try_from(bytes).map_err(|_| {
        Error::InvalidResponse("torrent size exceeds its limit")
    })?))
}
