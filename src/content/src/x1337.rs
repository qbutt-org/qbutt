use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::browser::{BrowserChoice, BrowserError, BrowserHandle, FetchInit};
use crate::http::{SourceDocument, SourceHttp, SourceSession, browser_document};
use crate::torrent::magnet;
use crate::webview::{WebViewCheck, WebViewError, WebViewSessions};
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, SourceOperation, Transport,
    Variant, VariantRef, VariantSelector, Work,
};

const ORIGIN: &str = "https://1337x.to/";
const SESSION_LIFETIME: Duration = Duration::from_secs(30 * 60);

struct Clearance {
    received: Instant,
    session: SourceSession,
}

/// Public 1337x metadata. The source's own magnet is the only acquisition;
/// advertised players, installers and external torrent mirrors are ignored.
pub struct X1337 {
    http: SourceHttp,
    origin: Url,
    webview: OnceLock<WebViewSessions>,
    clearance: Mutex<Option<Clearance>>,
}

struct OperationSession {
    http: SourceSession,
    use_browser: bool,
}

struct Release {
    work: Work,
    magnet: Url,
}

impl X1337 {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed 1337x origin");
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
        self.clearance.lock().expect("1337x clearance lock").take();
        if let Some(webview) = self.webview.get() {
            webview.close();
        }
    }

    /// Only an explicit source retry may show the profile used for silent checks.
    pub async fn complete_check(&self, operation: &SourceOperation) -> Result<()> {
        if operation.source() != Source::X1337 {
            return Err(Error::InvalidReference);
        }
        operation.validate()?;
        let url = match operation {
            SourceOperation::Search { query, .. } => self.search_entry(&search_query(query)?),
            SourceOperation::Inspect { reference, .. } => self.publication_url(reference)?,
            SourceOperation::Resolve(reference) => self.publication_url(&reference.publication)?,
            _ => return Err(Error::InvalidReference),
        };
        self.checked_session(url, true).await?;
        Ok(())
    }

    pub async fn search(
        &self,
        query: &str,
        category: Category,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Vec<Work>> {
        let query = search_query(query)?;
        let entry = self.search_entry(&query);
        let mut session = self.source_session()?;
        let result = self
            .search_request(
                &query,
                category,
                entry.clone(),
                &mut session,
                browser,
                choice,
            )
            .await;
        if !matches!(result, Err(Error::Challenge)) {
            return result;
        }
        session = self.checked_session(entry.clone(), false).await?;
        match self
            .search_request(&query, category, entry, &mut session, None, choice)
            .await
        {
            Err(Error::Challenge) => {
                self.clearance.lock().expect("1337x clearance lock").take();
                Err(Error::SourceCheckRequired)
            }
            result => result,
        }
    }

    fn search_entry(&self, query: &str) -> Url {
        // /srch establishes the site's search session before category requests.
        let mut entry = self.origin.join("srch").expect("fixed search form path");
        entry.query_pairs_mut().append_pair("search", query);
        entry
    }

    async fn search_request(
        &self,
        query: &str,
        category: Category,
        entry: Url,
        session: &mut OperationSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Vec<Work>> {
        let category_name = match category {
            Category::Movies => "Movies",
            Category::Tv => "TV",
            Category::Anime => "Anime",
            Category::Games => "Games",
            Category::Software => "Apps",
            Category::Books => "Other",
            Category::Manga | Category::Video => return Ok(Vec::new()),
        };
        let (arrived, body) = self.document(entry, session, browser, choice).await?;
        if search_is_empty(&arrived, &body, &self.origin, query)? {
            return Ok(Vec::new());
        }

        let mut url = self.origin.clone();
        url.path_segments_mut()
            .expect("fixed HTTPS origin")
            .extend(["category-search", query, category_name, "1", ""]);
        let (arrived, body) = self.document(url.clone(), session, browser, choice).await?;
        if arrived != url {
            return Err(Error::InvalidResponse(
                "1337x category search identity changed",
            ));
        }
        search_results(&body, &self.origin, query, category)
    }

    pub async fn inspect(
        &self,
        publication: &PublicationRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Work> {
        Ok(self.release(publication, browser, choice).await?.work)
    }

    pub async fn resolve(
        &self,
        selected: &VariantRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<ResolvedContent> {
        selected.validate()?;
        let release = self.release(&selected.publication, browser, choice).await?;
        if release.work.variants[0].reference != *selected {
            return Err(Error::VariantUnavailable);
        }
        Ok(ResolvedContent::Torrent {
            reference: selected.clone(),
            title: release.work.title,
            magnet: release.magnet,
        })
    }

    async fn release(
        &self,
        publication: &PublicationRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Release> {
        let url = self.publication_url(publication)?;
        let mut session = self.source_session()?;
        let mut result = self
            .document(url.clone(), &mut session, browser, choice)
            .await;
        if matches!(result, Err(Error::Challenge)) {
            session = self.checked_session(url.clone(), false).await?;
            result = self.document(url.clone(), &mut session, None, choice).await;
        }
        let (arrived, body) = match result {
            Err(Error::Challenge) => {
                self.clearance.lock().expect("1337x clearance lock").take();
                return Err(Error::SourceCheckRequired);
            }
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        if arrived != url {
            return Err(Error::VariantUnavailable);
        }
        release(&body, url, publication)
    }

    fn publication_url(&self, publication: &PublicationRef) -> Result<Url> {
        publication.x1337_path()?;
        Ok(self
            .origin
            .join(&format!("torrent/{}/", publication.id))
            .expect("validated 1337x publication path"))
    }

    fn source_session(&self) -> Result<OperationSession> {
        let mut clearance = self.clearance.lock().expect("1337x clearance lock");
        if clearance.as_ref().is_some_and(|cached| {
            cached.received.elapsed() >= SESSION_LIFETIME || cached.session.has_expired_cookie()
        }) {
            clearance.take();
        }
        let http = match clearance.as_ref() {
            Some(cached) => cached.session.clone(),
            None => SourceSession::anonymous(self.origin.clone())?,
        };
        Ok(OperationSession {
            http,
            use_browser: false,
        })
    }

    async fn checked_session(&self, url: Url, interactive: bool) -> Result<OperationSession> {
        self.clearance.lock().expect("1337x clearance lock").take();
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
        *self.clearance.lock().expect("1337x clearance lock") = Some(Clearance {
            received: Instant::now(),
            session: session.clone(),
        });
        Ok(OperationSession {
            http: session,
            use_browser: false,
        })
    }

    async fn document(
        &self,
        url: Url,
        session: &mut OperationSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<(Url, String)> {
        if !session.use_browser {
            let response = self
                .http
                .session_document(url.clone(), &mut session.http)
                .await?;
            match document(response) {
                Err(Error::Challenge) => session.use_browser = true,
                result => return result,
            }
        }
        let browser = browser.ok_or(Error::Challenge)?;
        match browser_document(browser, choice, url, FetchInit::default()).await {
            Ok(response) => document(response),
            Err(
                Error::BrowserChoiceRequired
                | Error::BrowserSession(
                    BrowserError::BrowserRequired
                    | BrowserError::Disconnected
                    | BrowserError::Stopped
                    | BrowserError::VerificationRequired
                    | BrowserError::BrowserCheckRequired
                    | BrowserError::LoginRequired,
                ),
            ) => Err(Error::Challenge),
            Err(error) => Err(error),
        }
    }
}

fn search_query(query: &str) -> Result<String> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
        return Err(Error::InvalidQuery);
    }
    let query = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() {
        return Err(Error::InvalidQuery);
    }
    Ok(query)
}

fn document(response: SourceDocument) -> Result<(Url, String)> {
    let body = String::from_utf8(response.body)
        .map_err(|_| Error::InvalidResponse("1337x document is not UTF-8"))?;
    if body.contains("cf-challenge")
        || (response.status == 403 && body.contains("/cdn-cgi/challenge-platform/"))
    {
        return Err(Error::Challenge);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if !response.content_type.as_deref().is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
    }) {
        return Err(Error::InvalidResponse("1337x document is not HTML"));
    }
    if body.contains("Bad search request") {
        return Err(Error::InvalidResponse("1337x rejected the search session"));
    }
    Ok((response.final_url, body))
}

fn search_is_empty(url: &Url, body: &str, origin: &Url, query: &str) -> Result<bool> {
    let segments = url
        .path_segments()
        .ok_or(Error::InvalidResponse("invalid 1337x search URL"))?
        .collect::<Vec<_>>();
    if url.origin() != origin.origin()
        || url.query().is_some()
        || url.fragment().is_some()
        || segments.len() != 4
        || segments[0] != "search"
        || segments[2] != "1"
        || !segments[3].is_empty()
        || percent_encoding::percent_decode_str(&segments[1].replace('+', " "))
            .decode_utf8()
            .map_err(|_| Error::InvalidResponse("invalid 1337x search query"))?
            != query
    {
        return Err(Error::InvalidResponse("1337x search identity changed"));
    }
    let document = Html::parse_document(body);
    if empty_search(document.root_element())? {
        return Ok(true);
    }
    confirm_query(document.root_element(), query)?;
    Ok(false)
}

fn empty_search(root: ElementRef<'_>) -> Result<bool> {
    if text(unique(root, ".box-info-heading > h1")?) != "Message:" {
        return Ok(false);
    }
    if !matches!(
        text(unique(root, ".box-info-detail > p")?).as_str(),
        "No results were returned." | "No results were returned. Please refine your search."
    ) {
        return Err(Error::InvalidResponse("unrecognized 1337x search message"));
    }
    Ok(true)
}

fn confirm_query(root: ElementRef<'_>, query: &str) -> Result<()> {
    if text(unique(root, ".box-info-heading h1 > span")?) != query {
        return Err(Error::InvalidResponse(
            "1337x search query was not confirmed",
        ));
    }
    Ok(())
}

fn search_results(body: &str, origin: &Url, query: &str, category: Category) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    if empty_search(root)? {
        return Ok(Vec::new());
    }
    confirm_query(root, query)?;
    let tables = Selector::parse("table.table-list").expect("fixed search table selector");
    let mut tables = root.select(&tables);
    let table = tables
        .next()
        .ok_or(Error::InvalidResponse("missing 1337x search results"))?;
    if tables.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous 1337x search results"));
    }
    let rows = Selector::parse("tbody > tr").expect("fixed search row selector");
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for (index, row) in table.select(&rows).enumerate() {
        if index >= 100 {
            return Err(Error::InvalidResponse("excessive 1337x search results"));
        }
        let category_link = unique(row, "td.name > a.icon[href^='/sub/']")?;
        let reported = category_link.attr("href").unwrap();
        if subcategory(reported) != Some(category) {
            continue;
        }
        let uploader = title(unique(row, "td:last-child > a[href^='/user/']")?)?;
        if excluded_uploader(&uploader) {
            continue;
        }
        let link = unique(row, "td.name > a[href^='/torrent/']")?;
        let (reference, url) = publication(link.attr("href").unwrap(), origin)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate 1337x publication"));
        }
        works.push(work(reference, url, title(link)?, category, uploader, true));
    }
    Ok(works)
}

