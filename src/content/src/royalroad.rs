use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use url::Url;

use crate::http::{SourceDocument, SourceHttp, SourceSession};
use crate::webview::{WebViewCheck, WebViewError, WebViewSessions};
use crate::{
    Chapter, ChapterRef, Error, PublicationRef, Result, Source, SourceOperation, TextChapter,
    VariantRef, Work,
};

#[path = "royalroad/response.rs"]
pub(crate) mod response;

const ORIGIN: &str = "https://www.royalroad.com";

/// Royal Road's public English fiction. The caller owns runtime and cancellation.
pub struct RoyalRoad {
    http: SourceHttp,
    origin: Url,
    webview: OnceLock<WebViewSessions>,
    clearance: Mutex<Option<SourceSession>>,
}

impl RoyalRoad {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Royal Road origin");
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
        self.clearance
            .lock()
            .expect("Royal Road clearance lock")
            .take();
        if let Some(webview) = self.webview.get() {
            webview.close();
        }
    }

    /// Only an explicit source retry may show the profile used for silent checks.
    pub async fn complete_check(&self, operation: &SourceOperation) -> Result<()> {
        if operation.source() != Source::RoyalRoad {
            return Err(Error::InvalidReference);
        }
        operation.validate()?;
        let url = match operation {
            SourceOperation::Search { query, .. } => self.search_url(query)?,
            SourceOperation::Inspect { reference, .. } => self.publication_url(reference)?,
            SourceOperation::Chapters(reference) => self.publication_url(&reference.publication)?,
            SourceOperation::Chapter(reference) => {
                self.publication_url(&reference.variant.publication)?
            }
            _ => return Err(Error::InvalidReference),
        };
        self.checked_session(url, true).await?;
        Ok(())
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Work>> {
        response::search(&self.page(self.search_url(query)?).await?, &self.origin)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.book(publication).await?.work)
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        Ok(self
            .book(&selected.publication)
            .await?
            .chapters
            .into_iter()
            .map(|entry| entry.chapter)
            .collect())
    }

    pub async fn chapter_text(&self, selected: &ChapterRef) -> Result<TextChapter> {
        selected.validate()?;
        // Fresh public membership prevents stale references from opening a
        // subscription chapter or a chapter reassigned to another fiction.
        let book = self.book(&selected.variant.publication).await?;
        let entry = book
            .chapters
            .into_iter()
            .find(|entry| entry.chapter.reference == *selected)
            .ok_or(Error::VariantUnavailable)?;
        let body = match self.page(entry.url).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::chapter(&body, &self.origin, selected)
    }

    async fn book(&self, publication: &PublicationRef) -> Result<response::Book> {
        let body = match self.page(self.publication_url(publication)?).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::book(&body, &self.origin, publication)
    }

    fn publication_url(&self, publication: &PublicationRef) -> Result<Url> {
        let id = publication.royalroad_id()?;
        Ok(self
            .origin
            .join(&format!("/fiction/{id}"))
            .expect("canonical decimal fiction path"))
    }

    fn search_url(&self, query: &str) -> Result<Url> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let mut url = self
            .origin
            .join("/fictions/search")
            .expect("fixed search path");
        url.query_pairs_mut().append_pair("title", query);
        Ok(url)
    }

    async fn page(&self, url: Url) -> Result<String> {
        let mut session = {
            let mut clearance = self.clearance.lock().expect("Royal Road clearance lock");
            if clearance
                .as_ref()
                .is_some_and(SourceSession::has_expired_cookie)
            {
                clearance.take();
            }
            match clearance.as_ref() {
                Some(cached) => cached.clone(),
                None => SourceSession::anonymous(self.origin.clone())?,
            }
        };
        let response = self
            .http
            .session_document(url.clone(), &mut session)
            .await?;
        match html_response(response, &url) {
            Err(Error::Challenge) => {
                session = self.checked_session(url.clone(), false).await?;
                let response = self
                    .http
                    .session_document(url.clone(), &mut session)
                    .await?;
                match html_response(response, &url) {
                    Err(Error::Challenge) => {
                        self.clearance
                            .lock()
                            .expect("Royal Road clearance lock")
                            .take();
                        Err(Error::SourceCheckRequired)
                    }
                    result => result,
                }
            }
            result => result,
        }
    }

    async fn checked_session(&self, url: Url, interactive: bool) -> Result<SourceSession> {
        self.clearance
            .lock()
            .expect("Royal Road clearance lock")
            .take();
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
                WebViewError::CheckRequired | WebViewError::Timeout => Error::SourceCheckRequired,
                WebViewError::Canceled if interactive => Error::SourceCheckRequired,
                error => Error::SourceCheck(error),
            })?;
        let session = SourceSession::from_webview(self.origin.clone(), checked)?;
        *self.clearance.lock().expect("Royal Road clearance lock") = Some(session.clone());
        Ok(session)
    }
}

fn html_response(response: SourceDocument, expected: &Url) -> Result<String> {
    let body = String::from_utf8(response.body);
    if let Ok(body) = &body
        && response::is_challenge(body, response.status)
    {
        return Err(Error::Challenge);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if expected.path() == "/fictions/search" && response.final_url != *expected {
        return Err(Error::InvalidResponse("Royal Road search identity changed"));
    }
    if !response.content_type.as_deref().is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
    }) {
        return Err(Error::InvalidResponse("Royal Road document is not HTML"));
    }
    body.map_err(|_| Error::InvalidResponse("Royal Road document is not UTF-8"))
}
