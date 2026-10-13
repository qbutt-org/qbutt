use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::SourceHttp;
use crate::torrent::magnet;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, Transport, Variant,
    VariantRef, VariantSelector, Work,
};

const ORIGIN: &str = "https://www.limetorrents.fun/";

/// LimeTorrents' public catalog. Only the site's magnet is acquired; its
/// external torrent-file and advertised direct-download links are not used.
pub struct LimeTorrents {
    http: SourceHttp,
    origin: Url,
}

impl LimeTorrents {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed LimeTorrents origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?,
            origin,
        })
    }

    pub async fn search(&self, query: &str, category: Category) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.len() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        if category == Category::Manga {
            return Ok(Vec::new());
        }

        // The site ignores category segments in search routes. Its "all" page
        // reports each row's category, so filter that reported value instead.
        let normalized = query
            .split(|character: char| !character.is_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if normalized.is_empty() {
            return Err(Error::InvalidQuery);
        }
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .expect("fixed HTTPS origin")
            .extend(["search", "all", &normalized, "date", "1", ""]);
        let (arrived, body) = self.http.html_page(url.clone()).await?;
        if arrived != url {
            return Err(Error::InvalidResponse("search redirected to another page"));
        }
        search_results(&body, &self.origin, &normalized, category)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.release(publication).await?.work)
    }

    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        let release = self.release(&selected.publication).await?;
        if release.work.variants[0].reference != *selected {
            return Err(Error::VariantUnavailable);
        }
        Ok(ResolvedContent::Torrent {
            reference: selected.clone(),
            title: release.work.title,
            magnet: release.magnet,
        })
    }

    async fn release(&self, publication: &PublicationRef) -> Result<Release> {
        let path = publication.limetorrents_path()?;
        let url = self
            .origin
            .join(path)
            .expect("validated single-segment publication path");
        let (arrived, body) = match self.http.html_page(url.clone()).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        if arrived != url {
            return Err(Error::VariantUnavailable);
        }
        release(&body, url, publication)
    }
}

struct Release {
    work: Work,
    magnet: Url,
}

fn search_results(body: &str, origin: &Url, query: &str, category: Category) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let query_label = unique(document.root_element(), "#content .querytext")?;
    if text(query_label) != query {
        return Err(Error::InvalidResponse("search query was not confirmed"));
    }

    let tables = Selector::parse("#content table.table2").expect("fixed results table selector");
    let mut tables = document.select(&tables);
    let Some(table) = tables.next() else {
        let empty = Selector::parse("#content h2").expect("fixed empty result selector");
        if document
            .select(&empty)
            .any(|heading| text(heading) == "No results found")
        {
            return Ok(Vec::new());
        }
        return Err(Error::InvalidResponse("missing search results table"));
    };
    if tables.next().is_some()
        || text(unique(table, "th.thleft > span:first-child")?) != "Torrent Name"
    {
        return Err(Error::InvalidResponse("ambiguous search results table"));
    }

    let rows = Selector::parse("tr[bgcolor]").expect("fixed result row selector");
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for (index, row) in table.select(&rows).enumerate() {
        if index >= 100 {
            return Err(Error::InvalidResponse("excessive search results"));
        }
        let cells = row
            .child_elements()
            .filter(|element| element.value().name() == "td")
            .collect::<Vec<_>>();
        if cells.len() != 6 {
            return Err(Error::InvalidResponse("invalid search result row"));
        }
        let reported = text(cells[1]);
        let reported = reported
            .rsplit_once(" - in ")
            .map(|(_, value)| value)
            .ok_or(Error::InvalidResponse("missing torrent category"))?;
        if source_category(reported) != Some(category) {
            continue;
        }

        let link = unique(cells[0], "div.tt-name > a[href$='.html']")?;
        let (reference, url) = publication(link.attr("href").unwrap(), origin)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate torrent publication"));
        }
        // Search titles have an extra entity layer compared with detail pages.
        // Preserve literal angle brackets while decoding that layer as text.
        let escaped_title = text(link).replace('<', "&lt;");
        let decoded_title = Html::parse_fragment(&escaped_title);
        works.push(work(
            reference,
            url,
            title(decoded_title.root_element())?,
            category,
            true,
        ));
    }
    Ok(works)
}

