use std::collections::{BTreeMap, HashSet};

use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::browser::{BrowserChoice, BrowserError, BrowserHandle};
use crate::http::{SourceDocument, SourceHttp, SourceSession};
use crate::model::canonical_decimal;
use crate::torrent::info_hash;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, TorrentMetadata, Transport,
    Variant, VariantRef, VariantSelector, Work,
};

const ORIGIN: &str = "https://byrutgame.org";

/// ByRut serves nginx 404 to HTTP client identifiers without Mozilla/5.0.
/// Keep qbutt identifiable while requesting its public document endpoints.
const USER_AGENT_VALUE: &str = concat!("qbutt-content/", env!("CARGO_PKG_VERSION"), " Mozilla/5.0");

pub struct ByRut {
    http: SourceHttp,
    origin: Url,
}

impl ByRut {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed ByRut origin");
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        Ok(Self {
            http: SourceHttp::new(origin.clone(), headers)?,
            origin,
        })
    }

    pub async fn search(
        &self,
        query: &str,
        category: Category,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        if category != Category::Games {
            return Ok(Vec::new());
        }

        let mut url = self.origin.join("/index.php").expect("fixed search path");
        url.query_pairs_mut()
            .append_pair("do", "search")
            .append_pair("subaction", "search")
            .append_pair("story", query);
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let body = self.document(url, &mut session, browser, choice).await?;
        search(&body, &self.origin, query)
    }

    pub async fn inspect(
        &self,
        publication: &PublicationRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Work> {
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let mut release = self
            .article(publication, &mut session, browser, choice)
            .await?;
        for torrent in &release.torrents {
            let metadata = self
                .torrent(&release.work, torrent, &mut session, browser, choice)
                .await?;
            let hash = info_hash(metadata.bytes())?;
            if release
                .work
                .variants
                .iter()
                .any(|variant| variant.info_hash.as_deref() == Some(hash.as_str()))
            {
                continue;
            }
            release.work.variants.push(Variant {
                reference: VariantRef {
                    publication: release.work.reference.clone(),
                    selector: VariantSelector::TorrentHash {
                        info_hash: hash.clone(),
                    },
                },
                title: torrent.label.clone(),
                transport: Transport::Torrent,
                episode: None,
                quality: None,
                translation: None,
                languages: Vec::new(),
                player: None,
                size: None,
                seeders: None,
                info_hash: Some(hash),
                reported_video_ads: false,
            });
        }
        Ok(release.work)
    }

    /// A replacement torrent on the same article cannot silently replace the
    /// selected infohash. Acquired bytes go only to the native torrent handoff.
    pub async fn resolve(
        &self,
        selected: &VariantRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<ResolvedContent> {
        selected.validate()?;
        let VariantSelector::TorrentHash {
            info_hash: expected,
        } = &selected.selector
        else {
            return Err(Error::InvalidReference);
        };
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let release = self
            .article(&selected.publication, &mut session, browser, choice)
            .await?;
        for torrent in &release.torrents {
            let metadata = self
                .torrent(&release.work, torrent, &mut session, browser, choice)
                .await?;
            if info_hash(metadata.bytes())? == *expected {
                return Ok(ResolvedContent::TorrentFile {
                    reference: selected.clone(),
                    metadata,
                });
            }
        }
        Err(Error::VariantUnavailable)
    }

    async fn article(
        &self,
        publication: &PublicationRef,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<ParsedRelease> {
        let slug = publication.byrut_slug()?;
        let url = self
            .origin
            .join(&format!("/{slug}.html"))
            .expect("validated ByRut slug");
        let body = match self.document(url, session, browser, choice).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        release(&body, &self.origin, publication)
    }

    async fn torrent(
        &self,
        work: &Work,
        torrent: &TorrentRelease,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<TorrentMetadata> {
        let download = match &torrent.link {
            TorrentLink::Direct(url) => url.clone(),
            TorrentLink::Ajax(block) => {
                let url = self
                    .origin
                    .join("/engine/ajax/controller.php?mod=ajaxsp")
                    .expect("fixed ByRut AJAX path");
                let (id, _) = work
                    .reference
                    .id
                    .split_once('-')
                    .ok_or(Error::InvalidReference)?;
                let form = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("block", block)
                    .append_pair("id", id)
                    .finish();
                let response = self
                    .http
                    .session_ajax_document(url.clone(), &form, session)
                    .await?;
                let body = match html_response(response, &url) {
                    Err(Error::Challenge) => {
                        self.borrow_browser(session, browser, choice, work.url.clone())
                            .await?;
                        html_response(
                            self.http
                                .session_ajax_document(url.clone(), &form, session)
                                .await?,
                            &url,
                        )?
                    }
                    result => result?,
                };
                let document = Html::parse_fragment(&body);
                let link = unique(
                    document.root_element(),
                    "a.packagedownld-spoiler__action[href]",
                )?;
                torrent_link(required(link, "href")?, &self.origin)?.1
            }
        };
        let response = self
            .http
            .session_metadata(download.clone(), session, None)
            .await?;
        match torrent_response(response, &download) {
            Err(Error::Challenge) => {
                self.borrow_browser(session, browser, choice, work.url.clone())
                    .await?;
                let response = self
                    .http
                    .session_metadata(download.clone(), session, None)
                    .await?;
                torrent_response(response, &download)
            }
            result => result,
        }
    }

    async fn document(
        &self,
        url: Url,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<String> {
        let response = self.http.session_document(url.clone(), session).await?;
        match html_response(response, &url) {
            Err(Error::Challenge) => {
                self.borrow_browser(session, browser, choice, url.clone())
                    .await?;
                html_response(
                    self.http.session_document(url.clone(), session).await?,
                    &url,
                )
            }
            result => result,
        }
    }

    async fn borrow_browser(
        &self,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
        scope: Url,
    ) -> Result<()> {
        let Some(browser) = browser else {
            return Err(Error::Challenge);
        };
        let borrowed = match browser.session_connected(choice, scope).await {
            Ok(session) => session,
            Err(
                BrowserError::BrowserRequired | BrowserError::Disconnected | BrowserError::Stopped,
            ) => {
                return Err(Error::Challenge);
            }
            Err(BrowserError::ChoiceRequired) => return Err(Error::BrowserChoiceRequired),
            Err(error) => return Err(Error::BrowserSession(error)),
        };
        *session = SourceSession::from_browser(self.origin.clone(), borrowed)?;
        Ok(())
    }
}

fn html_response(response: SourceDocument, expected: &Url) -> Result<String> {
    if response.status == 403 || response.status == 503 {
        return Err(Error::Challenge);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if response.final_url != *expected {
        return Err(Error::InvalidResponse("ByRut document identity changed"));
    }
    if !response
        .content_type
        .as_deref()
        .is_some_and(|value| value.starts_with("text/html"))
    {
        return Err(Error::InvalidResponse("ByRut document is not HTML"));
    }
    let body = String::from_utf8(response.body)
        .map_err(|_| Error::InvalidResponse("ByRut document is not UTF-8"))?;
    if body.contains("Just a moment") || body.contains("cf-challenge") {
        return Err(Error::Challenge);
    }
    Ok(body)
}

fn torrent_response(response: SourceDocument, expected: &Url) -> Result<TorrentMetadata> {
    if response.status == 403 || response.status == 503 {
        return Err(Error::Challenge);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if response.final_url != *expected {
        return Err(Error::InvalidResponse("ByRut download identity changed"));
    }
    if !response.content_type.as_deref().is_some_and(|value| {
        value.split(';').next().is_some_and(|mime| {
            mime.eq_ignore_ascii_case("application/x-bittorrent")
                || mime.eq_ignore_ascii_case("application/octet-stream")
        })
    }) {
        return Err(Error::InvalidResponse(
            "ByRut download is not torrent metadata",
        ));
    }
    TorrentMetadata::new(response.body)
}

fn search(body: &str, origin: &Url, query: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let search = unique(root, "#fullsearch input[name='story']")?;
    if search.attr("value") != Some(query) {
        return Err(Error::InvalidResponse("ByRut search query changed"));
    }
    let mut seen = HashSet::new();
    let mut works = Vec::new();
    for row in root.select(&selector("#dle-content a.search_res[href]")) {
        if seen.len() >= 100 {
            return Err(Error::InvalidResponse("excessive ByRut search results"));
        }
        let (reference, url) = publication(required(row, "href")?, origin)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate ByRut publication"));
        }
        if let Some(status) = optional(row, ".status-badge--ann")?
            && checked_text(status)? == "Нет раздачи"
        {
            continue;
        }
        let title = checked_text(unique(row, ".search_res_title")?)?;
        let mut work = work(reference, url, title, true);
        work.description = optional(row, ".search_res_sub .js-ver")?
            .map(checked_text)
            .transpose()?;
        work.cover = optional(row, ".search_res_img img[src]")?
            .map(|image| image_url(required(image, "src")?, origin))
            .transpose()?;
        works.push(work);
    }
    if seen.is_empty() {
        let message = optional(root, ".search-page__feedback .berrors_text")?
            .map(checked_text)
            .transpose()?;
        if !message
            .is_some_and(|message| message.starts_with("По вашему запросу ничего не найдено"))
        {
            return Err(Error::InvalidResponse("missing ByRut search results"));
        }
    }
    Ok(works)
}

struct ParsedRelease {
    work: Work,
    torrents: Vec<TorrentRelease>,
}

struct TorrentRelease {
    link: TorrentLink,
    label: Option<String>,
}

enum TorrentLink {
    Direct(Url),
    Ajax(String),
}

fn release(body: &str, origin: &Url, expected: &PublicationRef) -> Result<ParsedRelease> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let canonical = unique(root, "link[rel='canonical'][href]")?;
    let (reference, url) = publication(required(canonical, "href")?, origin)?;
    if reference != *expected {
        return Err(Error::VariantUnavailable);
    }
    let article = unique(root, "article.main_content[data-news-id]")?;
    let (number, _) = expected.id.split_once('-').ok_or(Error::InvalidReference)?;
    if article.attr("data-news-id") != Some(number) {
        return Err(Error::VariantUnavailable);
    }
    let title = checked_text(unique(article, ".game_details h1")?)?;
    let mut work = work(reference, url, title, false);
    work.cover = optional(article, ".poster-imgbox img[src]")?
        .map(|image| image_url(required(image, "src")?, origin))
        .transpose()?;
    work.description = optional(article, ".game_desc")?
        .map(checked_text)
        .transpose()?
        .map(|text| text.chars().take(16_000).collect());

    let main = unique(article, ".main-release-block")?;
    let version = optional(main, ".info-basictor__item--version .info-basictor__value")?
        .map(checked_text)
        .transpose()?;
    let status = optional(main, ".info-basictor__item--status .info-basictor__value")?
        .map(checked_text)
        .transpose()?;
    let release_type = optional(
        main,
        ".info-basictor__row .info-basictor__item:first-child .info-basictor__value",
    )?
    .map(checked_text)
    .transpose()?;
    let mut torrents = Vec::new();
    if let Some(link) = optional(main, "a.itemdown_games[href]")? {
        let (id, download) = torrent_link(required(link, "href")?, origin)?;
        if let Some(top) = optional(article, "a.itemtop_games[href]")?
            && torrent_link(required(top, "href")?, origin)?.0 != id
        {
            return Err(Error::InvalidResponse("ByRut download IDs differ"));
        }
        let label = [
            release_type.as_deref(),
            version.as_deref(),
            status.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" — ");
        torrents.push(TorrentRelease {
            link: TorrentLink::Direct(download),
            label: (!label.is_empty()).then_some(label),
        });
    } else {
        let notice = optional(article, ".tupd-text")?
            .map(checked_text)
            .transpose()?;
        if notice.as_deref() != Some("Торрента пока нет")
            && status.as_deref() != Some("Раздача из файлового хранилища (без торрента).")
        {
            return Err(Error::InvalidResponse("missing ByRut torrent link"));
        }
    }

    let mut blocks = HashSet::new();
    for block in article.select(&selector("#other_repacks .ajax-block[data-ajax]")) {
        if optional(block, ".showtitle[data-storage]")?.is_some() {
            continue;
        }
        let name = required(block, "data-ajax")?;
        if !name.strip_prefix("features").is_some_and(canonical_decimal)
            || !blocks.insert(name)
            || blocks.len() > 16
        {
            return Err(Error::InvalidResponse("invalid ByRut alternate block"));
        }
        let content = unique(block, ".ajax-content[id]")?;
        if content.attr("id") != Some(format!("ajaxsp-{name}-{number}").as_str()) {
            return Err(Error::InvalidResponse("ByRut alternate identity changed"));
        }
        let title = checked_text(unique(block, ".showtitle")?)?;
        let version = checked_text(unique(block, ".show_ver.torrent")?)?;
        torrents.push(TorrentRelease {
            link: TorrentLink::Ajax(name.to_owned()),
            label: Some(format!("{title} — {version}")),
        });
    }
    if torrents.is_empty() {
        return Err(Error::UnsupportedAcquisition(
            "ByRut publication has no available torrent file",
        ));
    }
    Ok(ParsedRelease { work, torrents })
}

fn publication(input: &str, origin: &Url) -> Result<(PublicationRef, Url)> {
    let url = site_url(input, origin)?;
    if url.query().is_some() {
        return Err(Error::InvalidReference);
    }
    let slug = url
        .path()
        .strip_prefix('/')
        .and_then(|path| path.strip_suffix(".html"))
        .ok_or(Error::InvalidReference)?;
    let reference = PublicationRef {
        source: Source::ByRut,
        id: slug.into(),
    };
    reference.validate()?;
    Ok((reference, url))
}

fn torrent_link(input: &str, origin: &Url) -> Result<(u64, Url)> {
    let url = site_url(input, origin)?;
    let mut fields = url.query_pairs();
    let valid = url.path() == "/index.php"
        && fields
            .next()
            .as_ref()
            .is_some_and(|(name, value)| name == "do" && value == "download");
    let id = fields.next();
    if !valid || fields.next().is_some() {
        return Err(Error::InvalidResponse("invalid ByRut download URL"));
    }
    let Some((name, id)) = id else {
        return Err(Error::InvalidResponse("missing ByRut download ID"));
    };
    if name != "id" || !canonical_decimal(&id) {
        return Err(Error::InvalidResponse("invalid ByRut download ID"));
    }
    Ok((
        id.parse()
            .map_err(|_| Error::InvalidResponse("ByRut download ID overflow"))?,
        url,
    ))
}

fn site_url(input: &str, origin: &Url) -> Result<Url> {
    let url = origin
        .join(input)
        .map_err(|_| Error::InvalidResponse("invalid ByRut URL"))?;
    if url.origin() != origin.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.scheme() != "https"
    {
        return Err(Error::InvalidResponse("foreign ByRut URL"));
    }
    Ok(url)
}

fn image_url(input: &str, origin: &Url) -> Result<Url> {
    let url = origin
        .join(input)
        .map_err(|_| Error::InvalidResponse("invalid ByRut cover"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("img.byrutgame.org")
        || url.query().is_some()
    {
        return Err(Error::InvalidResponse("foreign ByRut cover"));
    }
    Ok(url)
}

fn work(reference: PublicationRef, url: Url, title: String, needs_inspection: bool) -> Work {
    let aliases = if reference.id.contains("-po-seti-") {
        title
            .strip_suffix(" по сети")
            .filter(|name| !name.is_empty())
            .map(|name| vec![name.to_owned()])
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Work {
        reference,
        title,
        category: Category::Games,
        url,
        cover: None,
        description: None,
        year: None,
        aliases,
        identifiers: BTreeMap::new(),
        variants: Vec::new(),
        needs_inspection,
    }
}

fn checked_text(element: ElementRef<'_>) -> Result<String> {
    let text = element.text().collect::<Vec<_>>().join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || text.len() > 16_384 || text.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid ByRut text"));
    }
    Ok(text)
}

fn required<'a>(element: ElementRef<'a>, attribute: &str) -> Result<&'a str> {
    element
        .attr(attribute)
        .ok_or(Error::InvalidResponse("missing ByRut attribute"))
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed ByRut selector")
}

fn optional<'a>(root: ElementRef<'a>, value: &str) -> Result<Option<ElementRef<'a>>> {
    let selector = selector(value);
    let mut elements = root.select(&selector);
    let first = elements.next();
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous ByRut metadata"));
    }
    Ok(first)
}

fn unique<'a>(root: ElementRef<'a>, value: &str) -> Result<ElementRef<'a>> {
    optional(root, value)?.ok_or(Error::InvalidResponse("missing ByRut metadata"))
}
