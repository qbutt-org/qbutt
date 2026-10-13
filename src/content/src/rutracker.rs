use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use url::Url;

use crate::browser::{BrowserChoice, BrowserError, BrowserHandle};
use crate::http::{SourceDocument, SourceHttp, SourceSession};
use crate::webview::{WebViewCheck, WebViewError, WebViewSessions};
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "rutracker/document.rs"]
pub(crate) mod document;
#[path = "rutracker/response.rs"]
pub(crate) mod response;
#[path = "rutracker/taxonomy.rs"]
pub(crate) mod taxonomy;

const ORIGIN: &str = "https://rutracker.org";
const SESSION_LIFETIME: Duration = Duration::from_secs(30 * 60);

struct Clearance {
    received: Instant,
    session: SourceSession,
    manual: bool,
}

struct OperationSession {
    http: SourceSession,
    browser: BrowserBorrow,
}

#[derive(Clone, Copy)]
enum BrowserBorrow {
    None,
    Borrowed,
    Failed(BrowserError),
}

/// One RuTracker catalog. An approved connected browser can supply the initial
/// session; source checks use this provider's separate WebView profile.
pub struct RuTracker {
    http: SourceHttp,
    origin: Url,
    webview: OnceLock<WebViewSessions>,
    clearance: Mutex<Option<Clearance>>,
}

