use std::collections::BTreeMap;

use url::{Host, Url};

use super::BrowserError;

pub(super) const MAX_BODY_BYTES: usize = 512 * 1024;
pub(super) const MAX_FETCH_BODY_BYTES: usize = 680 * 1024;

pub fn public_url(url: &Url) -> Result<(), BrowserError> {
    // A selected page may include an anchor. Preserve its exact identity while
    // validating the address which the browser actually requests.
    let mut address = url.clone();
    address.set_fragment(None);
    crate::http::validate_media_url(&address).map_err(|_| BrowserError::InvalidResponse)?;
    if url.as_str().len() > 8192
        || match url.host() {
            Some(Host::Domain(host)) => {
                let host = host.trim_end_matches('.');
                !host.contains('.')
                    || host.ends_with(".local")
                    || host.ends_with(".localhost")
                    || host == "localhost"
            }
            Some(Host::Ipv6(_)) => true,
            _ => false,
        }
    {
        return Err(BrowserError::InvalidResponse);
    }
    Ok(())
}

pub(super) fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), BrowserError> {
    if headers.len() > 64
        || headers.iter().any(|(name, value)| {
            !token(name, 128) || value.len() > 16 * 1024 || value.chars().any(char::is_control)
        })
        || headers
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum::<usize>()
            > 64 * 1024
    {
        return Err(BrowserError::InvalidResponse);
    }
    Ok(())
}

pub(super) fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
}

pub(super) fn token(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

pub(super) fn uuid_text(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}
