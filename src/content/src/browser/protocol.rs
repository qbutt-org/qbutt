use serde::{Deserialize, Serialize};

use super::BrowserError;
use super::validation::text;

pub(crate) const MAX_FRAME_BYTES: usize = 768 * 1024;

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
            "browser_check_required" => BrowserError::BrowserCheckRequired,
            "browser_timeout" => BrowserError::Timeout,
            "unsupported_multi_audio" => BrowserError::UnsupportedMultiAudio,
            "unsupported_private_tracks" => BrowserError::UnsupportedPrivateTracks,
            "capture_expired" => BrowserError::CaptureExpired,
            "selected_content_changed" => BrowserError::SelectionChanged,
            "selection_unavailable" => BrowserError::SelectionUnavailable,
            "browser_busy" => BrowserError::Busy,
            "browser_required" => BrowserError::BrowserRequired,
            _ => BrowserError::SourceUnavailable,
        }
    }
}
