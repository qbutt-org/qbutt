use std::collections::BTreeMap;

use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue, REFERER};
use scraper::{Html, Selector};
use serde::Deserialize;
use url::Url;

use crate::http::{SourceHttp, validate_https_url};
use crate::{Error, Result, VideoQuality};

use super::response::Video;

const ORIGIN: &str = "https://kodikplayer.com";

#[derive(Deserialize)]
struct Sources {
    links: BTreeMap<String, Vec<Source>>,
}

#[derive(Deserialize)]
struct Source {
    src: String,
    #[serde(rename = "type")]
    mime: String,
}

pub(super) async fn resolve(selected: &Video, player: &Url) -> Result<(Url, VideoQuality)> {
    let origin = Url::parse(ORIGIN).expect("fixed Kodik origin");
    let route: Vec<_> = player
        .path_segments()
        .ok_or(Error::UnsafeAddress)?
        .collect();
    if player.origin() != origin.origin()
        || route.len() != 4
        || !matches!(route[0], "season" | "serial" | "seria" | "video")
        || !decimal(route[1])
        || !hash(route[2])
        || route[3] != "720p"
        || !decimal(&selected.number)
    {
        return Err(Error::InvalidResponse("invalid Kodik player identity"));
    }
    let playlist = matches!(route[0], "season" | "serial");
    let episodes: Vec<_> = player
        .query_pairs()
        .filter(|(key, _)| key == "episode")
        .map(|(_, value)| value.into_owned())
        .collect();
    if episodes.len() > 1
        || (playlist || !episodes.is_empty()) && episodes.first() != Some(&selected.number)
        || route[0] == "video" && selected.number != "1"
    {
        return Err(Error::VariantUnavailable);
    }

    // The ordinary season page retains its episode selector. The embedded
    // only_episode page omits the evidence needed to reject another episode.
    let mut page = player.clone();
    page.set_query(None);
    page.query_pairs_mut().extend_pairs(
        player
            .query_pairs()
            .filter(|(key, _)| key != "only_episode" && key != "only_season"),
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        REFERER,
        HeaderValue::from_str(page.as_str()).map_err(|_| Error::UnsafeAddress)?,
    );
    let http = SourceHttp::new(origin.clone(), headers)?;
    let (final_page, html) = http.html_page(page).await?;
    let final_episodes: Vec<_> = final_page
        .query_pairs()
        .filter(|(key, _)| key == "episode")
        .map(|(_, value)| value.into_owned())
        .collect();
    if final_page.path() != player.path() || final_episodes != episodes {
        return Err(Error::VariantUnavailable);
    }
    let form = player_form(&html, selected, &route)?;
    let sources: Sources = http
        .form_json(origin.join("ftor").expect("fixed Kodik form path"), &form)
        .await?;
    let (advertised, quality) = media_source(sources, player)?;

    // Current public Kodik manifests and segments also accept bare requests.
    // Validate the same descriptor qplay receives, without metadata headers.
    let media = SourceHttp::new(origin, HeaderMap::new())?;
    let (url, manifest) = media.playlist(advertised).await?;
    if !manifest.starts_with("#EXTM3U")
        || !manifest
            .lines()
            .any(|line| line.starts_with("#EXTINF:") || line.starts_with("#EXT-X-STREAM-INF:"))
    {
        return Err(Error::InvalidResponse("invalid Kodik HLS playlist"));
    }
    Ok((url, quality))
}

fn player_form(html: &str, selected: &Video, route: &[&str]) -> Result<String> {
    let document = Html::parse_document(html);
    let scripts = Selector::parse("script:not([src])").expect("fixed script selector");
    let script = document
        .select(&scripts)
        .flat_map(|element| element.text())
        .collect::<Vec<_>>()
        .join("\n");
    let translation = player_string(&script, "translationTitle")?;
    let voice = translation.strip_suffix(".Subtitles");
    let expected = selected
        .data
        .dubbing
        .strip_prefix("Озвучка ")
        .or_else(|| selected.data.dubbing.strip_prefix("Субтитры "))
        .unwrap_or(&selected.data.dubbing);
    if voice.is_some() != selected.is_subtitled()
        || normalized_voice(voice.unwrap_or(&translation)) != normalized_voice(expected)
    {
        return Err(Error::VariantUnavailable);
    }

    let kind = video_field(&script, "type")?;
    let id = video_field(&script, "id")?;
    let video_hash = video_field(&script, "hash")?;
    let expected_kind = if route[0] == "video" {
        "video"
    } else {
        "seria"
    };
    if kind != expected_kind || !decimal(id) || !hash(video_hash) {
        return Err(Error::InvalidResponse("invalid Kodik video identity"));
    }
    if matches!(route[0], "season" | "serial") {
        let options =
            Selector::parse(".serial-series-box select option").expect("fixed episode selector");
        let mut matching = document
            .select(&options)
            .filter(|option| option.value().attr("value") == Some(selected.number.as_str()));
        let option = matching.next().ok_or(Error::VariantUnavailable)?;
        if matching.next().is_some()
            || option.value().attr("selected").is_none()
            || option.value().attr("data-other-translation") != Some("false")
            || option.value().attr("data-id") != Some(id)
            || option.value().attr("data-hash") != Some(video_hash)
        {
            return Err(Error::VariantUnavailable);
        }
    } else if id != route[1] || video_hash != route[2] {
        return Err(Error::VariantUnavailable);
    }

    let mut form = url::form_urlencoded::Serializer::new(String::new());
    for name in ["domain", "d_sign", "pd", "pd_sign", "ref", "ref_sign"] {
        let value = player_string(&script, name)?;
        form.append_pair(if name == "domain" { "d" } else { name }, &value);
    }
    form.append_pair("type", kind)
        .append_pair("id", id)
        .append_pair("hash", video_hash)
        .append_pair("info", "{}")
        .append_pair("cdn_is_working", "false");
    Ok(form.finish())
}

