use std::fmt::Write;

use encoding_rs::{UTF_8, WINDOWS_1251};
use scraper::{Html, Selector};
use url::Url;

use crate::{Error, Result};

pub(super) fn query(value: &str) -> Result<String> {
    if value.is_empty() || value.chars().count() > 60 || value.chars().any(char::is_control) {
        return Err(Error::InvalidQuery);
    }
    let (bytes, _, invalid) = WINDOWS_1251.encode(value);
    if invalid {
        return Err(Error::InvalidQuery);
    }
    let mut encoded = String::new();
    for byte in bytes.iter().copied() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else if byte == b' ' {
            encoded.push('+');
        } else {
            write!(encoded, "%{byte:02X}").expect("writing to String");
        }
    }
    Ok(encoded)
}

pub(super) fn decode(bytes: &[u8], content_type: Option<&str>) -> Result<String> {
    let content_type = content_type.ok_or(Error::InvalidResponse("missing HTML content type"))?;
    let mut parts = content_type.split(';');
    if !parts
        .next()
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
    {
        return Err(Error::InvalidResponse("NNM-Club response is not HTML"));
    }
    let mut encoding = WINDOWS_1251;
    let mut charset_seen = false;
    for part in parts {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        if name.eq_ignore_ascii_case("charset") {
            if charset_seen {
                return Err(Error::InvalidResponse("ambiguous HTML charset"));
            }
            charset_seen = true;
            encoding = match value
                .trim()
                .trim_matches(['\'', '"'])
                .to_ascii_lowercase()
                .as_str()
            {
                "windows-1251" | "cp1251" => WINDOWS_1251,
                "utf-8" | "utf8" => UTF_8,
                _ => return Err(Error::InvalidResponse("unsupported NNM-Club charset")),
            };
        }
    }
    let (body, invalid) = encoding.decode_without_bom_handling(bytes);
    if invalid {
        return Err(Error::InvalidResponse("invalid NNM-Club text encoding"));
    }
    Ok(body.into_owned())
}

pub(super) fn access(body: &str, final_url: &Url) -> Result<()> {
    let document = Html::parse_document(body);
    let challenge = Selector::parse(
        "input[name='cap_sid'], input[name='g-recaptcha-response'], textarea[name='g-recaptcha-response'], input[name='h-captcha-response'], .g-recaptcha, .h-captcha, .cf-turnstile",
    )
    .unwrap();
    if document.select(&challenge).next().is_some()
        || body.contains("/cdn-cgi/challenge-platform/")
        || body.contains("_cf_chl_opt")
        || body.contains("challenges.cloudflare.com")
    {
        return Err(Error::Challenge);
    }
    if final_url.path() == "/forum/login.php" {
        return Err(Error::LoginRequired);
    }
    Ok(())
}
