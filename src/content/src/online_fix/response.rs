use std::collections::{BTreeMap, HashMap, HashSet};

use encoding_rs::{UTF_8, WINDOWS_1251};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::{SourceDocument, validate_https_url};
use crate::torrent::info_hash;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, TorrentMetadata, Transport,
    Variant, VariantRef, VariantSelector, Work,
};

use super::description;

pub(super) struct Article {
    pub work: Work,
    pub torrent_directories: Vec<Url>,
}

pub(super) struct Release {
    pub work: Work,
    torrents: HashMap<String, TorrentMetadata>,
}

impl Release {
    pub(super) fn new(work: Work) -> Self {
        Self {
            work,
            torrents: HashMap::new(),
        }
    }

    pub(super) fn add_torrent(
        &mut self,
        filename: String,
        metadata: TorrentMetadata,
    ) -> Result<()> {
        let hash = info_hash(metadata.bytes())?;
        if self.torrents.contains_key(&hash) {
            return Err(Error::InvalidResponse("duplicate Online-Fix torrent hash"));
        }
        self.work.variants.push(Variant {
            reference: VariantRef {
                publication: self.work.reference.clone(),
                selector: VariantSelector::TorrentHash {
                    info_hash: hash.clone(),
                },
            },
            title: Some(format!("Full game: {filename}")),
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
        self.torrents.insert(hash, metadata);
        Ok(())
    }

    pub(super) fn resolve(mut self, selected: &VariantRef) -> Result<ResolvedContent> {
        let VariantSelector::TorrentHash { info_hash } = &selected.selector else {
            return Err(Error::InvalidReference);
        };
        let metadata = self
            .torrents
            .remove(info_hash)
            .ok_or(Error::VariantUnavailable)?;
        Ok(ResolvedContent::TorrentFile {
            reference: selected.clone(),
            metadata,
        })
    }
}

pub(super) fn site_document(response: SourceDocument, expected: &Url) -> Result<String> {
    if response.final_url != *expected {
        return Err(Error::InvalidResponse(
            "Online-Fix response identity changed",
        ));
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    let content_type = response
        .content_type
        .as_deref()
        .ok_or(Error::InvalidResponse("missing Online-Fix content type"))?;
    let (mime, charset) = content_type
        .split_once(';')
        .ok_or(Error::InvalidResponse("missing Online-Fix charset"))?;
    if !mime.trim().eq_ignore_ascii_case("text/html") {
        return Err(Error::InvalidResponse("Online-Fix response is not HTML"));
    }
    let encoding = match charset.trim().to_ascii_lowercase().as_str() {
        "charset=windows-1251" | "charset=cp1251" => WINDOWS_1251,
        "charset=utf-8" => UTF_8,
        _ => return Err(Error::InvalidResponse("unsupported Online-Fix charset")),
    };
    let (body, invalid) = encoding.decode_without_bom_handling(&response.body);
    if invalid {
        return Err(Error::InvalidResponse("invalid Online-Fix text encoding"));
    }
    Ok(body.into_owned())
}

pub(super) fn search(body: &str, origin: &Url, query: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let echoed = unique(root, "#dle-content #searchinput[name='story']")?;
    if echoed.attr("value") != Some(query) {
        return Err(Error::InvalidResponse("Online-Fix search query changed"));
    }
    let content = unique(root, "#dle-content")?;
    let rows = selector(".news.news-search");
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for (index, row) in content.select(&rows).enumerate() {
        if index >= 100 {
            return Err(Error::InvalidResponse("too many Online-Fix results"));
        }
        let heading = unique(row, "h2.title")?;
        let link = heading
            .parent()
            .and_then(ElementRef::wrap)
            .ok_or(Error::InvalidResponse("missing Online-Fix result link"))?;
        if link.value().name() != "a" {
            return Err(Error::InvalidResponse("invalid Online-Fix result link"));
        }
        let (reference, url) = publication(
            link.attr("href")
                .ok_or(Error::InvalidResponse("missing Online-Fix result URL"))?,
            origin,
        )?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate Online-Fix publication"));
        }
        works.push(work(reference, url, game_title(heading)?, true));
    }
    if works.is_empty()
        && !content
            .text()
            .collect::<String>()
            .contains("поиск по сайту не дал никаких результатов")
    {
        return Err(Error::InvalidResponse(
            "Online-Fix search result is missing",
        ));
    }
    Ok(works)
}

pub(super) fn article(
    body: &str,
    origin: &Url,
    uploads_origin: &Url,
    url: &Url,
    expected: &PublicationRef,
) -> Result<Article> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let canonical = unique(root, "link[rel='canonical'][href]")?;
    let (reference, canonical_url) = publication(canonical.attr("href").unwrap(), origin)?;
    let (arrived, _) = publication(url.as_str(), origin)?;
    if reference != *expected || arrived != *expected {
        return Err(Error::VariantUnavailable);
    }

    let article = unique(root, ".full-story")?;
    let title = game_title(unique(article, "#news-title")?)?;
    let content = unique(article, "[itemprop='articleBody']")?;
    let mut directories = Vec::new();
    for link in content.select(&selector("a[href]")) {
        if !text(link).eq_ignore_ascii_case("Скачать Torrent") {
            continue;
        }
        let url = Url::parse(link.attr("href").unwrap())
            .map_err(|_| Error::InvalidResponse("invalid Online-Fix torrent directory"))?;
        validate_uploads_url(&url, uploads_origin, "/torrents/")?;
        if !url.path().ends_with('/') || directories.contains(&url) {
            return Err(Error::InvalidResponse(
                "invalid Online-Fix torrent directory",
            ));
        }
        directories.push(url);
    }
    if directories.is_empty() || directories.len() > 4 {
        return Err(Error::UnsupportedAcquisition(
            "No Online-Fix game torrent directory is available",
        ));
    }
    let description = description::extract(content, &canonical_url)?;
    let mut work = work(reference, canonical_url, title, false);
    work.description = description;
    Ok(Article {
        work,
        torrent_directories: directories,
    })
}

pub(super) fn directory(
    response: SourceDocument,
    directory: &Url,
    uploads_origin: &Url,
) -> Result<Vec<(Url, String)>> {
    if response.final_url != *directory || !(200..300).contains(&response.status) {
        return Err(Error::VariantUnavailable);
    }
    if !response
        .content_type
        .as_deref()
        .is_some_and(|value| value.starts_with("text/html"))
    {
        return Err(Error::InvalidResponse("Online-Fix directory is not HTML"));
    }
    let body = String::from_utf8(response.body)
        .map_err(|_| Error::InvalidResponse("invalid Online-Fix directory encoding"))?;
    let document = Html::parse_document(&body);
    let heading = text(unique(document.root_element(), "h1")?);
    let path = percent_encoding::percent_decode_str(directory.path())
        .decode_utf8()
        .map_err(|_| Error::InvalidResponse("invalid Online-Fix directory path"))?;
    if heading != format!("Index of {path}") {
        return Err(Error::InvalidResponse(
            "Online-Fix directory identity changed",
        ));
    }
    let mut files = Vec::new();
    for link in document.select(&selector("pre > a[href]")) {
        let href = link.attr("href").unwrap();
        if href == "../" || href.ends_with('/') {
            continue;
        }
        let filename = text(link);
        if !filename.ends_with(".torrent") {
            continue;
        }
        if files.len() >= 8 || filename.len() > 255 || filename.chars().any(char::is_control) {
            return Err(Error::InvalidResponse("invalid Online-Fix torrent listing"));
        }
        let url = directory
            .join(href)
            .map_err(|_| Error::InvalidResponse("invalid Online-Fix torrent URL"))?;
        validate_uploads_url(&url, uploads_origin, "/torrents/")?;
        if !url.as_str().starts_with(directory.as_str()) || url.path().ends_with('/') {
            return Err(Error::InvalidResponse(
                "torrent escaped Online-Fix directory",
            ));
        }
        files.push((url, filename));
    }
    if files.is_empty() {
        return Err(Error::InvalidResponse(
            "Online-Fix torrent directory is empty",
        ));
    }
    Ok(files)
}

fn validate_uploads_url(url: &Url, uploads_origin: &Url, prefix: &str) -> Result<()> {
    validate_https_url(url)?;
    if url.origin() != uploads_origin.origin()
        || !url.path().starts_with(prefix)
        || url.query().is_some()
    {
        return Err(Error::UnsafeAddress);
    }
    Ok(())
}

fn publication(input: &str, origin: &Url) -> Result<(PublicationRef, Url)> {
    let url = origin.join(input).map_err(|_| Error::InvalidReference)?;
    validate_https_url(&url)?;
    if url.origin() != origin.origin() || url.query().is_some() {
        return Err(Error::InvalidReference);
    }
    let reference = PublicationRef {
        source: Source::OnlineFix,
        id: url.path().trim_start_matches('/').to_owned(),
    };
    reference.validate()?;
    Ok((reference, url))
}

fn game_title(element: ElementRef<'_>) -> Result<String> {
    let title = text(element);
    let title = title.strip_suffix(" по сети").unwrap_or(&title).trim();
    if title.is_empty() || title.len() > 2048 || title.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid Online-Fix game title"));
    }
    Ok(title.to_owned())
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

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed Online-Fix selector")
}

fn unique<'a>(root: ElementRef<'a>, query: &str) -> Result<ElementRef<'a>> {
    let selected = selector(query);
    let mut elements = root.select(&selected);
    let first = elements
        .next()
        .ok_or(Error::InvalidResponse("missing Online-Fix metadata"))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous Online-Fix metadata"));
    }
    Ok(first)
}
