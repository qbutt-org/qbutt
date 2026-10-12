// SPDX-License-Identifier: GPL-3.0-or-later
//! Public, durable work references for `qbutt://open`.
//!
//! This document contains only stable public references. Providers re-inspect
//! publications in the recipient's session before offering acquisition actions.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::{PublicationRef, Source, VariantRef};

const MAX_URL_BYTES: usize = 24 * 1024;
const MAX_DOCUMENT_BYTES: usize = 16 * 1024;
const MAX_PUBLICATIONS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ShareError {
    #[error("This work has no shareable publication")]
    NoPublications,
    #[error("This shared work contains too many publications")]
    TooManyPublications,
    #[error("This publication cannot be shared")]
    InvalidPublication,
    #[error("The preferred variant does not belong to this work")]
    InvalidPreference,
    #[error("This qbutt link is too large")]
    TooLarge,
    #[error("This qbutt link is invalid or unsupported")]
    InvalidLink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedWork {
    version: u8,
    publications: Vec<PublicationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred: Option<VariantRef>,
}

impl SharedWork {
    pub fn new(
        publications: Vec<PublicationRef>,
        preferred: Option<VariantRef>,
    ) -> Result<Self, ShareError> {
        let document = Self {
            version: 1,
            publications,
            preferred,
        };
        document.validate()?;
        Ok(document)
    }

    pub fn publications(&self) -> &[PublicationRef] {
        &self.publications
    }

    pub fn preferred(&self) -> Option<&VariantRef> {
        self.preferred.as_ref()
    }

    pub fn is_publication_shareable(publication: &PublicationRef) -> bool {
        public_source(publication.source) && publication.validate().is_ok()
    }

    pub fn to_url(&self) -> Result<String, ShareError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| ShareError::InvalidLink)?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(ShareError::TooLarge);
        }
        let url = format!("qbutt://open?d={}", URL_SAFE_NO_PAD.encode(bytes));
        if url.len() > MAX_URL_BYTES {
            return Err(ShareError::TooLarge);
        }
        Ok(url)
    }

    pub fn from_url(input: &str) -> Result<Self, ShareError> {
        if input.len() > MAX_URL_BYTES {
            return Err(ShareError::TooLarge);
        }
        let url = Url::parse(input).map_err(|_| ShareError::InvalidLink)?;
        if url.scheme() != "qbutt"
            || url.host_str() != Some("open")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.path() != ""
            || url.fragment().is_some()
        {
            return Err(ShareError::InvalidLink);
        }
        let mut query = url.query_pairs();
        let Some((key, value)) = query.next() else {
            return Err(ShareError::InvalidLink);
        };
        if key != "d"
            || query.next().is_some()
            || value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ShareError::InvalidLink);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(value.as_bytes())
            .map_err(|_| ShareError::InvalidLink)?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(ShareError::TooLarge);
        }
        let document: Self = serde_json::from_slice(&bytes).map_err(|_| ShareError::InvalidLink)?;
        document.validate()?;
        Ok(document)
    }

    fn validate(&self) -> Result<(), ShareError> {
        if self.version != 1 {
            return Err(ShareError::InvalidLink);
        }
        if self.publications.is_empty() {
            return Err(ShareError::NoPublications);
        }
        if self.publications.len() > MAX_PUBLICATIONS {
            return Err(ShareError::TooManyPublications);
        }
        for (index, publication) in self.publications.iter().enumerate() {
            if !Self::is_publication_shareable(publication)
                || self.publications[..index].contains(publication)
            {
                return Err(ShareError::InvalidPublication);
            }
        }
        if let Some(preferred) = &self.preferred
            && (preferred.validate().is_err()
                || !self.publications.contains(&preferred.publication))
        {
            return Err(ShareError::InvalidPreference);
        }
        Ok(())
    }
}

fn public_source(source: Source) -> bool {
    matches!(
        source,
        Source::AniLiberty
            | Source::YummyAnime
            | Source::MangaDex
            | Source::MangaLib
            | Source::RanobeLib
            | Source::RuTor
            | Source::RuTracker
            | Source::Nyaa
            | Source::Rutube
            | Source::FitGirl
            | Source::RoyalRoad
            | Source::Kakuyomu
    )
}
