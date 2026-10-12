use reqwest::header::{HeaderMap, HeaderValue, ORIGIN, REFERER};
use serde::de::DeserializeOwned;
use url::Url;

#[cfg(windows)]
use crate::browser::{BrowserChoice, BrowserError, BrowserHandle};
use crate::http::{SourceHttp, SourceSession};
use crate::{Error, Result, Source};

const API_ORIGIN: &str = "https://api.cdnlibs.org";

/// One operation's Lib credentials. Never serialize, persist or log this value.
/// Create it before search as well as reading: anonymous search can succeed
/// while omitting publications available to the signed-in account.
pub struct LibSession {
    source: Source,
    api: SourceSession,
    #[cfg(windows)]
    browser: Option<(BrowserHandle, u64)>,
}

impl LibSession {
    pub fn anonymous(source: Source) -> Result<Self> {
        site_origin(source)?;
        Ok(Self {
            source,
            api: SourceSession::anonymous(Url::parse(API_ORIGIN).expect("fixed Lib API"))?,
            #[cfg(windows)]
            browser: None,
        })
    }

    /// Only an already connected qcatch session is used. This does not start a
    /// browser or wait for installation; public requests still work without it.
    #[cfg(windows)]
    pub async fn from_browser(
        source: Source,
        browser: Option<&BrowserHandle>,
        choice: BrowserChoice,
    ) -> Result<Self> {
        let mut session = Self::anonymous(source)?;
        let Some(browser) = browser else {
            return Ok(session);
        };
        let site = Url::parse(site_origin(source)?).expect("fixed Lib site");
        let (borrowed, token) = match browser.lib_session_connected(choice, site, None).await {
            Ok(session) => session,
            Err(BrowserError::BrowserRequired | BrowserError::Stopped) => {
                return Ok(session);
            }
            Err(error) => return Err(browser_error(error)),
        };
        session.browser = Some((browser.clone(), borrowed.identity));
        session.api =
            SourceSession::from_browser(Url::parse(API_ORIGIN).expect("fixed Lib API"), borrowed)?;
        session.api.set_bearer(token)?;
        Ok(session)
    }
}

pub(crate) struct LibHttp {
    source: Source,
    http: SourceHttp,
    origin: Url,
}

impl LibHttp {
    pub(crate) fn new(source: Source) -> Result<Self> {
        let site = site_origin(source)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "site-id",
            HeaderValue::from_static(match source {
                Source::MangaLib => "1",
                Source::RanobeLib => "3",
                _ => return Err(Error::InvalidReference),
            }),
        );
        headers.insert(ORIGIN, HeaderValue::from_static(site));
        headers.insert(REFERER, HeaderValue::from_static(site));
        let origin = Url::parse(API_ORIGIN).expect("fixed Lib API");
        Ok(Self {
            source,
            http: SourceHttp::new(origin.clone(), headers)?,
            origin,
        })
    }

    pub(crate) fn endpoint(&self, path: &str) -> Url {
        self.origin.join(path).expect("validated Lib API path")
    }

    pub(crate) async fn json<T: DeserializeOwned>(
        &self,
        url: Url,
        session: &mut LibSession,
    ) -> Result<T> {
        if session.source != self.source {
            return Err(Error::InvalidReference);
        }
        let first = self.http.session_json(url.clone(), &mut session.api).await;
        if !matches!(first, Err(Error::HttpStatus(401))) {
            return access_result(first);
        }

        // The site owns token refresh. Read its current value once after a 401;
        // a missing/expired session is a login failure, never an anonymous retry.
        #[cfg(windows)]
        if let Some((browser, identity)) = &session.browser {
            let site = Url::parse(site_origin(self.source)?).expect("fixed Lib site");
            let (borrowed, token) = browser
                .lib_session_connected(BrowserChoice::Auto, site, Some(*identity))
                .await
                .map_err(browser_error)?;
            if token.is_none() {
                return Err(Error::LoginRequired);
            }
            session.api = SourceSession::from_browser(self.origin.clone(), borrowed)?;
            session.api.set_bearer(token)?;
            return access_result(self.http.session_json(url, &mut session.api).await);
        }
        Err(Error::LoginRequired)
    }

    pub(crate) async fn image(
        &self,
        url: Url,
        allowed_hosts: &[&str],
        session: &LibSession,
    ) -> Result<Vec<u8>> {
        if session.source != self.source {
            return Err(Error::InvalidReference);
        }
        // API authorization is never supplied to image hosts, including after
        // redirects. This fresh jar contains only this CDN's browser cookies.
        let mut image_session = SourceSession::anonymous(url.clone())?;
        #[cfg(windows)]
        if let Some((browser, identity)) = &session.browser {
            let borrowed = browser
                .session_from_connection(BrowserChoice::Auto, url.clone(), Some(*identity))
                .await
                .map_err(browser_error)?;
            image_session = SourceSession::from_browser(url.clone(), borrowed)?;
        }
        self.http
            .session_image(url, allowed_hosts, &mut image_session)
            .await
    }
}

fn site_origin(source: Source) -> Result<&'static str> {
    match source {
        Source::MangaLib => Ok("https://mangalib.me"),
        Source::RanobeLib => Ok("https://ranobelib.me"),
        _ => Err(Error::InvalidReference),
    }
}

fn access_result<T>(result: Result<T>) -> Result<T> {
    match result {
        Err(Error::HttpStatus(401)) => Err(Error::LoginRequired),
        Err(Error::HttpStatus(403)) => Err(Error::Challenge),
        result => result,
    }
}

#[cfg(windows)]
fn browser_error(error: BrowserError) -> Error {
    match error {
        BrowserError::LoginRequired => Error::LoginRequired,
        BrowserError::ChoiceRequired => Error::BrowserChoiceRequired,
        BrowserError::BrowserCheckRequired | BrowserError::VerificationRequired => Error::Challenge,
        error => Error::BrowserSession(error),
    }
}
