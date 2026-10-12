use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    AniLiberty,
    YummyAnime,
    MangaDex,
    MangaLib,
    RuTor,
    Nyaa,
    Rutube,
    #[serde(rename = "fitgirl")]
    FitGirl,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationRef {
    pub source: Source,
    pub id: String,
}

impl PublicationRef {
    pub fn validate(&self) -> crate::Result<()> {
        match self.source {
            Source::AniLiberty => self.aniliberty_id().map(|_| ()),
            Source::YummyAnime => self.yummyanime_id().map(|_| ()),
            Source::RuTor => self.rutor_id().map(|_| ()),
            Source::Nyaa => self.nyaa_id().map(|_| ()),
            Source::Rutube => self.rutube_id().map(|_| ()),
            Source::FitGirl => self.fitgirl_slug().map(|_| ()),
            Source::MangaLib => self.mangalib_id().map(|_| ()),
            Source::MangaDex if canonical_uuid(&self.id) => Ok(()),
            Source::MangaDex => Err(crate::Error::InvalidReference),
        }
    }

    /// AniLiberty uses nonzero canonical decimal release IDs.
    pub(crate) fn aniliberty_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::AniLiberty)
    }

    pub(crate) fn rutor_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::RuTor)
    }

    pub(crate) fn nyaa_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::Nyaa)
    }

    pub(crate) fn fitgirl_slug(&self) -> crate::Result<&str> {
        if self.source == Source::FitGirl
            && !self.id.is_empty()
            && self.id.len() <= 200
            && !self.id.starts_with('-')
            && !self.id.ends_with('-')
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            Ok(&self.id)
        } else {
            Err(crate::Error::InvalidReference)
        }
    }

    pub(crate) fn rutube_id(&self) -> crate::Result<&str> {
        if self.source == Source::Rutube
            && self.id.len() == 32
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(&self.id)
        } else {
            Err(crate::Error::InvalidReference)
        }
    }

    pub(crate) fn mangalib_id(&self) -> crate::Result<u64> {
        let (id, slug) = self
            .id
            .split_once("--")
            .ok_or(crate::Error::InvalidReference)?;
        if self.source != Source::MangaLib
            || slug.is_empty()
            || slug.len() > 200
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !canonical_decimal(id)
        {
            return Err(crate::Error::InvalidReference);
        }
        id.parse().map_err(|_| crate::Error::InvalidReference)
    }

    pub(crate) fn yummyanime_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::YummyAnime)
    }

    fn decimal_id(&self, source: Source) -> crate::Result<u64> {
        if self.source != source {
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
    P144,
    P240,
    P360,
    P480,
    P720,
    P1080,
}

impl VideoQuality {
    pub fn height(self) -> u16 {
        match self {
            Self::P144 => 144,
            Self::P240 => 240,
            Self::P360 => 360,
            Self::P480 => 480,
            Self::P720 => 720,
            Self::P1080 => 1080,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VariantSelector {
    Torrent {
        id: u64,
    },
    TorrentHash {
        info_hash: String,
    },
    Episode {
        id: String,
        quality: VideoQuality,
    },
    Rutube {
        episode_id: String,
    },
    /// The single video identified by the containing Rutube publication.
    RutubeVideo,
    YummyAnimeVideo {
        id: u64,
        episode: String,
        dubbing: String,
        player: String,
        player_id: u64,
    },
    MangaTranslation {
        language: String,
        groups: Vec<String>,
    },
    LibTranslation {
        /// None identifies the API's null branch, not whichever translation is default.
        branch: Option<u64>,
    },
}

impl VariantSelector {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        let valid = match self {
            Self::RutubeVideo => true,
            Self::Torrent { id } => *id != 0,
            Self::TorrentHash { info_hash } => {
                info_hash.len() == 40
                    && info_hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }
            Self::Episode { id, .. } | Self::Rutube { episode_id: id } => {
                !id.is_empty() && id.len() <= 128
            }
            Self::YummyAnimeVideo {
                id,
                episode,
                dubbing,
                player,
                player_id,
            } => {
                *id != 0
                    && *player_id != 0
                    && [(episode, 128), (dubbing, 512), (player, 128)]
                        .into_iter()
                        .all(|(value, limit)| {
                            !value.trim().is_empty()
                                && value.len() <= limit
                                && !value.chars().any(char::is_control)
                        })
            }
            Self::MangaTranslation { language, groups } => {
                canonical_language(language)
                    && groups.len() <= 32
                    && groups.iter().all(|id| canonical_uuid(id))
                    && groups.windows(2).all(|pair| pair[0] < pair[1])
            }
            Self::LibTranslation { branch } => branch.is_none_or(|branch| branch != 0),
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

impl VariantRef {
    pub fn validate(&self) -> crate::Result<()> {
        self.publication.validate()?;
        self.selector.validate()?;
        let matches_source = matches!(
            (&self.publication.source, &self.selector),
            (Source::AniLiberty, VariantSelector::Torrent { .. })
                | (Source::AniLiberty, VariantSelector::Episode { .. })
                | (Source::AniLiberty, VariantSelector::Rutube { .. })
                | (Source::YummyAnime, VariantSelector::YummyAnimeVideo { .. })
                | (Source::MangaDex, VariantSelector::MangaTranslation { .. })
                | (Source::MangaLib, VariantSelector::LibTranslation { .. })
                | (Source::RuTor, VariantSelector::TorrentHash { .. })
                | (Source::Nyaa, VariantSelector::TorrentHash { .. })
                | (Source::FitGirl, VariantSelector::TorrentHash { .. })
                | (Source::Rutube, VariantSelector::RutubeVideo)
        );
        if matches_source {
            Ok(())
        } else {
            Err(crate::Error::InvalidReference)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Anime,
    Manga,
    Movies,
    Tv,
    Games,
    Software,
    Books,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Torrent,
    Video,
    Manga,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterRef {
    pub variant: VariantRef,
    pub id: String,
}

impl ChapterRef {
    pub fn validate(&self) -> crate::Result<()> {
        self.variant.validate()?;
        if match self.variant.publication.source {
            Source::MangaDex => canonical_uuid(&self.id),
            Source::MangaLib => canonical_decimal(&self.id),
            _ => false,
        } {
            Ok(())
        } else {
            Err(crate::Error::InvalidReference)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chapter {
    pub reference: ChapterRef,
    pub title: Option<String>,
    pub volume: Option<String>,
    pub number: Option<String>,
    pub pages: u32,
}

/// Fresh, short-lived image descriptors. Only a validated provider manifest can
/// construct these; QML selects a page index rather than supplying a URL.
#[derive(Clone, PartialEq, Eq)]
pub struct ChapterImage {
    pub(crate) url: Url,
}

impl ChapterImage {
    pub fn url(&self) -> &Url {
        &self.url
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ChapterPages {
    pub reference: ChapterRef,
    pub images: Vec<ChapterImage>,
}

pub(crate) fn canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
        && value != "00000000-0000-0000-0000-000000000000"
}

pub(crate) fn canonical_decimal(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|id| id != 0 && value == id.to_string())
}

pub(crate) fn canonical_language(value: &str) -> bool {
    matches!(value.len(), 2 | 5)
        && value.bytes().enumerate().all(|(index, byte)| {
            if index == 2 {
                byte == b'-'
            } else {
                byte.is_ascii_lowercase()
            }
        })
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

/// Acquired .torrent bytes, kept in memory until the native libtorrent handoff.
/// This is not a persistent reference and does not claim a validated infohash.
#[derive(Clone, PartialEq, Eq)]
pub struct TorrentMetadata(Vec<u8>);

impl TorrentMetadata {
    pub const MAX_BYTES: usize = 10 * 1024 * 1024;

    pub fn new(bytes: Vec<u8>) -> crate::Result<Self> {
        if bytes.is_empty() {
            return Err(crate::Error::InvalidResponse("empty torrent metadata"));
        }
        if bytes.len() > Self::MAX_BYTES {
            return Err(crate::Error::ResponseTooLarge);
        }
        Ok(Self(bytes))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
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
    TorrentFile {
        reference: VariantRef,
        title: String,
        metadata: TorrentMetadata,
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