fn release(body: &str, url: Url, expected: &PublicationRef) -> Result<Release> {
    let document = Html::parse_document(body);
    let root = unique(document.root_element(), ".torrent-detail-page")?;
    let title = title(unique(root, ".box-info-heading > h1")?)?;
    let rows = Selector::parse("ul.list > li").expect("fixed release metadata selector");
    let mut metadata = BTreeMap::new();
    for row in root.select(&rows) {
        let key = text(unique(row, "strong")?);
        let value = text(unique(row, "span")?);
        if metadata.insert(key, value).is_some() {
            return Err(Error::InvalidResponse("duplicate 1337x release metadata"));
        }
    }
    let field = |key| {
        metadata
            .get(key)
            .ok_or(Error::InvalidResponse("missing 1337x release metadata"))
    };
    let uploader = field("Uploaded By")?;
    if excluded_uploader(uploader) {
        return Err(Error::VariantUnavailable);
    }
    let category = match field("Category")?.as_str() {
        "Movies" => Category::Movies,
        "TV" | "Documentaries" => Category::Tv,
        "Anime" => Category::Anime,
        "Games" => Category::Games,
        "Apps" => Category::Software,
        "Other" if matches!(field("Type")?.as_str(), "E-Books" | "Comics" | "Audiobook") => {
            Category::Books
        }
        _ => return Err(Error::VariantUnavailable),
    };
    let size = size_bytes(field("Total size")?)?;
    let seeders = field("Seeders")?
        .parse::<u64>()
        .map_err(|_| Error::InvalidResponse("invalid 1337x seed count"))?;
    let hash = text(unique(root, ".infohash-box p > span")?).to_ascii_lowercase();
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidResponse("invalid 1337x infohash"));
    }
    let magnets = Selector::parse("a[href^='magnet:']").expect("fixed magnet selector");
    let mut acquired = None;
    for (index, link) in root.select(&magnets).enumerate() {
        if index >= 4 {
            return Err(Error::InvalidResponse("excessive 1337x magnets"));
        }
        let (url, identity) = magnet(link.attr("href").unwrap())?;
        if identity != hash || acquired.as_ref().is_some_and(|previous| previous != &url) {
            return Err(Error::InvalidResponse(
                "1337x magnet does not match its infohash",
            ));
        }
        acquired = Some(url);
    }
    let magnet = acquired.ok_or(Error::VariantUnavailable)?;
    let language = match field("Language")?.as_str() {
        "English" => Some("en"),
        "Russian" => Some("ru"),
        "Japanese" => Some("ja"),
        "Italian" => Some("it"),
        "French" => Some("fr"),
        "German" => Some("de"),
        "Spanish" => Some("es"),
        "Chinese" => Some("zh"),
        "Korean" => Some("ko"),
        "Portuguese" => Some("pt"),
        "Hindi" => Some("hi"),
        "Dutch" => Some("nl"),
        _ => None,
    };
    let mut work = work(
        expected.clone(),
        url,
        title,
        category,
        uploader.clone(),
        false,
    );
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
        languages: language.into_iter().map(str::to_owned).collect(),
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
    {
        return Err(Error::InvalidReference);
    }
    let id = url
        .path()
        .strip_prefix("/torrent/")
        .and_then(|path| path.strip_suffix('/'))
        .ok_or(Error::InvalidReference)?;
    let reference = PublicationRef {
        source: Source::X1337,
        id: id.to_owned(),
    };
    reference.x1337_path()?;
    Ok((reference, url))
}

