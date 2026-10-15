//! Rutube's catalog and player schemas, exact identity and durable mapping.

use std::collections::BTreeMap;

use serde::Deserialize;
use url::Url;

use crate::{
    Category, Error, PublicationRef, Result, Source, Transport, Variant, VariantRef,
    VariantSelector, VideoQuality, Work,
};

#[derive(Deserialize)]
pub(super) struct Search {
    pub(super) current_page: u32,
    pub(super) results: Vec<Video>,
}

#[derive(Deserialize)]
pub(super) struct Video {
    pub(super) id: String,
    pub(super) title: String,
    description: Option<String>,
    thumbnail_url: Option<String>,
    video_url: String,
    category: VideoCategory,
    is_deleted: bool,
    is_hidden: bool,
    is_paid: bool,
    pub(super) is_locked: Option<bool>,
}

#[derive(Deserialize)]
struct VideoCategory {
    id: u32,
}

impl Video {
    pub(super) fn validate(&self) -> Result<()> {
        let reference = PublicationRef {
            source: Source::Rutube,
            id: self.id.clone(),
        };
        if reference.validate().is_err()
            || self.title.trim().is_empty()
            || self.title.len() > 2048
            || self
                .title
                .chars()
                .any(|character| character.is_control() && !character.is_whitespace())
            || self
                .description
                .as_ref()
                .is_some_and(|value| value.len() > 128 * 1024)
            || self.video_url != format!("https://rutube.ru/video/{}/", self.id)
        {
            return Err(Error::InvalidResponse(
                "invalid Rutube publication metadata",
            ));
        }
        Ok(())
    }

    pub(super) fn available(&self) -> bool {
        !self.is_deleted && !self.is_hidden && !self.is_paid && self.is_locked != Some(true)
    }

    pub(super) fn matches_query(&self, query: &str) -> bool {
        let requests_supplement = is_supplementary(query, None);
        if is_supplementary(&self.title, self.description.as_deref()) && !requests_supplement {
            return false;
        }

        let title = self.title.to_lowercase().replace('ё', "е");
        let query = query.to_lowercase().replace('ё', "е");
        let description = self
            .description
            .as_ref()
            .filter(|_| requests_supplement)
            .map(|value| value.to_lowercase().replace('ё', "е"));
        query
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .all(|word| {
                title.contains(word)
                    || description
                        .as_ref()
                        .is_some_and(|description| description.contains(word))
            })
    }

    pub(super) fn category(&self) -> Option<Category> {
        // Confirmed by Rutube's public /api/video/category/ catalog. Generic
        // entertainment, cartoons and game videos do not establish a work type.
        match self.category.id {
            4 => Some(Category::Movies),
            5 | 43 => Some(Category::Tv),
            41 => Some(Category::Anime),
            _ => None,
        }
    }

    pub(super) fn into_work(self, origin: &Url, needs_inspection: bool) -> Result<Work> {
        let category = self.category().ok_or(Error::VariantUnavailable)?;
        let reference = PublicationRef {
            source: Source::Rutube,
            id: self.id,
        };
        let url = origin
            .join(&format!("/video/{}/", reference.id))
            .expect("canonical publication path");
        let cover = self
            .thumbnail_url
            .map(|value| {
                let cover = Url::parse(&value).map_err(|_| Error::UnsafeAddress)?;
                crate::http::validate_https_url(&cover)?;
                if cover.host_str() != Some("pic.rtbcdn.ru")
                    || cover.port().is_some()
                    || cover.query().is_some()
                {
                    return Err(Error::UnsafeAddress);
                }
                Ok(cover)
            })
            .transpose()?;
        let variant = Variant {
            reference: VariantRef {
                publication: reference.clone(),
                selector: VariantSelector::RutubeVideo,
            },
            title: None,
            transport: Transport::Video,
            episode: None,
            quality: None,
            translation: None,
            languages: Vec::new(),
            player: None,
            size: None,
            seeders: None,
            info_hash: None,
            reported_video_ads: false,
        };
        Ok(Work {
            reference,
            title: self.title,
            category,
            url,
            cover,
            description: self.description,
            year: None,
            aliases: Vec::new(),
            identifiers: BTreeMap::new(),
            variants: vec![variant],
            notes: Vec::new(),
            needs_inspection,
        })
    }
}

