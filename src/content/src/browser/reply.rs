use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

use super::validation::{public_url, text, token, validate_headers};
use super::{BrowserError, Selection};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResolveKind {
    #[default]
    Video,
    File,
    Torrent,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum BrowserReply {
    Cookies(Vec<BrowserCookie>),
    Fetch(FetchReply),
    Resolved(Box<ResolvedMedia>),
    VkSearch(VkSearchReply),
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub host_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<SameSite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<f64>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SameSite {
    NoRestriction,
    Lax,
    Strict,
    Unspecified,
}

impl BrowserCookie {
    pub(super) fn valid_for(&self, url: &Url) -> bool {
        let domain = self.domain.trim_start_matches('.').to_ascii_lowercase();
        let host = url.host_str().unwrap_or_default();
        (self.name.is_empty() || token(&self.name, 256))
            && self.value.len() <= 16 * 1024
            && !self.value.chars().any(char::is_control)
            && !domain.is_empty()
            && domain.len() <= 253
            && (host == domain || !self.host_only && host.ends_with(&format!(".{domain}")))
            && self.path.starts_with('/')
            && self.path.len() <= 2048
            && !self.path.chars().any(char::is_control)
            && (url.path() == self.path
                || url.path().starts_with(&self.path)
                    && (self.path.ends_with('/')
                        || url.path().as_bytes().get(self.path.len()) == Some(&b'/')))
            && (!self.secure || url.scheme() == "https")
            && self
                .expiration_date
                .is_none_or(|date| date.is_finite() && date >= 0.0)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FetchReply {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
    pub url: Url,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserResolver {
    Direct,
    #[serde(rename = "yt-dlp")]
    YtDlp,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedMedia {
    pub kind: ResolveKind,
    pub resolver: BrowserResolver,
    pub url: Url,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_url: Option<Url>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_track: Option<BrowserAudioTrack>,
    #[serde(default)]
    pub subtitles: Vec<BrowserSubtitle>,
    #[serde(default)]
    pub selection: Selection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
}

impl ResolvedMedia {
    pub(super) fn validate(
        &self,
        source: &Url,
        selection: &Selection,
        kind: ResolveKind,
    ) -> Result<(), BrowserError> {
        if &self.selection != selection {
            return Err(BrowserError::SelectionChanged);
        }
        if self.kind != kind && !(kind == ResolveKind::File && self.kind == ResolveKind::Torrent) {
            return Err(BrowserError::InvalidResponse);
        }
        if matches!(self.resolver, BrowserResolver::YtDlp)
            && (kind != ResolveKind::Video || self.url != *source || selection.capture_id.is_none())
        {
            return Err(BrowserError::InvalidResponse);
        }
        public_url(&self.url)?;
        validate_headers(&self.headers)?;
        if let Some(url) = &self.audio_url {
            public_url(url)?;
        }
        if self.subtitles.len() > 32
            || self.subtitles.iter().any(|subtitle| {
                public_url(&subtitle.url).is_err()
                    || !text(&subtitle.title, 500)
                    || subtitle
                        .language
                        .as_ref()
                        .is_some_and(|value| !text(value, 40))
            })
            || [&self.title, &self.translation, &self.quality]
                .into_iter()
                .flatten()
                .any(|value| !text(value, 500))
            || self
                .audio_track
                .as_ref()
                .is_some_and(|track| !track.valid())
            || kind != ResolveKind::Video
                && (self.audio_url.is_some()
                    || self.audio_track.is_some()
                    || !self.subtitles.is_empty())
        {
            return Err(BrowserError::InvalidResponse);
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserAudioTrack {
    pub title: String,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
}

impl BrowserAudioTrack {
    pub(super) fn valid(&self) -> bool {
        text(&self.title, 500)
            && [&self.kind, &self.language, &self.codec]
                .into_iter()
                .flatten()
                .all(|value| text(value, 100))
            && self.channels.is_none_or(|value| (1..=64).contains(&value))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSubtitle {
    pub title: String,
    pub url: Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VkSearchReply {
    pub query: String,
    pub videos: Vec<VkSearchVideo>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VkSearchVideo {
    pub id: String,
    pub title: String,
    pub url: Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<Url>,
}

impl VkSearchVideo {
    pub(super) fn valid(&self) -> bool {
        let Some((owner, video)) = self.id.split_once('_') else {
            return false;
        };
        let owner = owner.strip_prefix('-').unwrap_or(owner);
        !owner.is_empty()
            && owner.len() <= 20
            && owner.bytes().all(|value| value.is_ascii_digit())
            && !video.is_empty()
            && video.len() <= 20
            && video.bytes().all(|value| value.is_ascii_digit())
            && self.url.as_str() == format!("https://vkvideo.ru/video{}", self.id)
            && text(&self.title, 500)
            && self.author.as_ref().is_none_or(|value| text(value, 200))
            && self
                .duration
                .is_none_or(|value| value.is_finite() && value >= 0.0)
            && self
                .cover
                .as_ref()
                .is_none_or(|value| public_url(value).is_ok())
    }
}
