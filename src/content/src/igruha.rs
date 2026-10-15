use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use encoding_rs::WINDOWS_1251;
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::browser::{BrowserChoice, BrowserError, BrowserHandle};
use crate::http::{SourceDocument, SourceHttp, SourceSession};
use crate::model::canonical_decimal;
use crate::torrent::info_hash;
use crate::webview::{WebViewCheck, WebViewError, WebViewSessions};
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, TorrentMetadata, Transport,
    Variant, VariantRef, VariantSelector, Work,
};

const ORIGIN: &str = "https://itorrents-igruha.org";
const SESSION_LIFETIME: Duration = Duration::from_secs(30 * 60);

struct Clearance {
    received: Instant,
    session: SourceSession,
}

/// The public DLE catalog first borrows a connected browser's session after a
/// challenge, then checks its own WebView profile before requesting manual input.
pub struct Igruha {
    http: SourceHttp,
    origin: Url,
    webview: OnceLock<WebViewSessions>,
    clearance: Mutex<Option<Clearance>>,
}

struct Release {
    work: Work,
    metadata: Vec<TorrentMetadata>,
}

struct Download {
    url: Url,
    title: Option<String>,
    size: u64,
}

struct ReleasePage {
    work: Work,
    downloads: Vec<Download>,
}

impl Igruha {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Igruha origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?,
            origin,
            webview: OnceLock::new(),
            clearance: Mutex::new(None),
        })
    }

    pub fn initialize_webview(&self, profile_dir: PathBuf, app_dir: PathBuf) {
        let _ = self.webview.set(WebViewSessions::new(profile_dir, app_dir));
    }

    pub fn close(&self) {
        self.clearance.lock().expect("Igruha clearance lock").take();
        if let Some(webview) = self.webview.get() {
            webview.close();
        }
    }

    /// Only an explicit source retry may show the profile used for silent checks.
    pub async fn complete_check(&self) -> Result<()> {
        self.checked_session(self.origin.clone(), true).await?;
        Ok(())
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
        let mut session = self.source_session()?;
        let mut result = self
            .document(url.clone(), &mut session, browser, choice)
            .await;
        if matches!(result, Err(Error::Challenge)) {
            session = self.checked_session(url.clone(), false).await?;
            result = self.document(url, &mut session, None, choice).await;
        }
        let body = match result {
            Err(Error::Challenge) => {
                self.clearance.lock().expect("Igruha clearance lock").take();
                return Err(Error::SourceCheckRequired);
            }
            result => result?,
        };
        search_results(&body, &self.origin, query)
    }

    pub async fn inspect(
        &self,
        publication: &PublicationRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Work> {
        Ok(self.release(publication, browser, choice).await?.work)
    }

    /// Re-fetch the article and the selected torrent. A replaced download on
    /// the same article must never become the previously selected variant.
    pub async fn resolve(
        &self,
        selected: &VariantRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<ResolvedContent> {
        selected.validate()?;
        let release = self.release(&selected.publication, browser, choice).await?;
        let metadata = release
            .work
            .variants
            .iter()
            .position(|variant| variant.reference == *selected)
            .and_then(|index| release.metadata.into_iter().nth(index))
            .ok_or(Error::VariantUnavailable)?;
        Ok(ResolvedContent::TorrentFile {
            reference: selected.clone(),
            metadata,
        })
    }

    async fn release(
        &self,
        publication: &PublicationRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Release> {
        let slug = publication.igruha_slug()?;
        let url = self
            .origin
            .join(&format!("/{slug}.html"))
            .expect("validated Igruha slug");
        let mut session = self.source_session()?;
        let result = self
            .release_request(publication, url.clone(), &mut session, browser, choice)
            .await;
        if !matches!(result, Err(Error::Challenge)) {
            return result;
        }
        session = self.checked_session(url.clone(), false).await?;
        match self
            .release_request(publication, url, &mut session, None, choice)
            .await
        {
            Err(Error::Challenge) => {
                self.clearance.lock().expect("Igruha clearance lock").take();
                Err(Error::SourceCheckRequired)
            }
            result => result,
        }
    }

    async fn release_request(
        &self,
        publication: &PublicationRef,
        url: Url,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Release> {
        let body = match self.document(url.clone(), session, browser, choice).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        let mut page = release_page(&body, &self.origin, publication)?;
        let mut metadata = Vec::new();
        let mut hashes = HashSet::new();
        for download in page.downloads {
            let landing = match self
                .document(download.url.clone(), session, browser, choice)
                .await
            {
                Err(Error::HttpStatus(404 | 410)) => continue,
                result => result?,
            };
            let metadata_url = metadata_url(&landing, &self.origin, &url, &download.url)?;
            let response = self
                .http
                .session_referred_metadata(metadata_url.clone(), session, &download.url)
                .await?;
            let torrent = match torrent_response(response, &metadata_url) {
                Err(Error::HttpStatus(404 | 410)) => continue,
                Err(Error::Challenge) => {
                    self.borrow_browser(session, browser, choice, download.url.clone())
                        .await?;
                    let response = self
                        .http
                        .session_referred_metadata(metadata_url.clone(), session, &download.url)
                        .await?;
                    match torrent_response(response, &metadata_url) {
                        Err(Error::HttpStatus(404 | 410)) => continue,
                        result => result?,
                    }
                }
                result => result?,
            };
            let hash = info_hash(torrent.bytes())?;
            if !hashes.insert(hash.clone()) {
                continue;
            }
            page.work.variants.push(Variant {
                reference: VariantRef {
                    publication: publication.clone(),
                    selector: VariantSelector::TorrentHash {
                        info_hash: hash.clone(),
                    },
                },
                title: download.title,
                transport: Transport::Torrent,
                episode: None,
                quality: None,
                translation: None,
                languages: Vec::new(),
                player: None,
                size: Some(download.size),
                seeders: None,
                info_hash: Some(hash),
                reported_video_ads: false,
            });
            metadata.push(torrent);
        }
        if page.work.variants.is_empty() {
            return Err(Error::VariantUnavailable);
        }
        Ok(Release {
            work: page.work,
            metadata,
        })
    }

    fn source_session(&self) -> Result<SourceSession> {
        let mut clearance = self.clearance.lock().expect("Igruha clearance lock");
        if clearance
            .as_ref()
            .is_some_and(|cached| cached.received.elapsed() >= SESSION_LIFETIME)
        {
            clearance.take();
        }
        match clearance.as_ref() {
            Some(cached) => Ok(cached.session.clone()),
            None => SourceSession::anonymous(self.origin.clone()),
        }
    }

    async fn checked_session(&self, url: Url, interactive: bool) -> Result<SourceSession> {
        self.clearance.lock().expect("Igruha clearance lock").take();
        let webview = self
            .webview
            .get()
            .ok_or(Error::SourceCheck(WebViewError::Unavailable))?;
        let checked = webview
            .check(
                url,
                WebViewCheck {
                    interactive,
                    ..WebViewCheck::default()
                },
            )
            .await
            .map_err(|error| match error {
                WebViewError::CheckRequired => Error::SourceCheckRequired,
                WebViewError::Canceled | WebViewError::Timeout if interactive => {
                    Error::SourceCheckRequired
                }
                error => Error::SourceCheck(error),
            })?;
        let session = SourceSession::from_webview(self.origin.clone(), checked)?;
        *self.clearance.lock().expect("Igruha clearance lock") = Some(Clearance {
            received: Instant::now(),
            session: session.clone(),
        });
        Ok(session)
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
                BrowserError::BrowserRequired
                | BrowserError::Disconnected
                | BrowserError::Stopped
                | BrowserError::ChoiceRequired
                | BrowserError::VerificationRequired
                | BrowserError::BrowserCheckRequired
                | BrowserError::LoginRequired,
            ) => return Err(Error::Challenge),
            Err(error) => return Err(Error::BrowserSession(error)),
        };
        *session = SourceSession::from_browser(self.origin.clone(), borrowed)?;
        Ok(())
    }
}

fn html_response(response: SourceDocument, expected: &Url) -> Result<String> {
    if matches!(response.status, 403 | 503) {
        return Err(Error::Challenge);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if response.final_url != *expected {
        return Err(Error::InvalidResponse("Igruha document identity changed"));
    }
    if !response
        .content_type
        .as_deref()
        .is_some_and(|value| value.starts_with("text/html"))
    {
        return Err(Error::InvalidResponse("Igruha document is not HTML"));
    }
    let (body, _) = WINDOWS_1251.decode_without_bom_handling(&response.body);
    if body.contains("__ddg") || body.contains("DDoS-Guard") {
        return Err(Error::Challenge);
    }
    Ok(body.into_owned())
}

fn torrent_response(response: SourceDocument, expected: &Url) -> Result<TorrentMetadata> {
    if matches!(response.status, 403 | 503) {
        return Err(Error::Challenge);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if response.final_url != *expected {
        return Err(Error::InvalidResponse("Igruha torrent identity changed"));
    }
    if response
        .content_type
        .as_deref()
        .is_some_and(|value| value.starts_with("text/html"))
    {
        if response.body.windows(5).any(|window| window == b"__ddg")
            || response
                .body
                .windows(10)
                .any(|window| window == b"DDoS-Guard")
        {
            return Err(Error::Challenge);
        }
        return Err(Error::InvalidResponse("Igruha torrent is not metadata"));
    }
    TorrentMetadata::new(response.body)
}

fn search_results(body: &str, origin: &Url, query: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let input = unique(root, "#dle-content #fullsearch #searchinput[name='story']")?;
    if input.attr("value") != Some(query) {
        return Err(Error::InvalidResponse("Igruha search query changed"));
    }
    unique(root, "#dle-content .searchpage")?;

    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for row in root.select(&selector("#dle-content .game-card")) {
        if works.len() >= 100 {
            return Err(Error::InvalidResponse("excessive Igruha search results"));
        }
        let link = unique(row, ".game-card__name a[href]")?;
        let (reference, url) = publication(required(link, "href")?, origin)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate Igruha publication"));
        }
        let mut work = work(reference, url, checked_text(link)?, true);
        work.cover = optional(row, ".game-card__cover img.game-cover[src]")?
            .map(|image| cover_url(required(image, "src")?, origin))
            .transpose()?;
        works.push(work);
    }
    Ok(works)
}

fn release_page(body: &str, origin: &Url, expected: &PublicationRef) -> Result<ReleasePage> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let canonical = unique(root, "link[rel='canonical'][href]")?;
    let (reference, url) = publication(required(canonical, "href")?, origin)?;
    if reference != *expected {
        return Err(Error::VariantUnavailable);
    }
    let article = unique(root, "#dle-content")?;
    let title = checked_text(unique(article, ".page-title > h1")?)?;
    let mut work = work(reference, url, title, false);
    work.cover = optional(article, "#entry-poster-bg img.entry-image[src]")?
        .map(|image| cover_url(required(image, "src")?, origin))
        .transpose()?;
    let update = optional(article, ".updategame")?
        .map(checked_text)
        .transpose()?;

    let mut downloads = Vec::new();
    for label in article.select(&selector(".blockinfo > .is-centered > .size-line")) {
        if downloads.len() >= 8 {
            return Err(Error::InvalidResponse("excessive Igruha torrents"));
        }
        let wrapper = label
            .parent()
            .and_then(ElementRef::wrap)
            .ok_or(Error::InvalidResponse("missing Igruha release wrapper"))?;
        let sibling = wrapper
            .next_siblings()
            .find_map(ElementRef::wrap)
            .ok_or(Error::InvalidResponse("missing Igruha download block"))?;
        if !sibling.value().classes().any(|class| class == "dl-bar") {
            return Err(Error::InvalidResponse("Igruha download block changed"));
        }
        let link = unique(sibling, "a.torrent[href]")?;
        let download = download_url(required(link, "href")?, origin)?;
        let (size, title) = release_label(label, update.as_deref())?;
        downloads.push(Download {
            url: download,
            title,
            size,
        });
    }
    if downloads.is_empty() {
        return Err(Error::VariantUnavailable);
    }
    Ok(ReleasePage { work, downloads })
}

fn release_label(label: ElementRef<'_>, update: Option<&str>) -> Result<(u64, Option<String>)> {
    let value = checked_text(label)?;
    let value = value
        .strip_prefix("Размер:")
        .ok_or(Error::InvalidResponse("missing Igruha release size"))?
        .trim();
    let mut parts = value.splitn(3, ' ');
    let amount = parts
        .next()
        .and_then(|value| value.replace(',', ".").parse::<f64>().ok())
        .ok_or(Error::InvalidResponse("invalid Igruha release size"))?;
    let unit = parts
        .next()
        .ok_or(Error::InvalidResponse("missing Igruha release unit"))?;
    let multiplier = match unit {
        "KB" => 1024_u64,
        "MB" => 1024_u64.pow(2),
        "GB" => 1024_u64.pow(3),
        "TB" => 1024_u64.pow(4),
        _ => return Err(Error::InvalidResponse("invalid Igruha release unit")),
    };
    let bytes = amount * multiplier as f64;
    if !bytes.is_finite() || !(1.0..=u64::MAX as f64).contains(&bytes) {
        return Err(Error::InvalidResponse("invalid Igruha release size"));
    }
    let label = parts
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches('|')
        .trim();
    let mut title = label.to_owned();
    if !title.bytes().any(|byte| byte.is_ascii_digit()) {
        if let Some(update) = update.and_then(|text| text.strip_prefix("Изменения:")) {
            let update = update.trim();
            if !update.is_empty() {
                if !title.is_empty() {
                    title.push_str(" | ");
                }
                title.push_str(update);
            }
        }
    }
    Ok((bytes as u64, (!title.is_empty()).then_some(title)))
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
        source: Source::Igruha,
        id: slug.into(),
    };
    reference.validate()?;
    Ok((reference, url))
}