#[derive(Deserialize)]
pub(super) struct Options {
    video_id: String,
    effective_video: String,
    has_video: bool,
    is_hidden: bool,
    acl_access: Access,
    limits: Vec<serde_json::Value>,
    drm_token: Option<String>,
    video_balancer: Balancer,
}

#[derive(Deserialize)]
struct Access {
    allowed: bool,
}

#[derive(Deserialize)]
struct Balancer {
    m3u8: Option<String>,
}

impl Options {
    pub(super) fn stream(self, id: &str) -> Result<Url> {
        if self.video_id != id || self.effective_video != id {
            return Err(Error::VariantUnavailable);
        }
        if !self.has_video
            || self.is_hidden
            || !self.acl_access.allowed
            || !self.limits.is_empty()
            || self.drm_token.is_some()
        {
            return Err(Error::VariantUnavailable);
        }
        let address = self.video_balancer.m3u8.ok_or(Error::VariantUnavailable)?;
        if address.len() > 16 * 1024 {
            return Err(Error::InvalidResponse("invalid Rutube stream address"));
        }
        let url = Url::parse(&address).map_err(|_| Error::UnsafeAddress)?;
        crate::http::validate_https_url(&url)?;
        Ok(url)
    }
}

pub(super) fn publication_query(query: &str) -> Result<Option<PublicationRef>> {
    let Ok(url) = Url::parse(query) else {
        return Ok(None);
    };
    if !matches!(url.scheme(), "http" | "https") {
        return Ok(None);
    }
    if !matches!(url.host_str(), Some("rutube.ru" | "www.rutube.ru"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(Error::InvalidQuery);
    }
    let path = url.path().trim_end_matches('/');
    let id = ["/video/", "/embed/", "/play/embed/"]
        .into_iter()
        .find_map(|prefix| path.strip_prefix(prefix))
        .ok_or(Error::InvalidQuery)?;
    let reference = PublicationRef {
        source: Source::Rutube,
        id: id.to_ascii_lowercase(),
    };
    reference.validate()?;
    Ok(Some(reference))
}

pub(crate) fn is_supplementary(title: &str, description: Option<&str>) -> bool {
    let mut introduction = description
        .unwrap_or_default()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty());
    if introduction
        .next()
        .is_some_and(|word| word.to_lowercase() == "выпустили")
        && introduction
            .next()
            .is_some_and(|word| word.to_lowercase() == "трейлер")
    {
        return true;
    }

    let title = title.to_lowercase().replace('ё', "е");
    let words: Vec<_> = title
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    words.windows(3).any(|phrase| {
        matches!(
            phrase,
            ["за", count, "минут" | "минуту" | "минуты"]
                | ["in", count, "minute" | "minutes"]
                if count.chars().all(|character| character.is_ascii_digit())
        )
    }) || words.into_iter().any(|word| {
        matches!(
            word,
            "trailer"
                | "teaser"
                | "reaction"
                | "review"
                | "highlights"
                | "promo"
                | "preview"
                | "opening"
                | "ending"
                | "amv"
                | "edit"
                | "edits"
                | "clip"
                | "clips"
                | "recap"
                | "slowed"
        ) || [
            "трейлер",
            "тизер",
            "реакци",
            "обзор",
            "нарезк",
            "промо",
            "превью",
            "опенинг",
            "эндинг",
            "амв",
            "эдит",
            "клип",
            "пересказ",
        ]
        .iter()
        .any(|prefix| word.starts_with(prefix))
    })
}

