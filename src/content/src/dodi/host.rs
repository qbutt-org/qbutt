use scraper::{Html, Selector};
use url::Url;

use crate::http::{SourceDocument, SourceHttp, SourceSession};
use crate::{Error, Result, TorrentMetadata};

const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

pub(super) async fn download(
    http: &SourceHttp,
    url: Url,
    session: &mut SourceSession,
    id: &str,
) -> Result<TorrentMetadata> {
    let landing = http.session_document(url.clone(), session).await?;
    let form = download_form(&landing, &url, id)?;
    let origin_host = url.host_str().ok_or(Error::UnsafeAddress)?;
    let downloaded = http
        .session_hosted_metadata(
            url.clone(),
            session,
            &form,
            &[origin_host, "svr1.file-me.top"],
        )
        .await?;
    metadata(downloaded)
}

fn download_form(document: &SourceDocument, url: &Url, id: &str) -> Result<String> {
    check_status(document)?;
    if document.final_url != *url {
        return Err(Error::VariantUnavailable);
    }
    if !is_html(document) || document.body.len() > MAX_HTML_BYTES {
        return Err(Error::InvalidResponse("invalid DODI torrent host form"));
    }
    let body = std::str::from_utf8(&document.body)
        .map_err(|_| Error::InvalidResponse("torrent host form is not UTF-8"))?;
    let html = Html::parse_document(body);
    check_gate(&html, body)?;
    let form_selector = Selector::parse("form").expect("fixed host form selector");
    let input_selector =
        Selector::parse("input[type='hidden'][name]").expect("fixed host input selector");
    let mut matched = None;
    for form in html.select(&form_selector) {
        let mut fields = Vec::new();
        for input in form.select(&input_selector) {
            let name = input.attr("name").unwrap();
            let value = input.attr("value").unwrap_or_default();
            fields.push((name, value));
        }
        if !fields.contains(&("op", "download2")) {
            continue;
        }
        if fields.len() > 16
            || fields
                .iter()
                .enumerate()
                .any(|(index, (name, _))| fields[..index].iter().any(|(known, _)| known == name))
        {
            return Err(Error::InvalidResponse(
                "duplicate or excessive torrent host form fields",
            ));
        }
        if !fields.contains(&("id", id)) || matched.is_some() {
            return Err(Error::VariantUnavailable);
        }
        let action = url
            .join(form.attr("action").unwrap_or_default())
            .map_err(|_| Error::UnsafeAddress)?;
        if action != *url
            || !form
                .attr("method")
                .is_some_and(|method| method.eq_ignore_ascii_case("post"))
        {
            return Err(Error::UnsafeAddress);
        }
        let mut encoded = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in fields {
            if !matches!(
                name,
                "op" | "id"
                    | "rand"
                    | "usr_login"
                    | "fname"
                    | "referer"
                    | "method_free"
                    | "method_premium"
                    | "adblock_detected"
            ) {
                return Err(Error::InvalidResponse(
                    "unsupported torrent host form field",
                ));
            }
            encoded.append_pair(name, value);
        }
        matched = Some(encoded.finish());
    }
    matched.ok_or(Error::InvalidResponse("missing DODI torrent host form"))
}

fn metadata(document: SourceDocument) -> Result<TorrentMetadata> {
    check_status(&document)?;
    if is_html(&document) {
        if document.body.len() > MAX_HTML_BYTES {
            return Err(Error::ResponseTooLarge);
        }
        let body = std::str::from_utf8(&document.body)
            .map_err(|_| Error::InvalidResponse("torrent host response is not UTF-8"))?;
        check_gate(&Html::parse_document(body), body)?;
        return Err(Error::InvalidResponse(
            "torrent host returned HTML instead of metadata",
        ));
    }
    let mime = document
        .content_type
        .as_deref()
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if !mime.is_some_and(|value| {
        value.eq_ignore_ascii_case("application/x-bittorrent")
            || value.eq_ignore_ascii_case("application/octet-stream")
    }) {
        return Err(Error::InvalidResponse(
            "torrent host returned unsupported metadata type",
        ));
    }
    crate::torrent::info_hash(&document.body)?;
    TorrentMetadata::new(document.body)
}

fn check_status(document: &SourceDocument) -> Result<()> {
    if matches!(document.status, 404 | 410) {
        Err(Error::VariantUnavailable)
    } else if !(200..300).contains(&document.status) {
        Err(Error::HttpStatus(document.status))
    } else {
        Ok(())
    }
}

fn is_html(document: &SourceDocument) -> bool {
    document.content_type.as_deref().is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("text/html")
    })
}

fn check_gate(document: &Html, body: &str) -> Result<()> {
    let challenge = Selector::parse(".cf-turnstile, .g-recaptcha, .h-captcha")
        .expect("fixed host gate selector");
    if document.select(&challenge).next().is_some() {
        return Err(Error::InvalidResponse(
            "DODI torrent host requires browser verification",
        ));
    }
    if body.contains("You have to wait") && body.contains("till next download") {
        return Err(Error::InvalidResponse(
            "DODI torrent host download rate limit",
        ));
    }
    Ok(())
}
