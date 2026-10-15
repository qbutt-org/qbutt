use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

use encoding_rs::WINDOWS_1251;
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::{SourceDocument, SourceHttp, SourceSession};
use crate::torrent;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, TorrentMetadata, Transport,
    Variant, VariantRef, VariantSelector, Work,
};

const ORIGIN: &str = "https://freetp.org";
const MAX_FILES: usize = 16;

/// FreeTP's public game articles. Only same-origin `.torrent` attachments are
/// acquired; executable online fixes and external file hosts remain on the page.
pub struct FreeTP {
    http: SourceHttp,
    origin: Url,
}

struct Release {
    work: Work,
    torrents: BTreeMap<String, TorrentMetadata>,
}

impl FreeTP {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed FreeTP origin");
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

        let (bytes, _, invalid) = WINDOWS_1251.encode(query);
        if invalid {
            return Err(Error::InvalidQuery);
        }
        let mut story = String::new();
        for byte in bytes.iter().copied() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                story.push(char::from(byte));
            } else if byte == b' ' {
                story.push('+');
            } else {
                write!(story, "%{byte:02X}").expect("writing to String");
            }
        }

        let url = self.origin.join(&format!(
            "/index.php?do=search&subaction=search&story={story}&search_start=1&full_search=0&result_from=1"
        )).expect("fixed search path");
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let (arrived, body) = self.document(url.clone(), &mut session).await?;
        if arrived != url {
            return Err(Error::InvalidResponse("FreeTP search identity changed"));
        }
        parse_search(&body, &self.origin, query)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.release(publication).await?.work)
    }

    /// Re-fetch the publication and metadata. A changed file under the same
    /// FreeTP download ID cannot replace the selected torrent hash.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        let release = self.release(&selected.publication).await?;
        let VariantSelector::TorrentHash { info_hash } = &selected.selector else {
            return Err(Error::InvalidReference);
        };
        let metadata = release
            .torrents
            .get(info_hash)
            .ok_or(Error::VariantUnavailable)?
            .clone();
        Ok(ResolvedContent::TorrentFile {
            reference: selected.clone(),
            metadata,
        })
    }

    async fn release(&self, publication: &PublicationRef) -> Result<Release> {
        let id = publication.freetp_id()?;
        let url = self
            .origin
            .join(&format!("/index.php?newsid={id}"))
            .expect("validated publication ID");
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let (arrived, body) = match self.document(url, &mut session).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        let mut release = parse_release(&body, &self.origin, &arrived, publication)?;
        let candidates = {
            let document = Html::parse_document(&body);
            let content = unique(document.root_element(), &format!("#news-id-{id}"))?;
            let mut seen = HashSet::new();
            let mut candidates = Vec::new();

            for link in content.select(&selector("a[href]")) {
                let label = text(link);
                if label.is_empty() {
                    continue;
                }
                let Some(href) = link.value().attr("href") else {
                    continue;
                };
                let Some(file_id) = file_id(href, &self.origin) else {
                    continue;
                };
                if seen.insert(file_id) {
                    if candidates.len() == MAX_FILES {
                        return Err(Error::InvalidResponse("excessive FreeTP file links"));
                    }
                    let heading = link
                        .parent()
                        .and_then(ElementRef::wrap)
                        .and_then(|parent| {
                            parent.prev_siblings().filter_map(ElementRef::wrap).next()
                        })
                        .map(text)
                        .unwrap_or_default();
                    candidates.push((file_id, label, heading));
                }
            }
            candidates
        };

        for (file_id, label, heading) in candidates {
            let Some(filename) = self.landing(file_id, &mut session).await? else {
                continue;
            };
            let metadata = self.metadata(file_id, &mut session).await?;
            let hash = torrent::info_hash(metadata.bytes())?;
            if release.torrents.contains_key(&hash) {
                continue;
            }
            let title = variant_title(&label, &heading, &filename);
            release.work.variants.push(Variant {
                reference: VariantRef {
                    publication: publication.clone(),
                    selector: VariantSelector::TorrentHash {
                        info_hash: hash.clone(),
                    },
                },
                title: Some(title),
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
            release.torrents.insert(hash, metadata);
        }
        if release.work.variants.is_empty() {
            return Err(Error::UnsupportedAcquisition(
                "FreeTP publication has no available torrent files",
            ));
        }
        Ok(release)
    }

    async fn landing(&self, id: u64, session: &mut SourceSession) -> Result<Option<String>> {
        let url = self.origin.join(&format!("/getfile-{id}")).unwrap();
        let (arrived, body) = match self.document(url.clone(), session).await {
            Err(Error::HttpStatus(404 | 410)) => return Ok(None),
            result => result?,
        };
        if arrived != url {
            return Err(Error::InvalidResponse("FreeTP file identity changed"));
        }
        let document = Html::parse_document(&body);
        let title = text(unique(document.root_element(), "title")?);
        let filename = title
            .strip_suffix(" скачать бесплатно")
            .ok_or(Error::InvalidResponse("missing FreeTP file name"))?;
        if !filename.to_ascii_lowercase().ends_with(".torrent") {
            return Ok(None);
        }
        if filename.len() > 512 || filename.chars().any(char::is_control) {
            return Err(Error::InvalidResponse("invalid FreeTP file name"));
        }
        Ok(Some(filename.to_owned()))
    }

    async fn metadata(&self, id: u64, session: &mut SourceSession) -> Result<TorrentMetadata> {
        let referer = self.origin.join(&format!("/getfile-{id}")).unwrap();
        let url = self
            .origin
            .join(&format!("/engine/download.php?id={id}&area="))
            .unwrap();
        let response = self
            .http
            .session_referred_metadata(url.clone(), session, &referer)
            .await?;
        match response.status {
            200 if response.final_url == url => {}
            403 => return Err(Error::Challenge),
            404 | 410 => return Err(Error::VariantUnavailable),
            200 => return Err(Error::InvalidResponse("FreeTP file identity changed")),
            status => return Err(Error::HttpStatus(status)),
        }
        let content_type = response
            .content_type
            .as_deref()
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        if !content_type.is_some_and(|value| {
            [
                "application/force-download",
                "application/x-bittorrent",
                "application/octet-stream",
            ]
            .iter()
            .any(|mime| value.eq_ignore_ascii_case(mime))
        }) {
            return Err(Error::InvalidResponse(
                "FreeTP file is not torrent metadata",
            ));
        }
        TorrentMetadata::new(response.body)
    }

    async fn document(&self, url: Url, session: &mut SourceSession) -> Result<(Url, String)> {
        let response = self.http.session_document(url, session).await?;
        decode_document(response)
    }
}

fn decode_document(response: SourceDocument) -> Result<(Url, String)> {
    if response.status == 403 {
        return Err(Error::Challenge);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    let content_type = response.content_type.as_deref().unwrap_or_default();
    let parts = content_type.split(';').map(str::trim).collect::<Vec<_>>();
    if !parts[0].eq_ignore_ascii_case("text/html")
        || !parts[1..].iter().any(|part| {
            matches!(
                part.to_ascii_lowercase().as_str(),
                "charset=cp1251" | "charset=windows-1251"
            )
        })
    {
        return Err(Error::InvalidResponse("FreeTP response is not CP1251 HTML"));
    }
    let (body, invalid) = WINDOWS_1251.decode_without_bom_handling(&response.body);
    if invalid {
        return Err(Error::InvalidResponse("invalid FreeTP text encoding"));
    }
    Ok((response.final_url, body.into_owned()))
}

fn parse_search(body: &str, origin: &Url, query: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let form = unique(root, "#dle-content #fullsearch")?;
    let story = unique(form, "input[name='story']")?;
    let started = unique(form, "input[name='search_start']")?;
    if story.value().attr("value") != Some(query) || started.value().attr("value") != Some("1") {
        return Err(Error::InvalidResponse("FreeTP search was not confirmed"));
    }
    if text(unique(root, "#dle-content h1.heading3")?) != "Поиск по сайту" {
        return Err(Error::InvalidResponse("missing FreeTP search results"));
    }
    if text(unique(root, "#dle-content")?).contains("Вы сможете воспользоваться поиском через")
    {
        return Err(Error::InvalidResponse(
            "FreeTP search is temporarily limited",
        ));
    }

    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for link in document.select(&selector("#dle-content > .base > .heading > a[href]")) {
        if works.len() == 100 {
            return Err(Error::InvalidResponse("excessive FreeTP search results"));
        }
        let Some((reference, url)) = publication(link.value().attr("href").unwrap(), origin) else {
            continue;
        };
        if !seen.insert(reference.id.clone()) {
            return Err(Error::InvalidResponse("duplicate FreeTP publication"));
        }
        let title = text(link);
        if title.is_empty() || title.len() > 2048 {
            return Err(Error::InvalidResponse("invalid FreeTP publication title"));
        }
        works.push(Work {
            reference,
            aliases: game_aliases(&title),
            title,
            category: Category::Games,
            url,
            cover: None,
            description: None,
            year: None,
            identifiers: BTreeMap::new(),
            variants: Vec::new(),
            notes: Vec::new(),
            needs_inspection: true,
        });
    }
    Ok(works)
}

fn parse_release(
    body: &str,
    origin: &Url,
    url: &Url,
    expected: &PublicationRef,
) -> Result<Release> {
    let Some((arrived, canonical)) = publication(url.as_str(), origin) else {
        return Err(Error::VariantUnavailable);
    };
    if arrived != *expected {
        return Err(Error::VariantUnavailable);
    }
    let document = Html::parse_document(body);
    let root = document.root_element();
    let article = unique(root, "#dle-content > .base.fullstory")?;
    let title = text(unique(article, "h1")?);
    if title.is_empty() || title.len() > 2048 {
        return Err(Error::InvalidResponse("invalid FreeTP article title"));
    }
    let content = unique(article, &format!("#news-id-{}", expected.id))?;
    let description = content
        .select(&selector("p"))
        .map(text)
        .filter(|paragraph| !paragraph.is_empty())
        .take(80)
        .collect::<Vec<_>>()
        .join("\n\n");
    let description = (!description.is_empty()).then(|| description.chars().take(12_000).collect());
    Ok(Release {
        work: Work {
            reference: expected.clone(),
            aliases: game_aliases(&title),
            title,
            category: Category::Games,
            url: canonical,
            cover: None,
            description,
            year: None,
            identifiers: BTreeMap::new(),
            variants: Vec::new(),
            notes: Vec::new(),
            needs_inspection: false,
        },
        torrents: BTreeMap::new(),
    })
}

fn game_aliases(title: &str) -> Vec<String> {
    title
        .strip_suffix(" играть по сети и интернету Онлайн")
        .filter(|name| !name.is_empty())
        .map(|name| vec![name.to_owned()])
        .unwrap_or_default()
}

fn publication(input: &str, origin: &Url) -> Option<(PublicationRef, Url)> {
    let url = origin.join(input).ok()?;
    if url.scheme() != "https"
        || url.origin() != origin.origin()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let mut segments = url.path().trim_start_matches('/').split('/');
    let section = segments.next()?;
    if !matches!(section, "po-seti" | "singleplayer" | "games") {
        return None;
    }
    let filename = segments.next()?.strip_suffix(".html")?;
    if segments.next().is_some() {
        return None;
    }
    let (id, slug) = filename.split_once('-')?;
    if slug.is_empty() || slug.len() > 240 || !crate::model::canonical_decimal(id) {
        return None;
    }
    let reference = PublicationRef {
        source: Source::FreeTP,
        id: id.to_owned(),
    };
    Some((reference, url))
}

fn file_id(input: &str, origin: &Url) -> Option<u64> {
    let url = origin.join(input).ok()?;
    if url.scheme() != "https"
        || url.origin() != origin.origin()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let id = url.path().strip_prefix("/getfile-")?;
    if !crate::model::canonical_decimal(id) {
        return None;
    }
    id.parse().ok()
}

fn variant_title(label: &str, heading: &str, filename: &str) -> String {
    let context = format!("{heading} {label} {filename}").to_lowercase();
    let words = context
        .split(|character: char| !character.is_alphanumeric())
        .collect::<Vec<_>>();
    let has = |word| words.contains(&word);
    let kind = if has("fix") || has("фикс") || has("onlinefix") {
        "Онлайн-фикс"
    } else if has("patch") || has("update") || has("патч") || has("обновление") {
        "Патч"
    } else if has("dlc") || has("дополнение") || has("саундтрек") || has("soundtrack")
    {
        "Дополнение"
    } else if has("игру")
        || has("игра")
        || has("repack")
        || has("репак")
        || has("portable")
        || context.contains("steam-rip")
    {
        "Игра"
    } else {
        "Торрент"
    };
    let label = label.trim_start_matches("Скачать ").trim();
    let name = if label.len() < 8 { filename } else { label };
    format!("{kind}: {name}")
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
    Selector::parse(css).expect("fixed FreeTP selector")
}

fn unique<'a>(root: ElementRef<'a>, css: &str) -> Result<ElementRef<'a>> {
    let parsed = selector(css);
    let mut elements = root.select(&parsed);
    let first = elements
        .next()
        .ok_or(Error::InvalidResponse("missing FreeTP page metadata"))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous FreeTP page metadata"));
    }
    Ok(first)
}