pub(super) fn select_stream(manifest: &str, base: &Url) -> Result<(Url, VideoQuality)> {
    let mut lines = manifest.lines();
    if lines.next() != Some("#EXTM3U") {
        return Err(Error::InvalidResponse("invalid Rutube master playlist"));
    }
    let mut streams = Vec::new();
    while let Some(line) = lines.next() {
        let Some(attributes) = line.strip_prefix("#EXT-X-STREAM-INF:") else {
            continue;
        };
        let mut resolutions = attributes
            .split(',')
            .filter_map(|attribute| attribute.strip_prefix("RESOLUTION="));
        let resolution = resolutions
            .next()
            .ok_or(Error::InvalidResponse("missing Rutube stream resolution"))?;
        if resolutions.next().is_some() {
            return Err(Error::InvalidResponse("ambiguous Rutube stream resolution"));
        }
        let (width, height) = resolution
            .split_once('x')
            .ok_or(Error::InvalidResponse("invalid Rutube stream resolution"))?;
        if width
            .parse::<u32>()
            .ok()
            .filter(|width| *width > 0)
            .is_none()
        {
            return Err(Error::InvalidResponse("invalid Rutube stream resolution"));
        }
        let height = height
            .parse::<u32>()
            .map_err(|_| Error::InvalidResponse("invalid Rutube stream resolution"))?;
        let address = lines
            .next()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .ok_or(Error::InvalidResponse("missing Rutube stream address"))?;
        if address.chars().any(char::is_whitespace) {
            return Err(Error::InvalidResponse("missing Rutube stream address"));
        }
        let stream = base.join(address).map_err(|_| Error::UnsafeAddress)?;
        crate::http::validate_https_url(&stream)?;
        streams.push((height, attributes, stream));
    }
    let maximum = streams
        .iter()
        .map(|(height, _, _)| *height)
        .max()
        .ok_or(Error::InvalidResponse("missing Rutube stream variants"))?;
    let mut candidates = streams
        .into_iter()
        .filter(|(height, _, _)| *height == maximum);
    let (_, attributes, stream) = candidates.next().expect("maximum has a stream");
    // Rutube advertises the same rendition on several CDN mirrors. Preserve
    // the first declared URI only when every maximum has identical attributes;
    // differing dimensions, codecs, audio or bitrate cannot establish one choice.
    if candidates.any(|(_, other, _)| attributes != other) {
        return Err(Error::InvalidResponse("ambiguous Rutube maximum rendition"));
    }
    if attributes.split(',').any(|attribute| {
        ["AUDIO=", "VIDEO=", "SUBTITLES="]
            .iter()
            .any(|name| attribute.starts_with(name))
    }) {
        return Err(Error::InvalidResponse(
            "Rutube rendition requires external tracks",
        ));
    }
    let quality = match maximum {
        144 => VideoQuality::P144,
        240 => VideoQuality::P240,
        360 => VideoQuality::P360,
        480 => VideoQuality::P480,
        720 => VideoQuality::P720,
        1080 => VideoQuality::P1080,
        _ => {
            return Err(Error::InvalidResponse(
                "unsupported Rutube maximum resolution",
            ));
        }
    };
    Ok((stream, quality))
}

pub(super) fn validate_media_playlist(manifest: &str) -> Result<()> {
    let mut lines = manifest.lines();
    if lines.next() != Some("#EXTM3U") {
        return Err(Error::InvalidResponse("invalid Rutube media playlist"));
    }
    let mut has_segments = false;
    let mut has_duration = false;
    while let Some(line) = lines.next() {
        if line.starts_with("#EXT-X-STREAM-INF:") {
            return Err(Error::InvalidResponse(
                "Rutube rendition is another master playlist",
            ));
        }
        if let Some(duration) = line.strip_prefix("#EXTINF:") {
            if !duration
                .split(',')
                .next()
                .and_then(|duration| duration.parse::<f64>().ok())
                .is_some_and(|duration| duration.is_finite() && duration > 0.0)
                || lines.next().is_none_or(|address| {
                    address.is_empty()
                        || address.starts_with('#')
                        || address.chars().any(char::is_whitespace)
                })
            {
                return Err(Error::InvalidResponse("invalid Rutube media segment"));
            }
            has_segments = true;
        }
        has_duration |= line
            .strip_prefix("#EXT-X-TARGETDURATION:")
            .and_then(|duration| duration.parse::<u32>().ok())
            .is_some_and(|duration| duration != 0);
    }
    if !has_segments || !has_duration {
        return Err(Error::InvalidResponse("missing Rutube media segments"));
    }
    Ok(())
}
