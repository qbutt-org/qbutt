//! Source choices and redacted, repeatable failures. No acquisition URLs or
//! credentials belong to these profile records.

use serde::{Deserialize, Serialize};

use crate::{Category, ChapterRef, Error, PublicationRef, Source, VariantRef};

impl Source {
    pub const ALL: [Self; 23] = [
        Self::AniLiberty,
        Self::AniTokyo,
        Self::YummyAnime,
        Self::AnimeSpirit,
        Self::MangaDex,
        Self::MangaLib,
        Self::RanobeLib,
        Self::RuTor,
        Self::RuTracker,
        Self::NnmClub,
        Self::Nyaa,
        Self::Rutube,
        Self::FitGirl,
        Self::Dodi,
        Self::OnlineFix,
        Self::FreeTP,
        Self::ByRut,
        Self::TheLastGame,
        Self::LimeTorrents,
        Self::RoyalRoad,
        Self::Kakuyomu,
        Self::Syosetu,
        Self::RanobeHub,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::LocalBook => "local_book",
            Self::AniLiberty => "aniliberty",
            Self::AniTokyo => "anitokyo",
            Self::YummyAnime => "yani",
            Self::AnimeSpirit => "animespirit",
            Self::MangaDex => "mangadex",
            Self::MangaLib => "mangalib",
            Self::RanobeLib => "ranobelib",
            Self::RuTor => "rutor",
            Self::RuTracker => "rutracker",
            Self::NnmClub => "nnmclub",
            Self::Nyaa => "nyaa",
            Self::Rutube => "rutube",
            Self::Dodi => "dodi",
            Self::FitGirl => "fitgirl",
            Self::OnlineFix => "onlinefix",
            Self::FreeTP => "freetp",
            Self::ByRut => "byrut",
            Self::TheLastGame => "thelastgame",
            Self::LimeTorrents => "limetorrents",
            Self::RoyalRoad => "royalroad",
            Self::Kakuyomu => "kakuyomu",
            Self::Syosetu => "syosetu",
            Self::RanobeHub => "ranobehub",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::LocalBook => "Local book",
            Self::AniLiberty => "AniLiberty",
            Self::AniTokyo => "AniTokyo",
            Self::YummyAnime => "YummyAnime",
            Self::AnimeSpirit => "AnimeSpirit",
            Self::MangaDex => "MangaDex",
            Self::MangaLib => "MangaLib",
            Self::RanobeLib => "RanobeLib",
            Self::RuTor => "RuTor",
            Self::RuTracker => "RuTracker",
            Self::NnmClub => "NNM-Club",
            Self::Nyaa => "Nyaa.si",
            Self::Rutube => "Rutube",
            Self::Dodi => "DODI Repacks",
            Self::FitGirl => "FitGirl",
            Self::OnlineFix => "Online-Fix",
            Self::FreeTP => "FreeTP",
            Self::ByRut => "ByRut",
            Self::TheLastGame => "TheLastGame",
            Self::LimeTorrents => "LimeTorrents",
            Self::RoyalRoad => "Royal Road",
            Self::Kakuyomu => "Kakuyomu",
            Self::Syosetu => "小説家になろう",
            Self::RanobeHub => "RanobeHub",
        }
    }

    pub fn account_url(self) -> Option<&'static str> {
        match self {
            Self::RuTracker | Self::MangaLib | Self::RanobeLib => self.site_url(),
            _ => None,
        }
    }

    pub fn site_url(self) -> Option<&'static str> {
        Some(match self {
            Self::LocalBook => return None,
            Self::AniLiberty => "https://anilibria.top",
            Self::AniTokyo => "https://anitokyo.tv",
            Self::YummyAnime => "https://yani.tv",
            Self::AnimeSpirit => "https://my.animespirit.ru",
            Self::MangaDex => "https://mangadex.org",
            Self::MangaLib => "https://mangalib.me",
            Self::RanobeLib => "https://ranobelib.me",
            Self::RuTor => "https://rutor.info",
            Self::RuTracker => "https://rutracker.org/forum/index.php",
            Self::NnmClub => "https://nnmclub.to",
            Self::Nyaa => "https://nyaa.si",
            Self::Rutube => "https://rutube.ru",
            Self::FitGirl => "https://fitgirl-repacks.site",
            Self::OnlineFix => "https://online-fix.me",
            Self::FreeTP => "https://freetp.org",
            Self::ByRut => "https://byrutgame.org",
            Self::LimeTorrents => "https://www.limetorrents.fun",
            Self::RoyalRoad => "https://www.royalroad.com",
            Self::Kakuyomu => "https://kakuyomu.jp",
            Self::Syosetu => "https://yomou.syosetu.com",
            Self::RanobeHub => "https://ranobehub.org",
        })
    }

    pub fn categories(self) -> &'static [Category] {
        match self {
            Self::LocalBook => &[Category::Books],
            Self::AniLiberty | Self::AniTokyo | Self::YummyAnime | Self::AnimeSpirit => {
                &[Category::Anime]
            }
            Self::MangaDex | Self::MangaLib => &[Category::Manga],
            Self::RanobeLib => &[Category::Books],
            Self::Rutube => &[Category::Movies, Category::Tv, Category::Anime],
            Self::FitGirl
            | Self::OnlineFix
            | Self::FreeTP
            | Self::ByRut
            | Self::TheLastGame
            | Self::Dodi => &[Category::Games],
            Self::RoyalRoad | Self::Kakuyomu | Self::Syosetu => &[Category::Books],
            Self::RanobeHub => &[Category::Books],
            Self::Nyaa => &[
                Category::Anime,
                Category::Games,
                Category::Software,
                Category::Books,
            ],
            Self::RuTor => &[
                Category::Movies,
                Category::Tv,
                Category::Anime,
                Category::Games,
                Category::Software,
                Category::Books,
            ],
            Self::LimeTorrents => &[
                Category::Movies,
                Category::Tv,
                Category::Anime,
                Category::Games,
                Category::Software,
                Category::Books,
            ],
            Self::RuTracker => &[
                Category::Movies,
                Category::Tv,
                Category::Anime,
                Category::Games,
                Category::Software,
                Category::Books,
                Category::Manga,
            ],
            Self::NnmClub => &[Category::Movies, Category::Software],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceOperation {
    Search {
        source: Source,
        query: String,
        category: Category,
        language: Option<String>,
    },
    Inspect {
        reference: PublicationRef,
        language: Option<String>,
    },
    Resolve(VariantRef),
    Chapters(VariantRef),
    Chapter(ChapterRef),
    ReaderPage {
        reference: ChapterRef,
        page: u32,
    },
}

impl SourceOperation {
    pub fn source(&self) -> Source {
        match self {
            Self::Search { source, .. } => *source,
            Self::Inspect { reference, .. } => reference.source,
            Self::Resolve(reference) | Self::Chapters(reference) => reference.publication.source,
            Self::Chapter(reference) | Self::ReaderPage { reference, .. } => {
                reference.variant.publication.source
            }
        }
    }

    pub fn validate(&self) -> crate::Result<()> {
        match self {
            Self::Search {
                source,
                query,
                category,
                language,
            } => {
                if query.is_empty()
                    || query.trim() != query
                    || query.chars().count() > 500
                    || query.chars().any(char::is_control)
                    || !source.categories().contains(category)
                {
                    return Err(Error::InvalidQuery);
                }
                validate_language(*source, language)
            }
            Self::Inspect {
                reference,
                language,
            } => {
                reference.validate()?;
                validate_language(reference.source, language)
            }
            Self::Resolve(reference) => reference.validate(),
            Self::Chapters(reference) => {
                reference.validate()?;
                validate_reader(reference.publication.source)
            }
            Self::Chapter(reference) => reference.validate(),
            Self::ReaderPage { reference, page } => {
                reference.validate()?;
                if *page < 2000 {
                    Ok(())
                } else {
                    Err(Error::InvalidReference)
                }
            }
        }
    }
}

fn validate_reader(source: Source) -> crate::Result<()> {
    if matches!(
        source,
        Source::MangaDex
            | Source::MangaLib
            | Source::RanobeLib
            | Source::RoyalRoad
            | Source::Kakuyomu
            | Source::Syosetu
            | Source::RanobeHub
            | Source::LocalBook
    ) {
        Ok(())
    } else {
        Err(Error::InvalidReference)
    }
}

fn validate_language(source: Source, language: &Option<String>) -> crate::Result<()> {
    match (source, language) {
        (Source::MangaDex, Some(language)) if crate::model::canonical_language(language) => Ok(()),
        (Source::MangaDex, _) | (_, Some(_)) => Err(Error::InvalidQuery),
        (_, None) => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFailureReason {
    Network,
    Timeout,
    Http(u16),
    Metadata,
    ResponseTooLarge,
    UnsafeAddress,
    InvalidRedirect,
    VariantUnavailable,
    LoginRequired,
    Challenge,
    SourceCheckRequired,
    BrowserChoiceRequired,
    BrowserSession,
    SourceCheck,
    UnsupportedTorrent,
}

impl SourceFailureReason {
    /// Store a classification, never the provider's URL, response or error text.
    pub fn from_error(error: &Error) -> Option<Self> {
        match error {
            Error::InvalidQuery
            | Error::InvalidReference
            | Error::UnsupportedAcquisition(_)
            | Error::LocalBookIo(_)
            | Error::InvalidLocalBook(_)
            | Error::LocalBookChanged => None,
            Error::VariantUnavailable => Some(Self::VariantUnavailable),
            Error::HttpStatus(status) => Some(Self::Http(*status)),
            Error::Timeout => Some(Self::Timeout),
            Error::InvalidResponse(_) | Error::Json(_) => Some(Self::Metadata),
            Error::ResponseTooLarge => Some(Self::ResponseTooLarge),
            Error::UnsafeAddress => Some(Self::UnsafeAddress),
            Error::InvalidRedirect => Some(Self::InvalidRedirect),
            Error::Network(_) => Some(Self::Network),
            Error::LoginRequired => Some(Self::LoginRequired),
            Error::Challenge => Some(Self::Challenge),
            #[cfg(windows)]
            Error::SourceCheckRequired => Some(Self::SourceCheckRequired),
            Error::BrowserChoiceRequired => Some(Self::BrowserChoiceRequired),
            #[cfg(windows)]
            Error::BrowserSession(_) => Some(Self::BrowserSession),
            #[cfg(windows)]
            Error::SourceCheck(_) => Some(Self::SourceCheck),
            Error::UnsupportedTorrent => Some(Self::UnsupportedTorrent),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFailure {
    pub operation: SourceOperation,
    pub reason: SourceFailureReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSettings {
    pub source: Source,
    pub enabled: bool,
    /// An outcome may clear only the exact failure revision it observed.
    pub revision: i64,
    pub failure: Option<SourceFailure>,
}
