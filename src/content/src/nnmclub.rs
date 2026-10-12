use url::Url;

use crate::browser::{BrowserChoice, BrowserError, BrowserHandle};
use crate::http::{SourceDocument, SourceHttp, SourceSession};
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "nnmclub/document.rs"]
mod document;
#[path = "nnmclub/response.rs"]
mod response;

const ORIGIN: &str = "https://nnmclub.to";

/// Public tracker search and magnet acquisition. The browser supplies a session
/// only when the site explicitly requires a verification or login step.
pub struct NnmClub {
    http: SourceHttp,
    origin: Url,
}

impl NnmClub {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed NNM-Club origin");
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
        if !matches!(category, Category::Movies | Category::Software) {
            return Ok(Vec::new());
        }
        let query = document::query(query.trim())?;
        let mut session = SourceSession::anonymous(self.origin.clone())?;
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
        let mut session = SourceSession::anonymous(self.origin.clone())?;
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
        let first = if let Some(form) = form {
            self.http
                .session_form_document(url.clone(), form, session)
                .await?
        } else {
            self.http.session_document(url.clone(), session).await?
        };
        match checked_document(first, &url) {
            Err(error @ (Error::Challenge | Error::LoginRequired)) => {
                self.retry_with_browser(url, form, session, browser, choice, error)
                    .await
            }
            result => result,
        }
    }

    async fn retry_with_browser(
        &self,
        url: Url,
        form: Option<&str>,
        session: &mut SourceSession,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
        blocked: Error,
    ) -> Result<String> {
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
        let response = if let Some(form) = form {
            self.http
                .session_form_document(url.clone(), form, session)
                .await?
        } else {
            self.http.session_document(url.clone(), session).await?
        };
        checked_document(response, &url)
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