fn release(body: &str, url: Url, expected: &PublicationRef) -> Result<Release> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let title = title(unique(root, "#content > h1")?)?;
    let table = unique(root, "#content .torrentinfo > table")?;
    let rows = Selector::parse("tr").expect("fixed metadata row selector");
    let mut hash = None;
    let mut category = None;
    let mut size = None;
    for row in table.select(&rows) {
        let cells = row
            .child_elements()
            .filter(|element| element.value().name() == "td")
            .collect::<Vec<_>>();
        if cells.len() != 2 {
            continue;
        }
        match text(cells[0]).trim_end_matches(':').trim() {
            "Torrent Hash" => {
                let value = text(cells[1]).to_ascii_lowercase();
                if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(Error::InvalidResponse("invalid torrent hash"));
                }
                if hash.replace(value).is_some() {
                    return Err(Error::InvalidResponse("ambiguous torrent hash"));
                }
            }
            "Torrent Added" => {
                let links = Selector::parse("a[href^='/browse-torrents/']")
                    .expect("fixed category selector");
                let link = cells[1]
                    .select(&links)
                    .next()
                    .ok_or(Error::InvalidResponse("missing torrent category"))?;
                let value = source_category(&text(link))
                    .ok_or(Error::InvalidResponse("unsupported torrent category"))?;
                if category.replace(value).is_some() {
                    return Err(Error::InvalidResponse("ambiguous torrent category"));
                }
            }
            "Torrent Size" => {
                if size.replace(size_bytes(&text(cells[1]))?).is_some() {
                    return Err(Error::InvalidResponse("ambiguous torrent size"));
                }
            }
            _ => {}
        }
    }
    let hash = hash.ok_or(Error::InvalidResponse("missing torrent hash"))?;
    let category = category.ok_or(Error::InvalidResponse("missing torrent category"))?;
    let size = size.ok_or(Error::InvalidResponse("missing torrent size"))?;

    let magnets = Selector::parse("#content .downloadarea a[href^='magnet:']")
        .expect("fixed magnet selector");
    let mut magnet_url = None;
    for (index, link) in document.select(&magnets).enumerate() {
        if index >= 4 {
            return Err(Error::InvalidResponse("excessive torrent magnets"));
        }
        let (url, magnet_hash) = magnet(link.attr("href").unwrap())?;
        if magnet_hash != hash || magnet_url.as_ref().is_some_and(|previous| previous != &url) {
            return Err(Error::InvalidResponse("torrent magnet hash mismatch"));
        }
        magnet_url = Some(url);
    }
    let magnet = magnet_url.ok_or(Error::VariantUnavailable)?;

    let seeders = unique(root, "#content > span.greenish")?;
    let seeders = text(seeders)
        .strip_prefix("Seeders :")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .ok_or(Error::InvalidResponse("invalid seed count"))?;
    let mut work = work(expected.clone(), url, title, category, false);
    work.variants.push(Variant {
        reference: VariantRef {
            publication: expected.clone(),
            selector: VariantSelector::TorrentHash {
                info_hash: hash.clone(),
            },
        },
        title: None,
        transport: Transport::Torrent,
        episode: None,
        quality: None,
        translation: None,
        languages: Vec::new(),
        player: None,
        size: Some(size),
        seeders: Some(seeders),
        info_hash: Some(hash),
        reported_video_ads: false,
    });
    Ok(Release { work, magnet })
}

fn publication(input: &str, origin: &Url) -> Result<(PublicationRef, Url)> {
    let url = origin.join(input).map_err(|_| Error::InvalidReference)?;
    if url.origin() != origin.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url
            .path_segments()
            .is_none_or(|mut segments| segments.next().is_none() || segments.next().is_some())
    {
        return Err(Error::InvalidReference);
    }
    let reference = PublicationRef {
        source: Source::LimeTorrents,
        id: url.path().trim_start_matches('/').to_owned(),
    };
    reference.limetorrents_path()?;
    Ok((reference, url))
}

fn source_category(value: &str) -> Option<Category> {
    match value {
        "Movies" => Some(Category::Movies),
        "TV shows" => Some(Category::Tv),
        "Anime" => Some(Category::Anime),
        "Games" => Some(Category::Games),
        "Applications" => Some(Category::Software),
        "E-books" => Some(Category::Books),
        _ => None,
    }
}

fn size_bytes(value: &str) -> Result<u64> {
    let (number, unit) = value
        .split_once(' ')
        .ok_or(Error::InvalidResponse("invalid torrent size"))?;
    let number = number
        .parse::<f64>()
        .map_err(|_| Error::InvalidResponse("invalid torrent size"))?;
    let multiplier = match unit {
        "B" => 1_u64,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        "TB" => 1024_u64.pow(4),
        _ => return Err(Error::InvalidResponse("invalid torrent size unit")),
    };
    let bytes = number * multiplier as f64;
    if !bytes.is_finite() || !(1.0..=u64::MAX as f64).contains(&bytes) {
        return Err(Error::InvalidResponse("invalid torrent size"));
    }
    Ok(bytes as u64)
}

fn work(
    reference: PublicationRef,
    url: Url,
    title: String,
    category: Category,
    needs_inspection: bool,
) -> Work {
    Work {
        reference,
        title,
        category,
        url,
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants: Vec::new(),
        needs_inspection,
    }
}

fn title(element: ElementRef<'_>) -> Result<String> {
    let value = text(element);
    if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid torrent title"));
    }
    Ok(value)
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn unique<'a>(root: ElementRef<'a>, selector: &str) -> Result<ElementRef<'a>> {
    let selector = Selector::parse(selector).expect("fixed metadata selector");
    let mut elements = root.select(&selector);
    let element = elements
        .next()
        .ok_or(Error::InvalidResponse("missing LimeTorrents metadata"))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous LimeTorrents metadata"));
    }
    Ok(element)
}
