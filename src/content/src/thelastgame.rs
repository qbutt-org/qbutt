use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::{SourceDocument, SourceHttp, SourceSession, validate_https_url};
use crate::torrent::info_hash;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, TorrentMetadata, Transport,
    Variant, VariantRef, VariantSelector, Work,
};

const ORIGIN: &str = "https://thelastgame.ru";
const MAX_TORRENTS: usize = 16;

/// Public game articles and their same-origin torrent attachments. The blue
/// uTorria button runs a third-party tool and is not an acquisition route.
pub struct TheLastGame {
    http: SourceHttp,
    origin: Url,
}

struct Release {
    work: Work,
    torrents: BTreeMap<String, TorrentMetadata>,
}

impl TheLastGame {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed TheLastGame origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?,
            origin,
        })
    }

    pub async fn search(&self, query: &str, category: Category) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        if category != Category::Games {
            return Ok(Vec::new());
        }

        let mut url = self.origin.clone();
        url.query_pairs_mut().append_pair("s", query);
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let (arrived, body) = html(
            self.http
                .session_document(url.clone(), &mut session)
                .await?,
        )?;
        if arrived != url {
            // WordPress redirects a search with exactly one result to its article.
            let (reference, _) = publication(arrived.as_str(), &self.origin)?;
            let mut work = article(&body, &self.origin, &arrived, &reference)?;
            work.needs_inspection = true;
            return Ok(vec![work]);
        }
        search_results(&body, &self.origin, query)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.release(publication).await?.work)
    }

    /// A changed torrent at an unchanged URL cannot replace the selected hash.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        let mut release = self.release(&selected.publication).await?;
        let VariantSelector::TorrentHash { info_hash } = &selected.selector else {
            return Err(Error::InvalidReference);
        };
        let metadata = release
            .torrents
            .remove(info_hash)
            .ok_or(Error::VariantUnavailable)?;
        Ok(ResolvedContent::TorrentFile {
            reference: selected.clone(),
            metadata,
        })
    }

    async fn release(&self, publication: &PublicationRef) -> Result<Release> {
        let slug = publication_slug(publication)?;
        let url = self
            .origin
            .join(&format!("/{slug}/"))
            .expect("validated article slug");
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let response = self.http.session_document(url, &mut session).await?;
        if matches!(response.status, 404 | 410) {
            return Err(Error::VariantUnavailable);
        }
        let (arrived, body) = html(response)?;
        let mut work = article(&body, &self.origin, &arrived, publication)?;
        let urls = {
            let document = Html::parse_document(&body);
            let article = unique(
                document.root_element(),
                ".content > .pad.group > article.post",
            )?;
            let links = selector(".entry-inner a.btn_green[href]");
            let mut urls = Vec::new();
            for link in article.select(&links) {
                let Some(download) = torrent_url(link.attr("href").unwrap(), &self.origin) else {
                    continue;
                };
                if urls.contains(&download) {
                    continue;
                }
                if urls.len() == MAX_TORRENTS {
                    return Err(Error::InvalidResponse("too many TheLastGame torrents"));
                }
                urls.push(download);
            }
            urls
        };
        if urls.is_empty() {
            return Err(Error::UnsupportedAcquisition(
                "TheLastGame article has no same-origin torrent",
            ));
        }

        let mut torrents = BTreeMap::new();
        for download in &urls {
            let response = self
                .http
                .session_referred_metadata(download.clone(), &mut session, &arrived)
                .await?;
            if matches!(response.status, 404 | 410) {
                continue;
            }
            let metadata = torrent_response(response, download)?;
            let hash = info_hash(metadata.bytes())?;
            if torrents.contains_key(&hash) {
                continue;
            }
            let filename = download
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .and_then(|name| name.strip_suffix(".torrent"))
                .expect("validated torrent filename");
            let filename = percent_encoding::percent_decode_str(filename)
                .decode_utf8()
                .map_err(|_| Error::InvalidResponse("invalid TheLastGame torrent filename"))?
                .replace('_', " ");
            let file_title = filename.trim_matches('.').trim();
            if file_title.is_empty() || file_title.chars().any(char::is_control) {
                return Err(Error::InvalidResponse(
                    "invalid TheLastGame torrent filename",
                ));
            }
            work.variants.push(Variant {
                reference: VariantRef {
                    publication: publication.clone(),
                    selector: VariantSelector::TorrentHash {
                        info_hash: hash.clone(),
                    },
                },
                title: Some(file_title.to_owned()),
                transport: Transport::Torrent,
                episode: None,
                quality: None,
                translation: None,
                languages: Vec::new(),
                player: None,
                size: None,
                seeders: None,
                info_hash: Some(hash.clone()),
                reported_video_ads: false,
            });
            torrents.insert(hash, metadata);
        }
        if work.variants.is_empty() {
            return Err(Error::VariantUnavailable);
        }
        Ok(Release { work, torrents })
    }
}

fn html(response: SourceDocument) -> Result<(Url, String)> {
    if matches!(response.status, 403 | 503) {
        return Err(Error::Challenge);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if !response.content_type.as_deref().is_some_and(|mime| {
        mime.split(';')
            .next()
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("text/html"))
    }) {
        return Err(Error::InvalidResponse("TheLastGame article is not HTML"));
    }
    let body = String::from_utf8(response.body)
        .map_err(|_| Error::InvalidResponse("TheLastGame article is not UTF-8"))?;
    if body.contains("Just a moment") || body.contains("cf-challenge") {
        return Err(Error::Challenge);
    }
    Ok((response.final_url, body))
}

