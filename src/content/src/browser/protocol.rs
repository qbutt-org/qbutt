use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::{Host, Url};

use super::BrowserError;

pub(crate) const MAX_FRAME_BYTES: usize = 768 * 1024;
const MAX_BODY_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrowserId {
    Edge,
    Chrome,
}

impl BrowserId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Edge => "edge",
            Self::Chrome => "chrome",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrowserChoice {
    #[default]
    Auto,
    Edge,
    Chrome,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub connections: Vec<BrowserConnection>,
    pub stopped: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConnection {
    pub browser: BrowserId,
    pub user_agent: String,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResolveKind {
    #[default]
    Video,
    File,
    Torrent,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FetchInit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "method", deny_unknown_fields)]
pub enum BrowserCommand {
    #[serde(rename = "cookies")]
    Cookies { url: Url },
    #[serde(rename = "fetch")]
    Fetch {
        url: Url,
        #[serde(default)]
        init: FetchInit,
    },
    #[serde(rename = "resolve")]
    Resolve {
        url: Url,
        #[serde(default)]
        selection: Selection,
        #[serde(default)]
        kind: ResolveKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        referrer: Option<Url>,
    },
    #[serde(rename = "vk-search")]
    VkSearch { url: Url, query: String },
}

impl BrowserCommand {
    pub fn vk_search(query: String) -> Result<Self, BrowserError> {
        let mut url = Url::parse("https://vkvideo.ru/").expect("constant URL");
        url.query_pairs_mut().append_pair("q", &query);
        let command = Self::VkSearch { url, query };
        command.validate()?;
        Ok(command)
    }

    pub(crate) fn validate(&self) -> Result<(), BrowserError> {
        public_url(match self {
            Self::Cookies { url }
            | Self::Fetch { url, .. }
            | Self::Resolve { url, .. }
            | Self::VkSearch { url, .. } => url,
        })
        .map_err(|_| BrowserError::InvalidRequest)?;

        match self {
            Self::Fetch { init, .. } => {
                if init.method.as_deref().is_some_and(|method| {
                    !matches!(
                        method,
                        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS"
                    )
                }) || init
                    .body
                    .as_ref()
                    .is_some_and(|body| body.len() > MAX_BODY_BYTES)
                    || validate_headers(&init.headers).is_err()
                {
                    return Err(BrowserError::InvalidRequest);
                }
            }
            Self::Resolve {
                selection,
                referrer,
                ..
            } => {
                selection.validate()?;
                if referrer
                    .as_ref()
                    .is_some_and(|url| public_url(url).is_err())
                {
                    return Err(BrowserError::InvalidRequest);
                }
            }
            Self::VkSearch { url, query } => {
                if query.is_empty()
                    || query.trim() != query
                    || query.chars().count() > 200
                    || query.chars().any(char::is_control)
                {
                    return Err(BrowserError::InvalidRequest);
                }
                let mut expected = Url::parse("https://vkvideo.ru/").expect("constant URL");
                expected.query_pairs_mut().append_pair("q", query);
                if url != &expected {
                    return Err(BrowserError::InvalidRequest);
                }
            }
            Self::Cookies { .. } => {}
        }
        if serde_json::to_vec(self)
            .map_err(|_| BrowserError::InvalidRequest)?
            .len()
            + 32
            > MAX_FRAME_BYTES
        {
            return Err(BrowserError::InvalidRequest);
        }
        Ok(())
    }

    pub(crate) fn timeout(&self) -> Duration {
        Duration::from_secs(if matches!(self, Self::Resolve { .. }) {
            45
        } else {
            20
        })
    }

    pub(crate) fn reply(&self, value: serde_json::Value) -> Result<BrowserReply, BrowserError> {
        let invalid = |_| BrowserError::InvalidResponse;
        match self {
            Self::Cookies { url } => {
                let cookies: Vec<BrowserCookie> = serde_json::from_value(value).map_err(invalid)?;
                if cookies.len() > 512 || cookies.iter().any(|cookie| !cookie.valid_for(url)) {
                    return Err(BrowserError::InvalidResponse);
                }
                Ok(BrowserReply::Cookies(cookies))
            }
            Self::Fetch { .. } => {
                let reply: FetchReply = serde_json::from_value(value).map_err(invalid)?;
                public_url(&reply.url)?;
                validate_headers(&reply.headers)?;
                if !(100..=599).contains(&reply.status) || reply.body.len() > MAX_BODY_BYTES {
                    return Err(BrowserError::InvalidResponse);
                }
                Ok(BrowserReply::Fetch(reply))
            }
            Self::Resolve {
                url,
                selection,
                kind,
                ..
            } => {
                let reply: ResolvedMedia = serde_json::from_value(value).map_err(invalid)?;
                reply.validate(url, selection, *kind)?;
                Ok(BrowserReply::Resolved(Box::new(reply)))
            }
            Self::VkSearch { query, .. } => {
                let reply: VkSearchReply = serde_json::from_value(value).map_err(invalid)?;
                if &reply.query != query
                    || reply.videos.len() > 100
                    || reply.videos.iter().any(|video| !video.valid())
                {
                    return Err(BrowserError::InvalidResponse);
                }
                Ok(BrowserReply::VkSearch(reply))
            }
        }
    }
}

impl Selection {
    fn validate(&self) -> Result<(), BrowserError> {
        if [
            &self.episode,
            &self.season,
            &self.translation_id,
            &self.player,
        ]
        .into_iter()
        .flatten()
        .any(|value| !text(value, 200))
            || self
                .capture_id
                .as_ref()
                .is_some_and(|value| !uuid_text(value))
        {
            return Err(BrowserError::InvalidRequest);
        }
        Ok(())
    }
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
    fn valid_for(&self, url: &Url) -> bool {
        let domain = self.domain.trim_start_matches('.').to_ascii_lowercase();
        let host = url.host_str().unwrap_or_default();
        token(&self.name, 256)
            && self.value.len() <= 16 * 1024
            && !self.value.chars().any(char::is_control)
            && !domain.is_empty()
            && domain.len() <= 253
            && (host == domain || !self.host_only && host.ends_with(&format!(".{domain}")))
            && self.path.starts_with('/')
            && self.path.len() <= 2048
            && !self.path.chars().any(char::is_control)
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
    fn validate(
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
    fn valid(&self) -> bool {
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
    fn valid(&self) -> bool {
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Hello {
    #[serde(rename = "type")]
    pub kind: String,
    pub browser: BrowserId,
    pub version: u8,
    pub user_agent: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl Hello {
    pub(crate) fn connection(self) -> Result<BrowserConnection, BrowserError> {
        if self.kind != "hello"
            || self.version != 1
            || !text(&self.user_agent, 1024)
            || self.capabilities.len() > 16
            || self.capabilities.iter().any(|value| {
                value.is_empty()
                    || value.len() > 40
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            })
        {
            return Err(BrowserError::InvalidResponse);
        }
        Ok(BrowserConnection {
            browser: self.browser,
            user_agent: self.user_agent,
            capabilities: self.capabilities,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Response {
    pub id: u64,
    pub result: Option<serde_json::Value>,
    pub error: Option<RemoteError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteError {
    code: String,
    // Never expose untrusted text or the optional acquisition URL to callers.
    message: String,
    url: Option<String>,
}

impl RemoteError {
    pub(crate) fn error(self) -> BrowserError {
        if self.code.len() > 64
            || self.message.len() > 4096
            || self.url.as_ref().is_some_and(|value| value.len() > 8192)
        {
            return BrowserError::InvalidResponse;
        }
        match self.code.as_str() {
            "verification_required" => BrowserError::VerificationRequired,
            "capture_expired" => BrowserError::CaptureExpired,
            "selected_content_changed" => BrowserError::SelectionChanged,
            "selection_unavailable" => BrowserError::SelectionUnavailable,
            "browser_busy" => BrowserError::Busy,
            "browser_required" => BrowserError::BrowserRequired,
            _ => BrowserError::SourceUnavailable,
        }
    }
}

fn public_url(url: &Url) -> Result<(), BrowserError> {
    crate::http::validate_media_url(url).map_err(|_| BrowserError::InvalidResponse)?;
    if url.as_str().len() > 8192
        || match url.host() {
            Some(Host::Domain(host)) => {
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

fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), BrowserError> {
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

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
}

fn token(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn uuid_text(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}