fn script_value<'a>(script: &'a str, name: &str) -> Result<&'a str> {
    let mut values = script.lines().filter_map(|line| {
        line.trim()
            .strip_prefix(name)?
            .trim_start()
            .strip_prefix('=')?
            .trim()
            .strip_suffix(';')
            .map(str::trim)
    });
    let value = values
        .next()
        .ok_or(Error::InvalidResponse("missing Kodik player value"))?;
    // A responsive page can declare the same title twice. Conflicting values
    // cannot establish one exact selection.
    if values.any(|other| other != value) {
        return Err(Error::InvalidResponse("ambiguous Kodik player value"));
    }
    Ok(value)
}

fn player_string(script: &str, name: &str) -> Result<String> {
    let value = script_value(script, &format!("var {name}"))?;
    serde_json::from_str(value).map_err(|_| Error::InvalidResponse("invalid Kodik string"))
}

fn video_field<'a>(script: &'a str, name: &str) -> Result<&'a str> {
    script_value(script, &format!("vInfo.{name}"))?
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .ok_or(Error::InvalidResponse("invalid Kodik video field"))
}

fn normalized_voice(value: &str) -> String {
    let name = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    // Both public catalogs identify the same studio under these exact names.
    if name == "anilibria.tv" {
        "anilibria".into()
    } else {
        name
    }
}

fn media_source(sources: Sources, player: &Url) -> Result<(Url, VideoQuality)> {
    let mut maximum = None;
    for (name, choices) in sources.links {
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Error::InvalidResponse("invalid Kodik quality identity"));
        }
        let height = name
            .parse::<u32>()
            .map_err(|_| Error::InvalidResponse("invalid Kodik maximum quality"))?;
        if height.to_string() != name {
            return Err(Error::InvalidResponse("ambiguous Kodik quality identity"));
        }
        if maximum
            .as_ref()
            .is_none_or(|(current, _)| height > *current)
        {
            maximum = Some((height, choices));
        }
    }
    let (height, choices) = maximum.ok_or(Error::VariantUnavailable)?;
    let quality = match height {
        144 => VideoQuality::P144,
        240 => VideoQuality::P240,
        360 => VideoQuality::P360,
        480 => VideoQuality::P480,
        720 => VideoQuality::P720,
        1080 => VideoQuality::P1080,
        _ => return Err(Error::InvalidResponse("unsupported Kodik maximum quality")),
    };
    if choices.len() != 1 || !choices[0].mime.to_ascii_lowercase().contains("mpegurl") {
        return Err(Error::InvalidResponse(
            "ambiguous Kodik maximum quality stream",
        ));
    }
    let source = &choices[0].src;
    if source.is_empty() || source.len() > 16 * 1024 {
        return Err(Error::InvalidResponse("invalid Kodik stream address"));
    }
    let address = if source.contains("//") {
        source.clone()
    } else {
        // The public player rotates ASCII letters by 18 before Base64 decoding.
        let mut encoded: Vec<_> = source
            .bytes()
            .map(|byte| match byte {
                b'A'..=b'Z' => b'A' + (byte - b'A' + 18) % 26,
                b'a'..=b'z' => b'a' + (byte - b'a' + 18) % 26,
                _ => byte,
            })
            .collect();
        while !encoded.len().is_multiple_of(4) {
            encoded.push(b'=');
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| Error::InvalidResponse("invalid Kodik URL encoding"))?;
        String::from_utf8(decoded)
            .map_err(|_| Error::InvalidResponse("invalid Kodik URL encoding"))?
    };
    let url = player.join(&address).map_err(|_| Error::UnsafeAddress)?;
    validate_https_url(&url)?;
    Ok((url, quality))
}

fn decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn hash(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