fn torrent_response(response: SourceDocument, expected: &Url) -> Result<TorrentMetadata> {
    if matches!(response.status, 403 | 503) {
        return Err(Error::Challenge);
    }
    if response.status != 200 {
        return Err(Error::HttpStatus(response.status));
    }
    if response.final_url != *expected {
        return Err(Error::VariantUnavailable);
    }
    if !response.content_type.as_deref().is_some_and(|mime| {
        mime.split(';').next().is_some_and(|mime| {
            mime.trim().eq_ignore_ascii_case("application/octet-stream")
                || mime.trim().eq_ignore_ascii_case("application/x-bittorrent")
        })
    }) {
        return Err(Error::InvalidResponse(
            "TheLastGame download is not torrent metadata",
        ));
    }
    TorrentMetadata::new(response.body)
}

fn search_results(body: &str, origin: &Url, query: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let notebox = unique(root, ".content .notebox")?;
    if text(unique(notebox, "span")?) != query.split_whitespace().collect::<Vec<_>>().join(" ") {
        return Err(Error::InvalidResponse("TheLastGame search query changed"));
    }
    let heading = text(unique(root, ".content .page-title h1")?);
    let count = heading
        .strip_suffix(" Search results")
        .and_then(|count| count.parse::<usize>().ok())
        .ok_or(Error::InvalidResponse("missing TheLastGame search count"))?;
    let cards = selector("#grid-wrapper .post-row > article.post");
    let rows = root.select(&cards).collect::<Vec<_>>();
    if count == 0 && text(notebox).contains("ничего не найдено") {
        if !rows.is_empty() {
            return Err(Error::InvalidResponse(
                "TheLastGame empty search contains results",
            ));
        }
        return Ok(Vec::new());
    }
    if rows.is_empty() || rows.len() > 100 || rows.len() > count {
        return Err(Error::InvalidResponse("missing TheLastGame search results"));
    }
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let link = unique(row, ".post-title a[href]")?;
        let (reference, url) = publication(link.attr("href").unwrap(), origin)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate TheLastGame publication"));
        }
        works.push(work(reference, url, checked_text(link)?, true));
    }
    Ok(works)
}

fn article(body: &str, origin: &Url, arrived: &Url, expected: &PublicationRef) -> Result<Work> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let canonical = unique(root, "link[rel='canonical'][href]")?;
    let (reference, url) = publication(canonical.attr("href").unwrap(), origin)?;
    let (arrival, _) = publication(arrived.as_str(), origin)?;
    if reference != *expected || arrival != *expected {
        return Err(Error::VariantUnavailable);
    }
    let article = unique(root, ".content > .pad.group > article.post")?;
    let raw_title = checked_text(unique(article, "h1.post-title")?)?;
    let title = raw_title
        .strip_prefix("Скачать игру ")
        .and_then(|title| title.strip_suffix(" [Новая Версия] на ПК (на Русском)"))
        .unwrap_or(&raw_title)
        .to_owned();
    let mut work = work(reference, url, title, false);
    let entry = unique(article, ".entry-inner")?;
    let mut description = Vec::new();
    if let Some(paragraph) = entry.select(&selector("p")).next() {
        let introduction = text(paragraph);
        if !introduction.is_empty() && introduction.len() <= 12_000 {
            description.push(introduction);
        }
    }
    // These are publication-wide reports, not proof that every attachment has
    // the same edition or languages. Preserve them without relabeling variants.
    if let Some(details) = entry
        .select(&selector("div"))
        .map(text)
        .find(|value| value.starts_with("Информация о игре") && value.len() <= 4000)
    {
        description.push(details);
    }
    work.description = (!description.is_empty()).then(|| description.join("\n\n"));
    Ok(work)
}

fn publication(input: &str, origin: &Url) -> Result<(PublicationRef, Url)> {
    let url = origin.join(input).map_err(|_| Error::InvalidReference)?;
    validate_https_url(&url)?;
    if url.origin() != origin.origin() || url.query().is_some() || url.fragment().is_some() {
        return Err(Error::InvalidReference);
    }
    let slug = url
        .path()
        .strip_prefix('/')
        .and_then(|path| path.strip_suffix('/'))
        .ok_or(Error::InvalidReference)?;
    let reference = PublicationRef {
        source: Source::TheLastGame,
        id: slug.to_owned(),
    };
    reference.validate()?;
    Ok((reference, url))
}

pub(crate) fn publication_slug(reference: &PublicationRef) -> Result<&str> {
    let slug = &reference.id;
    if reference.source == Source::TheLastGame
        && !slug.is_empty()
        && slug.len() <= 200
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        Ok(slug)
    } else {
        Err(Error::InvalidReference)
    }
}

fn torrent_url(input: &str, origin: &Url) -> Option<Url> {
    let url = origin.join(input).ok()?;
    (validate_https_url(&url).is_ok()
        && url.origin() == origin.origin()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().starts_with("/download/")
        && url.path().ends_with(".torrent")
        && url.path().len() <= 512
        && url.path() != "/download/.torrent")
        .then_some(url)
}

fn work(reference: PublicationRef, url: Url, title: String, needs_inspection: bool) -> Work {
    Work {
        reference,
        title,
        category: Category::Games,
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

fn checked_text(element: ElementRef<'_>) -> Result<String> {
    let value = text(element);
    if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid TheLastGame title"));
    }
    Ok(value)
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

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("fixed TheLastGame selector")
}

fn unique<'a>(root: ElementRef<'a>, css: &str) -> Result<ElementRef<'a>> {
    let parsed = selector(css);
    let mut elements = root.select(&parsed);
    let first = elements
        .next()
        .ok_or(Error::InvalidResponse("missing TheLastGame page metadata"))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse(
            "ambiguous TheLastGame page metadata",
        ));
    }
    Ok(first)
}
