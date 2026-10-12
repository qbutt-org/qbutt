use url::Url;

use crate::browser::{BrowserChoice, BrowserError, BrowserHandle};
use crate::http::{SourceDocument, SourceHttp, SourceSession};
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "rutracker/document.rs"]
pub(crate) mod document;
#[path = "rutracker/response.rs"]
pub(crate) mod response;
#[path = "rutracker/taxonomy.rs"]
pub(crate) mod taxonomy;

const ORIGIN: &str = "https://rutracker.org";

/// One canonical RuTracker catalog. Browser sessions are borrowed only when a
/// public request needs login or verification; this service never opens a tab.
pub struct RuTracker {
    http: SourceHttp,
    origin: Url,
}

impl RuTracker {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed RuTracker origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?,
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
        let encoded = document::query(query)?;
        let mut session = SourceSession::anonymous(self.origin.clone())?;
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
        let mut session = SourceSession::anonymous(self.origin.clone())?;
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
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<taxonomy::Categories> {
        let url = self.origin.join("/forum/index.php?map").unwrap();
        let body = self.document(url, session, browser, choice).await?;
        taxonomy::categories(&body)
    }

    async fn document(
        &self,
        url: Url,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<String> {
        let first = self.http.session_document(url.clone(), session).await?;
        let blocked = match document(first, &url) {
            Err(error @ (Error::LoginRequired | Error::Challenge)) => error,
            result => return result,
        };
        let Some(browser) = browser else {
            return Err(blocked);
        };
        let scope = self.origin.join("/forum/tracker.php").unwrap();
        let borrowed = match browser.session_connected(choice, scope).await {
            Ok(session) => session,
            Err(
                BrowserError::BrowserRequired | BrowserError::Disconnected | BrowserError::Stopped,
            ) => {
                return Err(blocked);
            }
            Err(BrowserError::ChoiceRequired) => return Err(Error::BrowserChoiceRequired),
            Err(error) => return Err(Error::BrowserSession(error)),
        };
        *session = SourceSession::from_browser(self.origin.clone(), borrowed)?;
        document(
            self.http.session_document(url.clone(), session).await?,
            &url,
        )
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