fn download_url(input: &str, origin: &Url) -> Result<Url> {
    let url = site_url(input, origin)?;
    let mut fields = url.query_pairs();
    let valid = url.path() == "/"
        && fields
            .next()
            .as_ref()
            .is_some_and(|(name, value)| name == "do" && value == "download");
    let id = fields.next();
    if !valid || fields.next().is_some() {
        return Err(Error::InvalidResponse("invalid Igruha torrent URL"));
    }
    let Some((name, id)) = id else {
        return Err(Error::InvalidResponse("missing Igruha torrent ID"));
    };
    if name != "id" || !canonical_decimal(&id) {
        return Err(Error::InvalidResponse("invalid Igruha torrent ID"));
    }
    Ok(url)
}

fn metadata_url(body: &str, origin: &Url, article: &Url, landing: &Url) -> Result<Url> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let back = unique(root, "#dle-content .dl-back a[href]")?;
    if site_url(required(back, "href")?, origin)? != *article {
        return Err(Error::VariantUnavailable);
    }
    let link = unique(root, "#dle-content #btn-download[href]")?;
    let url = site_url(required(link, "href")?, origin)?;
    let id = landing
        .query_pairs()
        .find(|(name, _)| name == "id")
        .map(|(_, value)| value)
        .expect("validated Igruha download URL");
    let mut fields = url.query_pairs();
    if url.path() != "/engine/download.php"
        || !fields
            .next()
            .as_ref()
            .is_some_and(|(name, value)| name == "id" && value == &id)
        || fields.next().is_some()
    {
        return Err(Error::InvalidResponse("invalid Igruha metadata URL"));
    }
    Ok(url)
}