fn subcategory(path: &str) -> Option<Category> {
    let id = path
        .strip_prefix("/sub/")?
        .strip_suffix("/0/")?
        .parse::<u16>()
        .ok()?;
    match id {
        1..=4 | 42 | 54 | 55 | 66 | 70 | 73 | 76 => Some(Category::Movies),
        5..=9 | 41 | 71 | 74 | 75 => Some(Category::Tv),
        10..=17 | 43..=46 | 72 | 77 | 82 => Some(Category::Games),
        18..=21 | 56 | 57 => Some(Category::Software),
        28 | 78..=81 => Some(Category::Anime),
        36 | 39 | 52 => Some(Category::Books),
        _ => None,
    }
}

// r/Piracy's 1337x note and FMHY Unsafe Sites identify this current uploader.
// This exclusion does not establish the safety of any remaining payload.
fn excluded_uploader(uploader: &str) -> bool {
    uploader.eq_ignore_ascii_case("IGGGAMESCOM")
}

fn size_bytes(value: &str) -> Result<u64> {
    let (number, unit) = value
        .split_once(' ')
        .ok_or(Error::InvalidResponse("invalid 1337x size"))?;
    let number = number
        .parse::<f64>()
        .map_err(|_| Error::InvalidResponse("invalid 1337x size"))?;
    let multiplier = match unit {
        "B" => 1_u64,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024_u64.pow(3),
        "TB" => 1024_u64.pow(4),
        _ => return Err(Error::InvalidResponse("invalid 1337x size unit")),
    };
    let bytes = number * multiplier as f64;
    if !bytes.is_finite() || !(1.0..=u64::MAX as f64).contains(&bytes) {
        return Err(Error::InvalidResponse("invalid 1337x size"));
    }
    Ok(bytes as u64)
}

fn work(
    reference: PublicationRef,
    url: Url,
    title: String,
    category: Category,
    uploader: String,
    needs_inspection: bool,
) -> Work {
    Work {
        reference,
        title,
        category,
        url,
        cover: None,
        description: Some(format!("Uploader: {uploader}")),
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants: Vec::new(),
        notes: Vec::new(),
        needs_inspection,
    }
}

fn title(element: ElementRef<'_>) -> Result<String> {
    let value = text(element);
    if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid 1337x title or uploader"));
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
    let selector = Selector::parse(selector).expect("fixed 1337x metadata selector");
    let mut elements = root.select(&selector);
    let element = elements
        .next()
        .ok_or(Error::InvalidResponse("missing 1337x metadata"))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous 1337x metadata"));
    }
    Ok(element)
}
