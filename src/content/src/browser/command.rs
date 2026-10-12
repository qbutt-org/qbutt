use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use super::protocol::MAX_FRAME_BYTES;
use super::reply::*;
use super::validation::{MAX_BODY_BYTES, public_url, validate_headers};
use super::{BrowserError, Selection};

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
        // Includes qcatch's reconnect alarm and the actual browser action.
        Duration::from_secs(if matches!(self, Self::Resolve { .. }) {
            80
        } else {
            60
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
