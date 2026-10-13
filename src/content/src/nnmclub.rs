use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use url::Url;

use crate::browser::{BrowserChoice, BrowserError, BrowserHandle, FetchInit};
use crate::http::{SourceDocument, SourceHttp, SourceSession, browser_document};
use crate::webview::{WebViewCheck, WebViewError, WebViewSessions};
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "nnmclub/document.rs"]
mod document;
#[path = "nnmclub/response.rs"]
mod response;

const ORIGIN: &str = "https://nnmclub.to";

/// Public tracker search and magnet acquisition. Source checks use an isolated
/// WebView2 profile; a connected browser can also lend its current session.
pub struct NnmClub {
    http: SourceHttp,
    origin: Url,
    webview: OnceLock<WebViewSessions>,
    clearance: Mutex<Option<SourceSession>>,
}

impl NnmClub {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed NNM-Club origin");
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
            .expect("NNM-Club clearance lock")
            .take();
        if let Some(webview) = self.webview.get() {
            webview.close();
        }
    }

    pub async fn complete_check(&self) -> Result<()> {
        let webview = self
            .webview
            .get()
            .ok_or(Error::SourceCheck(WebViewError::Unavailable))?;
        let checked = webview
            .check(
                self.origin.join("/forum/tracker.php").unwrap(),
                WebViewCheck {
                    interactive: true,
                    ..WebViewCheck::default()
                },
            )
            .await
            .map_err(|error| match error {
                WebViewError::CheckRequired | WebViewError::Canceled | WebViewError::Timeout => {
                    Error::SourceCheckRequired
                }
                error => Error::SourceCheck(error),
            })?;
        let session = SourceSession::from_webview(self.origin.clone(), checked)?;
        *self.clearance.lock().expect("NNM-Club clearance lock") = Some(session);
        Ok(())
    }

    pub async fn search(
        &self,
        query: &str,
        category: Category,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Vec<Work>> {
        if !matches!(category, Category::Movies | Category::Software) {
            return Ok(Vec::new());
        }
        let query = document::query(query.trim())?;
        let mut session = self.source_session()?;
        let tracker = self.origin.join("/forum/tracker.php").unwrap();
        let taxonomy = self
            .document(tracker.clone(), None, &mut session, browser, choice)
            .await?;
        let forums = response::Forums::parse(&taxonomy)?;
        let form = response::search_form(&query, forums.for_category(category));
        let results = self
            .document(tracker, Some(&form), &mut session, browser, choice)
            .await?;
        response::search(&results, &self.origin, category, &forums)
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
        self.release(&selected.publication, browser, choice)
            .await?
            .resolve(selected)
    }

    async fn release(
        &self,
        publication: &PublicationRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<response::Release> {
        publication.nnmclub_id()?;
        let mut session = self.source_session()?;
        let tracker = self.origin.join("/forum/tracker.php").unwrap();
        let taxonomy = self
            .document(tracker, None, &mut session, browser, choice)
            .await?;
        let forums = response::Forums::parse(&taxonomy)?;
        let url = response::topic_url(&self.origin, publication);
        let body = match self
            .document(url, None, &mut session, browser, choice)
            .await
        {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::release(&body, &self.origin, publication, &forums)
    }

    async fn document(
        &self,
        url: Url,
        form: Option<&str>,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<String> {
        match self.fetch_document(&url, form, session).await {
            Err(error @ (Error::Challenge | Error::LoginRequired)) => {
                self.recover_document(url, form, session, browser, choice, error)
                    .await
            }
            result => result,
        }
    }

    async fn recover_document(
        &self,
        url: Url,
        form: Option<&str>,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
        blocked: Error,
    ) -> Result<String> {
        if let Some(browser) = browser {
            let scope = self.origin.join("/forum/tracker.php").unwrap();
            match browser.session_connected(choice, scope).await {
                Ok(borrowed) => {
                    *session = SourceSession::from_browser(self.origin.clone(), borrowed)?;
                    match self.fetch_document(&url, form, session).await {
                        Err(Error::Challenge | Error::LoginRequired) => {}
                        result => return result,
                    }
                }
                Err(
                    BrowserError::BrowserRequired
                    | BrowserError::Disconnected
                    | BrowserError::Stopped,
                ) => {}
                Err(BrowserError::ChoiceRequired) => return Err(Error::BrowserChoiceRequired),
                Err(error) => return Err(Error::BrowserSession(error)),
            }

            let mut init = FetchInit::default();
            if let Some(form) = form {
                init.method = Some("POST".into());
                init.headers.insert(
                    "content-type".into(),
                    "application/x-www-form-urlencoded".into(),
                );
                init.body = Some(form.into());
            }
            match browser_document(browser, choice, url.clone(), init).await {
                Ok(response) => match checked_document(response, &url) {
                    Err(Error::Challenge | Error::LoginRequired) => {}
                    result => return result,
                },
                Err(Error::BrowserSession(
                    BrowserError::BrowserRequired
                    | BrowserError::Disconnected
                    | BrowserError::Stopped
                    | BrowserError::VerificationRequired
                    | BrowserError::BrowserCheckRequired
                    | BrowserError::LoginRequired,
                )) => {}
                Err(error) => return Err(error),
            }
        }

        let Some(webview) = self.webview.get() else {
            return Err(blocked);
        };
        let checked = webview
            .check(
                self.origin.join("/forum/tracker.php").unwrap(),
                WebViewCheck::default(),
            )
            .await
            .map_err(|error| match error {
                WebViewError::CheckRequired => Error::SourceCheckRequired,
                error => Error::SourceCheck(error),
            })?;
        *session = SourceSession::from_webview(self.origin.clone(), checked)?;
        match self.fetch_document(&url, form, session).await {
            Ok(body) => {
                *self.clearance.lock().expect("NNM-Club clearance lock") = Some(session.clone());
                Ok(body)
            }
            Err(Error::Challenge | Error::LoginRequired) => Err(Error::SourceCheckRequired),
            result => result,
        }
    }

    fn source_session(&self) -> Result<SourceSession> {
        if let Some(session) = self
            .clearance
            .lock()
            .expect("NNM-Club clearance lock")
            .as_ref()
            .filter(|session| !session.has_expired_cookie())
        {
            return Ok(session.clone());
        }
        SourceSession::anonymous(self.origin.clone())
    }

    async fn fetch_document(
        &self,
        url: &Url,
        form: Option<&str>,
        session: &mut SourceSession,
    ) -> Result<String> {
        let response = if let Some(form) = form {
            self.http
                .session_form_document(url.clone(), form, session)
                .await?
        } else {
            self.http.session_document(url.clone(), session).await?
        };
        checked_document(response, url)
    }
}

fn checked_document(response: SourceDocument, expected: &Url) -> Result<String> {
    let body = document::decode(&response.body, response.content_type.as_deref())?;
    document::access(&body, &response.final_url)?;
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if response.final_url != *expected {
        return Err(Error::InvalidResponse("NNM-Club response identity changed"));
    }
    Ok(body)
}
