use cookie_store::{Cookie, CookieExpiration, RawCookie};
use reqwest::RequestBuilder;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue, SET_COOKIE, USER_AGENT};
use url::{Origin, Url};

use crate::{Error, Result};

const MAX_COOKIES: usize = 512;
const MAX_COOKIE_BYTES: usize = 16 * 1024;
const MAX_SESSION_BYTES: usize = 256 * 1024;
const MAX_COOKIE_HEADER_BYTES: usize = 64 * 1024;

/// Credentials live only for the provider operation and never enter public
/// models, persistence or diagnostics. Exact-origin HTTP guards remain external
/// to RFC cookie domain/path matching.
pub(crate) struct SourceSession {
    pub(super) origin: Origin,
    user_agent: Option<HeaderValue>,
    cookies: Vec<Cookie<'static>>,
}

impl SourceSession {
    pub(crate) fn anonymous(origin: Url) -> Result<Self> {
        super::validate_https_url(&origin)?;
        Ok(Self {
            origin: origin.origin(),
            user_agent: None,
            cookies: Vec::new(),
        })
    }

    #[cfg(windows)]
    pub(crate) fn from_browser(
        origin: Url,
        session: crate::browser::BrowserSession,
    ) -> Result<Self> {
        let mut source = Self::anonymous(origin)?;
        if session.url.origin() != source.origin {
            return Err(Error::UnsafeAddress);
        }
        source.user_agent = Some(
            HeaderValue::from_str(&session.connection.user_agent)
                .map_err(|_| Error::InvalidResponse("invalid browser user agent"))?,
        );
        for browser in session.cookies {
            let mut raw = RawCookie::build((browser.name, browser.value))
                .path(browser.path)
                .secure(browser.secure)
                .http_only(browser.http_only.unwrap_or(false));
            if !browser.host_only {
                raw = raw.domain(browser.domain);
            }
            let mut cookie = Cookie::try_from_raw_cookie(&raw.build(), &session.url)
                .map_err(|_| Error::InvalidResponse("invalid browser cookie"))?
                .into_owned();
            if let Some(expiry) = browser.expiration_date {
                let expiry = time::OffsetDateTime::from_unix_timestamp_nanos(
                    (expiry * 1_000_000_000.0) as i128,
                )
                .map_err(|_| Error::InvalidResponse("invalid browser cookie expiry"))?;
                cookie.expires = CookieExpiration::AtUtc(expiry);
            }
            source.store(cookie)?;
        }
        Ok(source)
    }

    pub(super) fn apply(&self, request: RequestBuilder, url: &Url) -> Result<RequestBuilder> {
        let mut cookies: Vec<_> = self
            .cookies
            .iter()
            .filter(|cookie| !cookie.is_expired() && cookie.matches(url))
            .collect();
        cookies.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.as_ref().len()));
        let mut header = String::new();
        for cookie in cookies {
            if !header.is_empty() {
                header.push_str("; ");
            }
            header.push_str(cookie.name());
            header.push('=');
            header.push_str(cookie.value());
            if header.len() > MAX_COOKIE_HEADER_BYTES {
                return Err(Error::ResponseTooLarge);
            }
        }
        let header = HeaderValue::from_str(&header)
            .map_err(|_| Error::InvalidResponse("invalid source cookie"))?;
        let request = request.header(COOKIE, header);
        Ok(match &self.user_agent {
            Some(user_agent) => request.header(USER_AGENT, user_agent),
            None => request,
        })
    }

    pub(super) fn receive(&mut self, headers: &HeaderMap, url: &Url) -> Result<()> {
        self.cookies.retain(|cookie| !cookie.is_expired());
        for header in headers.get_all(SET_COOKIE) {
            if header.len() > MAX_COOKIE_BYTES {
                return Err(Error::ResponseTooLarge);
            }
            let Some(cookie) = header
                .to_str()
                .ok()
                .and_then(|value| Cookie::parse(value.to_owned(), url).ok())
            else {
                continue;
            };
            self.store(cookie)?;
        }
        Ok(())
    }

    fn store(&mut self, cookie: Cookie<'static>) -> Result<()> {
        // The browser and HTTP values must remain one cookie pair when sent.
        if cookie.name().is_empty()
            || cookie
                .name()
                .bytes()
                .any(|byte| !byte.is_ascii_graphic() || b"()<>@,;:\\\"/[]?={}".contains(&byte))
            || cookie
                .value()
                .bytes()
                .any(|byte| !byte.is_ascii_graphic() || b"\",;\\".contains(&byte))
        {
            return Ok(());
        }
        self.cookies.retain(|old| {
            !old.is_expired()
                && !(old.domain.as_cow() == cookie.domain.as_cow()
                    && old.path == cookie.path
                    && old.name() == cookie.name())
        });
        if !cookie.is_expired() {
            self.cookies.push(cookie);
        }
        let bytes: usize = self
            .cookies
            .iter()
            .map(|cookie| {
                cookie.name().len()
                    + cookie.value().len()
                    + cookie.domain.as_cow().as_deref().map_or(0, str::len)
                    + cookie.path.as_ref().len()
            })
            .sum();
        if self.cookies.len() > MAX_COOKIES || bytes > MAX_SESSION_BYTES {
            return Err(Error::ResponseTooLarge);
        }
        Ok(())
    }
}
