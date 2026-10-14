use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

#[path = "playback.rs"]
pub(crate) mod playback;
pub use playback::{PlaybackAudioTrack, PlaybackMedia};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    LocalBook,
    AniLiberty,
    AniTokyo,
    YummyAnime,
    AnimeSpirit,
    MangaDex,
    MangaLib,
    RanobeLib,
    RuTor,
    RuTracker,
    #[serde(rename = "nnmclub")]
    NnmClub,
    Nyaa,
    Rutube,
    #[serde(rename = "vkvideo")]
    VkVideo,
    Dodi,
    #[serde(rename = "fitgirl")]
    FitGirl,
    #[serde(rename = "onlinefix")]
    OnlineFix,
    #[serde(rename = "freetp")]
    FreeTP,
    #[serde(rename = "byrut")]
    ByRut,
    #[serde(rename = "igruha")]
    Igruha,
    #[serde(rename = "thelastgame")]
    TheLastGame,
    #[serde(rename = "limetorrents")]
    LimeTorrents,
    #[serde(rename = "1337x")]
    X1337,
    RoyalRoad,
    Kakuyomu,
    Syosetu,
    Estar,
    RanobeHub,
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
            Source::LocalBook
                if self.id.len() == 64
                    && self
                        .id
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) =>
            {
                Ok(())
            }
            Source::LocalBook => Err(crate::Error::InvalidReference),
            Source::AniLiberty => self.aniliberty_id().map(|_| ()),
            Source::AniTokyo => self.anitokyo_id().map(|_| ()),
            Source::YummyAnime => self.yummyanime_id().map(|_| ()),
            Source::AnimeSpirit => self.animespirit_id().map(|_| ()),
            Source::RuTor => self.rutor_id().map(|_| ()),
            Source::RuTracker => self.rutracker_id().map(|_| ()),
            Source::NnmClub => self.nnmclub_id().map(|_| ()),
            Source::Nyaa => self.nyaa_id().map(|_| ()),
            Source::Rutube => self.rutube_id().map(|_| ()),
            Source::VkVideo => self.vkvideo_id().map(|_| ()),
            Source::Dodi => self.dodi_parts().map(|_| ()),
            Source::FitGirl => self.fitgirl_slug().map(|_| ()),
            Source::OnlineFix => self.online_fix_path().map(|_| ()),
            Source::FreeTP => self.freetp_id().map(|_| ()),
            Source::ByRut => self.byrut_slug().map(|_| ()),
            Source::Igruha => self.igruha_slug().map(|_| ()),
            Source::TheLastGame => self.thelastgame_slug().map(|_| ()),
            Source::LimeTorrents => self.limetorrents_path().map(|_| ()),
            Source::X1337 => self.x1337_path().map(|_| ()),
            Source::RoyalRoad => self.royalroad_id().map(|_| ()),
            Source::RanobeHub => self.ranobehub_id().map(|_| ()),
            Source::MangaLib => self.mangalib_id().map(|_| ()),
            Source::RanobeLib => self.ranobelib_id().map(|_| ()),
            Source::Kakuyomu => self.kakuyomu_id().map(|_| ()),
            Source::Syosetu => self.syosetu_id().map(|_| ()),
            Source::Estar => self.estar_id().map(|_| ()),
            Source::MangaDex if canonical_uuid(&self.id) => Ok(()),
            Source::MangaDex => Err(crate::Error::InvalidReference),
        }
    }

    /// AniLiberty uses nonzero canonical decimal release IDs.
    pub(crate) fn aniliberty_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::AniLiberty)
    }

    pub(crate) fn anitokyo_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::AniTokyo)
    }

    pub(crate) fn rutor_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::RuTor)
    }

    pub(crate) fn rutracker_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::RuTracker)
    }

    pub(crate) fn nnmclub_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::NnmClub)
    }

    pub(crate) fn nyaa_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::Nyaa)
    }

    pub(crate) fn x1337_path(&self) -> crate::Result<&str> {
        let (id, slug) = self
            .id
            .split_once('/')
            .ok_or(crate::Error::InvalidReference)?;
        if self.source != Source::X1337
            || !canonical_decimal(id)
            || slug.is_empty()
            || slug.len() > 500
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(crate::Error::InvalidReference);
        }
        Ok(&self.id)
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

    pub(crate) fn online_fix_path(&self) -> crate::Result<&str> {
        if self.source != Source::OnlineFix || self.id.len() > 250 {
            return Err(crate::Error::InvalidReference);
        }
        let (category, filename) = self
            .id
            .strip_prefix("games/")
            .and_then(|path| path.split_once('/'))
            .ok_or(crate::Error::InvalidReference)?;
        let (id, slug) = filename
            .strip_suffix(".html")
            .and_then(|filename| filename.split_once('-'))
            .ok_or(crate::Error::InvalidReference)?;
        if category.is_empty()
            || !category
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            || !canonical_decimal(id)
            || slug.is_empty()
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(crate::Error::InvalidReference);
        }
        Ok(&self.id)
    }

    pub(crate) fn freetp_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::FreeTP)
    }

    pub(crate) fn byrut_slug(&self) -> crate::Result<&str> {
        let Some((number, slug)) = self.id.split_once('-') else {
            return Err(crate::Error::InvalidReference);
        };
        if self.source != Source::ByRut
            || !canonical_decimal(number)
            || slug.is_empty()
            || slug.len() > 200
            || slug.starts_with('-')
            || slug.ends_with('-')
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(crate::Error::InvalidReference);
        }
        Ok(&self.id)
    }

    pub(crate) fn igruha_slug(&self) -> crate::Result<&str> {
        let Some((number, slug)) = self.id.split_once('-') else {
            return Err(crate::Error::InvalidReference);
        };
        if self.source != Source::Igruha
            || !canonical_decimal(number)
            || slug.is_empty()
            || slug.len() > 200
            || slug.starts_with('-')
            || slug.ends_with('-')
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(crate::Error::InvalidReference);
        }
        Ok(&self.id)
    }

    pub(crate) fn thelastgame_slug(&self) -> crate::Result<&str> {
        let slug = &self.id;
        if self.source == Source::TheLastGame
            && !slug.is_empty()
            && slug.len() <= 200
            && !slug.starts_with('-')
            && !slug.ends_with('-')
            && slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            Ok(slug)
        } else {
            Err(crate::Error::InvalidReference)
        }
    }

    pub(crate) fn limetorrents_path(&self) -> crate::Result<&str> {
        if self.source != Source::LimeTorrents || self.id.is_empty() || self.id.len() > 512 {
            return Err(crate::Error::InvalidReference);
        }
        let decoded = percent_encoding::percent_decode_str(&self.id)
            .decode_utf8()
            .map_err(|_| crate::Error::InvalidReference)?;
        if decoded
            .bytes()
            .any(|byte| matches!(byte, b'/' | b'\\' | b'?' | b'#'))
            || decoded.chars().any(char::is_control)
        {
            return Err(crate::Error::InvalidReference);
        }
        let (slug, id) = self
            .id
            .strip_suffix(".html")
            .and_then(|path| path.rsplit_once("-torrent-"))
            .ok_or(crate::Error::InvalidReference)?;
        let number = id
            .parse::<u64>()
            .map_err(|_| crate::Error::InvalidReference)?;
        if slug.is_empty() || number == 0 || number.to_string() != id {
            return Err(crate::Error::InvalidReference);
        }
        Ok(&self.id)
    }

    pub(crate) fn kakuyomu_id(&self) -> crate::Result<&str> {
        if self.source == Source::Kakuyomu && canonical_kakuyomu_id(&self.id) {
            Ok(&self.id)
        } else {
            Err(crate::Error::InvalidReference)
        }
    }

    pub(crate) fn syosetu_id(&self) -> crate::Result<&str> {
        let bytes = self.id.as_bytes();
        if self.source == Source::Syosetu
            && (6..=8).contains(&bytes.len())
            && bytes[0] == b'n'
            && bytes[1..5].iter().all(u8::is_ascii_digit)
            && bytes[5..].iter().all(u8::is_ascii_lowercase)
        {
            Ok(&self.id)
        } else {
            Err(crate::Error::InvalidReference)
        }
    }

    pub(crate) fn royalroad_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::RoyalRoad)
    }

    pub(crate) fn estar_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::Estar)
    }

    pub(crate) fn ranobehub_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::RanobeHub)
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
        self.lib_id(Source::MangaLib)
    }

    pub(crate) fn ranobelib_id(&self) -> crate::Result<u64> {
        self.lib_id(Source::RanobeLib)
    }

    fn lib_id(&self, source: Source) -> crate::Result<u64> {
        let (id, slug) = self
            .id
            .split_once("--")
            .ok_or(crate::Error::InvalidReference)?;
        if self.source != source
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

    pub(crate) fn animespirit_id(&self) -> crate::Result<u64> {
        self.decimal_id(Source::AnimeSpirit)
    }

    pub(crate) fn dodi_parts(&self) -> crate::Result<(u64, &str)> {
        let (id, slug) = self
            .id
            .split_once("--")
            .ok_or(crate::Error::InvalidReference)?;
        if self.source != Source::Dodi
            || !canonical_decimal(id)
            || slug.is_empty()
            || slug.len() > 240
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(crate::Error::InvalidReference);
        }
        let id = id.parse().map_err(|_| crate::Error::InvalidReference)?;
        Ok((id, slug))
    }

    pub(crate) fn vkvideo_id(&self) -> crate::Result<&str> {
        let Some((owner, video)) = self.id.split_once('_') else {
            return Err(crate::Error::InvalidReference);
        };
        let owner = owner.strip_prefix('-').unwrap_or(owner);
        if self.source == Source::VkVideo && canonical_decimal(owner) && canonical_decimal(video) {
            Ok(&self.id)
        } else {
            Err(crate::Error::InvalidReference)
        }
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
    DodiTorrent {
        id: String,
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
    /// The single video identified by the containing VK Video publication.
    VkVideo,
    YummyAnimeVideo {
        id: u64,
        episode: String,
        dubbing: String,
        player: String,
        player_id: u64,
    },
    AniTokyoVideo {
        id: u64,
        group_id: u64,
        episode: String,
        dubbing: String,
    },
    AnimeSpiritKodik {
        player_kind: String,
        player_id: u64,
        episode: String,
        episode_id: String,
        episode_hash: String,
        voice: String,
        kodik_translation: String,
        subtitled: bool,
    },
    MangaTranslation {
        language: String,
        groups: Vec<String>,
    },
    LibTranslation {
        /// None identifies the API's null branch, not whichever translation is default.
        branch: Option<u64>,
    },
    /// The original text published by the containing source publication.
    OriginalText,
    /// Pages stored in one local comic archive or image.
    LocalPages,
    RanobeHubTranslation {
        branch: String,
    },
}

impl VariantSelector {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        let valid = match self {
            Self::RanobeHubTranslation { branch } => {
                !branch.is_empty()
                    && branch.len() <= 100
                    && branch.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            }
            Self::RutubeVideo | Self::VkVideo | Self::OriginalText | Self::LocalPages => true,
            Self::Torrent { id } => *id != 0,
            Self::TorrentHash { info_hash } => {
                info_hash.len() == 40
                    && info_hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }
            Self::DodiTorrent { id } => {
                id.len() == 12
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
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
            Self::AniTokyoVideo {
                id,
                group_id,
                episode,
                dubbing,
            } => {
                *id != 0
                    && *group_id != 0
                    && crate::model::canonical_decimal(episode)
                    && !dubbing.trim().is_empty()
                    && dubbing.len() <= 512
                    && !dubbing.chars().any(char::is_control)
            }
            Self::AnimeSpiritKodik {
                player_kind,
                player_id,
                episode,
                episode_id,
                episode_hash,
                voice,
                kodik_translation,
                ..
            } => {
                matches!(player_kind.as_str(), "serial" | "season")
                    && *player_id != 0
                    && !episode.is_empty()
                    && episode.len() <= 16
                    && episode.bytes().all(|byte| byte.is_ascii_digit())
                    && canonical_decimal(episode_id)
                    && episode_hash.len() == 32
                    && episode_hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    && [(voice, 256), (kodik_translation, 256)]
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
    pub fn supports_video_acquisition(&self) -> bool {
        match &self.selector {
            VariantSelector::Episode { .. }
            | VariantSelector::RutubeVideo
            | VariantSelector::VkVideo
            | VariantSelector::AniTokyoVideo { .. }
            | VariantSelector::AnimeSpiritKodik { .. } => true,
            VariantSelector::YummyAnimeVideo { player, .. } => {
                player == "Плеер VK"
                    || matches!(
                        player.strip_prefix("Плеер ").unwrap_or(player),
                        "Aksor" | "CVH" | "Kodik"
                    )
            }
            _ => false,
        }
    }

    pub fn validate(&self) -> crate::Result<()> {
        self.publication.validate()?;
        self.selector.validate()?;
        let matches_source = matches!(
            (&self.publication.source, &self.selector),
            (Source::AniLiberty, VariantSelector::Torrent { .. })
                | (Source::AniLiberty, VariantSelector::Episode { .. })
                | (Source::AniLiberty, VariantSelector::Rutube { .. })
                | (Source::YummyAnime, VariantSelector::YummyAnimeVideo { .. })
                | (Source::AniTokyo, VariantSelector::AniTokyoVideo { .. })
                | (
                    Source::AnimeSpirit,
                    VariantSelector::AnimeSpiritKodik { .. }
                )
                | (Source::MangaDex, VariantSelector::MangaTranslation { .. })
                | (Source::MangaLib, VariantSelector::LibTranslation { .. })
                | (Source::RanobeLib, VariantSelector::LibTranslation { .. })
                | (Source::RuTor, VariantSelector::TorrentHash { .. })
                | (Source::RuTracker, VariantSelector::TorrentHash { .. })
                | (Source::NnmClub, VariantSelector::TorrentHash { .. })
                | (Source::Nyaa, VariantSelector::TorrentHash { .. })
                | (Source::Dodi, VariantSelector::DodiTorrent { .. })
                | (Source::FitGirl, VariantSelector::TorrentHash { .. })
                | (Source::OnlineFix, VariantSelector::TorrentHash { .. })
                | (Source::FreeTP, VariantSelector::TorrentHash { .. })
                | (Source::ByRut, VariantSelector::TorrentHash { .. })
                | (Source::Igruha, VariantSelector::TorrentHash { .. })
                | (Source::TheLastGame, VariantSelector::TorrentHash { .. })
                | (Source::LimeTorrents, VariantSelector::TorrentHash { .. })
                | (Source::X1337, VariantSelector::TorrentHash { .. })
                | (Source::Rutube, VariantSelector::RutubeVideo)
                | (Source::VkVideo, VariantSelector::VkVideo)
                | (Source::RoyalRoad, VariantSelector::OriginalText)
                | (Source::Kakuyomu, VariantSelector::OriginalText)
                | (Source::Syosetu, VariantSelector::OriginalText)
                | (Source::Estar, VariantSelector::OriginalText)
                | (
                    Source::RanobeHub,
                    VariantSelector::RanobeHubTranslation { .. }
                )
                | (Source::LocalBook, VariantSelector::OriginalText)
                | (Source::LocalBook, VariantSelector::LocalPages)
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
    Video,
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
    Text,
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
            Source::LocalBook => canonical_decimal(&self.id),
            Source::MangaDex => canonical_uuid(&self.id),
            Source::MangaLib | Source::RanobeLib | Source::RanobeHub => canonical_decimal(&self.id),
            Source::RoyalRoad => self.id.split_once('/').is_some_and(|(fiction, chapter)| {
                fiction == self.variant.publication.id && canonical_decimal(chapter)
            }),
            Source::Kakuyomu => self.id.split_once('/').is_some_and(|(work, episode)| {
                work == self.variant.publication.id && canonical_kakuyomu_id(episode)
            }),
            Source::Syosetu => self.id == "root" || canonical_decimal(&self.id),
            Source::Estar => self.id.split_once('/').is_some_and(|(work, page)| {
                work == self.variant.publication.id && canonical_decimal(page)
            }),
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

// Kakuyomu exposes decimal-string identifiers; retain their opaque identity.
pub(crate) fn canonical_kakuyomu_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
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
    /// One exact browser resolution. Its protected descriptor remains ephemeral.
    BrowserVideo {
        reference: VariantRef,
        title: String,
        media: PlaybackMedia,
    },
}