fn site_url(input: &str, origin: &Url) -> Result<Url> {
    let url = origin
        .join(input)
        .map_err(|_| Error::InvalidResponse("invalid Igruha URL"))?;
    if url.origin() != origin.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.scheme() != "https"
    {
        return Err(Error::InvalidResponse("foreign Igruha URL"));
    }
    Ok(url)
}

fn cover_url(input: &str, origin: &Url) -> Result<Url> {
    let url = site_url(input, origin)?;
    if !url.path().starts_with("/uploads/posts/") || url.query().is_some() {
        return Err(Error::InvalidResponse("invalid Igruha cover"));
    }
    Ok(url)
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
    let text = element.text().collect::<Vec<_>>().join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || text.len() > 16_384 || text.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid Igruha text"));
    }
    Ok(text)
}

fn required<'a>(element: ElementRef<'a>, attribute: &str) -> Result<&'a str> {
    element
        .attr(attribute)
        .ok_or(Error::InvalidResponse("missing Igruha attribute"))
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed Igruha selector")
}

fn optional<'a>(root: ElementRef<'a>, value: &str) -> Result<Option<ElementRef<'a>>> {
    let selector = selector(value);
    let mut elements = root.select(&selector);
    let first = elements.next();
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous Igruha metadata"));
    }
    Ok(first)
}

fn unique<'a>(root: ElementRef<'a>, value: &str) -> Result<ElementRef<'a>> {
    optional(root, value)?.ok_or(Error::InvalidResponse("missing Igruha metadata"))
}
