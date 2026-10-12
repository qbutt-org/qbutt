//! Source choices and redacted, repeatable failures. No acquisition URLs or
//! credentials belong to these profile records.

use serde::{Deserialize, Serialize};

use crate::{Category, Error, PublicationRef, Source, VariantRef};

impl Source {
    pub const ALL: [Self; 3] = [Self::AniLiberty, Self::MangaDex, Self::RuTor];

    pub fn id(self) -> &'static str {
        match self {
            Self::AniLiberty => "aniliberty",
            Self::MangaDex => "mangadex",
            Self::RuTor => "rutor",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::AniLiberty => "AniLiberty",
            Self::MangaDex => "MangaDex",
            Self::RuTor => "RuTor",
        }
    }

    pub fn categories(self) -> &'static [Category] {
        match self {
            Self::AniLiberty => &[Category::Anime],
            Self::MangaDex => &[Category::Manga],
            Self::RuTor => &[
                Category::Movies,
                Category::Tv,
                Category::Anime,
                Category::Games,
                Category::Software,
                Category::Books,
            ],
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
}

impl SourceOperation {
    pub fn source(&self) -> Source {
        match self {
            Self::Search { source, .. } => *source,
            Self::Inspect { reference, .. } => reference.source,
            Self::Resolve(reference) => reference.publication.source,
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
        }
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
}

impl SourceFailureReason {
    /// Store a classification, never the provider's URL, response or error text.
    pub fn from_error(error: &Error) -> Option<Self> {
        match error {
            Error::InvalidQuery | Error::InvalidReference => None,
            Error::VariantUnavailable => Some(Self::VariantUnavailable),
            Error::HttpStatus(status) => Some(Self::Http(*status)),
            Error::Timeout => Some(Self::Timeout),
            Error::InvalidResponse(_) | Error::Json(_) => Some(Self::Metadata),
            Error::ResponseTooLarge => Some(Self::ResponseTooLarge),
            Error::UnsafeAddress => Some(Self::UnsafeAddress),
            Error::InvalidRedirect => Some(Self::InvalidRedirect),
            Error::Network(_) => Some(Self::Network),
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
