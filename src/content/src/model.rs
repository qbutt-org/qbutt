use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    AniLiberty,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationRef {
    pub source: Source,
    pub id: String,
}

impl PublicationRef {
    /// The current source uses nonzero canonical decimal release IDs.
    pub(crate) fn aniliberty_id(&self) -> crate::Result<u64> {
        if self.source != Source::AniLiberty {
            return Err(crate::Error::InvalidReference);
        }
        let id = self
            .id
            .parse::<u64>()
            .map_err(|_| crate::Error::InvalidReference)?;
        if id == 0 || self.id != id.to_string() {
            return Err(crate::Error::InvalidReference);
        }
        Ok(id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoQuality {
    P480,
    P720,
    P1080,
}

impl VideoQuality {
    pub fn height(self) -> u16 {
        match self {
            Self::P480 => 480,
            Self::P720 => 720,
            Self::P1080 => 1080,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VariantSelector {
    Torrent { id: u64 },
    Episode { id: String, quality: VideoQuality },
    Rutube { episode_id: String },
}

impl VariantSelector {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        let valid = match self {
            Self::Torrent { id } => *id != 0,
            Self::Episode { id, .. } | Self::Rutube { episode_id: id } => {
                !id.is_empty() && id.len() <= 128
            }
        };
        if valid {
            Ok(())
        } else {
            Err(crate::Error::InvalidReference)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantRef {
    pub publication: PublicationRef,
    pub selector: VariantSelector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Anime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Torrent,
    Video,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub reference: VariantRef,
    /// A publication-specific label. None uses the containing work's title.
    pub title: Option<String>,
    pub transport: Transport,
    pub episode: Option<String>,
    pub quality: Option<String>,
    pub translation: Option<String>,
    pub languages: Vec<String>,
    pub player: Option<String>,
    pub size: Option<u64>,
    pub seeders: Option<u64>,
    pub info_hash: Option<String>,
    pub reported_video_ads: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Work {
    pub reference: PublicationRef,
    pub title: String,
    pub category: Category,
    pub url: Url,
    pub cover: Option<Url>,
    pub description: Option<String>,
    pub year: Option<u16>,
    pub aliases: Vec<String>,
    pub identifiers: BTreeMap<String, String>,
    pub variants: Vec<Variant>,
    /// Search metadata still needs a publication inspection, even when the API
    /// happens to include a subset of variants in that search response.
    pub needs_inspection: bool,
}

/// A resolution is a fresh acquisition descriptor, not a persistent share link.
/// DirectVideo does not imply that a player has been launched or a stream read.
#[derive(Clone, PartialEq, Eq)]
pub enum ResolvedContent {
    Torrent {
        reference: VariantRef,
        title: String,
        magnet: Url,
    },
    DirectVideo {
        reference: VariantRef,
        title: String,
        url: Url,
        quality: VideoQuality,
    },
    /// This is the exact external publication, requiring the media resolver.
    ExternalVideo {
        reference: VariantRef,
        title: String,
        url: Url,
    },
}