impl RuTracker {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed RuTracker origin");
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
            .expect("RuTracker clearance lock")
            .take();
        if let Some(webview) = self.webview.get() {
            webview.close();
        }
    }

    /// An explicit source action opens the same profile used for silent checks.
    pub async fn complete_check(&self) -> Result<()> {
        let webview = self
            .webview
            .get()
            .ok_or(Error::SourceCheck(WebViewError::Unavailable))?;
        let checked = webview
            .check(
                self.origin.join("/forum/index.php").unwrap(),
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
        *self.clearance.lock().expect("RuTracker clearance lock") = Some(Clearance {
            received: Instant::now(),
            session,
            manual: true,
        });
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
        let encoded = document::query(query)?;
        let mut session = self.source_session(browser, choice).await?;
        let map = self.categories(&mut session, browser, choice).await?;
        let mut url = self.origin.join("/forum/tracker.php").unwrap();
        url.set_query(Some(&format!("nm={encoded}&o=10&s=2&df=1")));
        let body = self.document(url, &mut session, browser, choice).await?;
        response::search(&body, &self.origin, category, &map)
    }

    pub async fn inspect(
        &self,
        publication: &PublicationRef,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Work> {
        Ok(self.release(publication, browser, choice).await?.work)
    }

    /// Re-fetch the exact topic and its current source-advertised magnet. A
    /// replaced torrent hash cannot replace a previously selected variant.
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
        publication.rutracker_id()?;
        let mut session = self.source_session(browser, choice).await?;
        let map = self.categories(&mut session, browser, choice).await?;
        let url = response::topic_url(&self.origin, publication);
        let body = match self.document(url, &mut session, browser, choice).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::release(&body, &self.origin, publication, &map)
    }

    async fn categories(
        &self,
        session: &mut OperationSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<taxonomy::Categories> {
        let url = self.origin.join("/forum/index.php?map").unwrap();
        let body = self.document(url, session, browser, choice).await?;
        taxonomy::categories(&body, &self.origin)
    }

    async fn source_session(
        &self,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<OperationSession> {
        let cached = {
            let clearance = self.clearance.lock().expect("RuTracker clearance lock");
            clearance
                .as_ref()
                .filter(|clearance| {
                    clearance.received.elapsed() < SESSION_LIFETIME
                        && !clearance.session.has_expired_cookie()
                })
                .map(|clearance| (clearance.session.clone(), clearance.manual))
        };
        if let Some((session, true)) = &cached {
            return Ok(OperationSession {
                http: session.clone(),
                browser: BrowserBorrow::None,
            });
        }
        let mut browser_state = BrowserBorrow::None;
        if let Some(browser) = browser {
            let scope = self.origin.join("/forum/tracker.php").unwrap();
            match browser.session_connected(choice, scope).await {
                Ok(borrowed) => {
                    return Ok(OperationSession {
                        http: SourceSession::from_browser(self.origin.clone(), borrowed)?,
                        browser: BrowserBorrow::Borrowed,
                    });
                }
                Err(BrowserError::BrowserRequired | BrowserError::Disconnected) => {}
                Err(error) => browser_state = BrowserBorrow::Failed(error),
            }
        }
        if let Some((cached, _)) = cached {
            return Ok(OperationSession {
                http: cached,
                browser: browser_state,
            });
        }
        Ok(OperationSession {
            http: SourceSession::anonymous(self.origin.clone())?,
            browser: browser_state,
        })
    }

    async fn document(
        &self,
        url: Url,
        session: &mut OperationSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<String> {
        let first = self
            .http
            .session_document(url.clone(), &mut session.http)
            .await?;
        let blocked = match document(first, &url) {
            Err(error @ (Error::LoginRequired | Error::Challenge)) => error,
            result => return result,
        };
        if matches!(&blocked, Error::LoginRequired)
            && !matches!(session.browser, BrowserBorrow::Borrowed)
            && let Some(browser) = browser
        {
            let scope = self.origin.join("/forum/tracker.php").unwrap();
            let borrowed = if let BrowserBorrow::Failed(error) = session.browser {
                Err(error)
            } else {
                browser.session_connected(choice, scope).await
            };
            match borrowed {
                Ok(borrowed) => {
                    session.http = SourceSession::from_browser(self.origin.clone(), borrowed)?;
                    session.browser = BrowserBorrow::Borrowed;
                    let response = self
                        .http
                        .session_document(url.clone(), &mut session.http)
                        .await?;
                    return document(response, &url);
                }
                Err(error) => match error {
                    BrowserError::BrowserRequired
                    | BrowserError::Disconnected
                    | BrowserError::Stopped => {}
                    BrowserError::ChoiceRequired => return Err(Error::BrowserChoiceRequired),
                    error => return Err(Error::BrowserSession(error)),
                },
            }
        }

        let Some(webview) = self.webview.get() else {
            return Err(if matches!(&blocked, Error::LoginRequired) {
                blocked
            } else {
                Error::SourceCheck(WebViewError::Unavailable)
            });
        };
        let verified = match webview.check(url.clone(), WebViewCheck::default()).await {
            Ok(session) => session,
            Err(WebViewError::CheckRequired) => {
                return Err(if matches!(&blocked, Error::LoginRequired) {
                    Error::LoginRequired
                } else {
                    Error::SourceCheckRequired
                });
            }
            Err(error) => return Err(Error::SourceCheck(error)),
        };
        session.http = SourceSession::from_webview(self.origin.clone(), verified)?;
        session.browser = BrowserBorrow::None;
        let response = self
            .http
            .session_document(url.clone(), &mut session.http)
            .await?;
        match document(response, &url) {
            Ok(body) => {
                *self.clearance.lock().expect("RuTracker clearance lock") = Some(Clearance {
                    received: Instant::now(),
                    session: session.http.clone(),
                    manual: false,
                });
                Ok(body)
            }
            Err(Error::Challenge) => Err(Error::SourceCheckRequired),
            result => result,
        }
    }
}

fn document(response: SourceDocument, expected: &Url) -> Result<String> {
    let body = document::decode(&response.body, response.content_type.as_deref())?;
    document::access(&body, &response.final_url)?;
    if !(200..300).contains(&response.status) {
        return Err(Error::HttpStatus(response.status));
    }
    if response.final_url != *expected {
        return Err(Error::InvalidResponse(
            "RuTracker response identity changed",
        ));
    }
    Ok(body)
}
