use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{ChapterRef, Source, Transport, VariantRef, VariantSelector, Work};

/// Raw provider labels are retained so reopening a work restores its dropdowns.
/// Filters never replace the identity of an explicitly selected variant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantFilters {
    pub episode: Option<String>,
    pub translation: Option<String>,
    pub source: Option<Source>,
    pub language: Option<String>,
    pub player: Option<String>,
    pub transport: Option<Transport>,
    pub query: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantPreferences {
    pub selected: Option<VariantRef>,
    pub filters: VariantFilters,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LastOpened {
    pub variant: VariantRef,
    pub at_unix_millis: i64,
}

/// A zero-based page within an exact chapter, with a viewport-independent
/// vertical scroll fraction. Acquisition URLs and cached paths are transient.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderProgress {
    pub chapter: ChapterRef,
    pub page: u32,
    pub scroll_offset: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryEntry {
    pub work: Work,
    pub preferences: VariantPreferences,
    pub favorite: bool,
    pub last_opened: Option<LastOpened>,
    pub reader_progress: Option<ReaderProgress>,
}

impl LibraryEntry {
    pub(super) fn validate(&self) -> std::result::Result<(), &'static str> {
        validate_work(&self.work)?;

        for selected in self
            .preferences
            .selected
            .iter()
            .chain(self.last_opened.iter().map(|opened| &opened.variant))
            .chain(
                self.reader_progress
                    .iter()
                    .map(|progress| &progress.chapter.variant),
            )
        {
            if !self
                .work
                .variants
                .iter()
                .any(|variant| &variant.reference == selected)
            {
                return Err("the exact selected variant is absent from the work");
            }
        }
        if self
            .last_opened
            .as_ref()
            .is_some_and(|opened| opened.at_unix_millis < 0)
        {
            return Err("the history timestamp is invalid");
        }
        if let Some(progress) = &self.reader_progress {
            progress
                .chapter
                .validate()
                .map_err(|_| "the saved chapter reference is invalid")?;
            if progress.page > i32::MAX as u32 {
                return Err("the saved page index exceeds the reader limit");
            }
            if !progress.scroll_offset.is_finite() || !(0.0..=1.0).contains(&progress.scroll_offset)
            {
                return Err("the saved scroll position is invalid");
            }
        }
        Ok(())
    }

    pub(super) fn refresh(&mut self, work: &Work) {
        let mut refreshed = work.clone();
        // Search metadata is partial; an inspected publication is authoritative.
        if work.needs_inspection {
            refreshed.description = refreshed
                .description
                .or_else(|| self.work.description.clone());
            refreshed.cover = refreshed.cover.or_else(|| self.work.cover.clone());
            refreshed.year = work.year.or(self.work.year);
            for alias in &self.work.aliases {
                if !refreshed.aliases.contains(alias) {
                    refreshed.aliases.push(alias.clone());
                }
            }
            for (key, value) in &self.work.identifiers {
                refreshed
                    .identifiers
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
        }
        let known = work
            .variants
            .iter()
            .map(|variant| &variant.reference)
            .collect::<HashSet<_>>();
        for variant in &self.work.variants {
            if !known.contains(&variant.reference) {
                refreshed.variants.push(variant.clone());
            }
        }
        self.work = refreshed;
    }
}

fn validate_work(work: &Work) -> std::result::Result<(), &'static str> {
    work.reference
        .validate()
        .map_err(|_| "the publication reference is invalid")?;
    if work.title.trim().is_empty() {
        return Err("the work title is missing");
    }
    if !work.reference.source.categories().contains(&work.category) {
        return Err("the work category does not match its source");
    }
    let valid_publication_url = if work.reference.source == Source::RuTracker {
        work.url.scheme() == "https"
            && work.url.host_str() == Some("rutracker.org")
            && work.url.port().is_none()
            && work.url.username().is_empty()
            && work.url.password().is_none()
            && work.url.path() == "/forum/viewtopic.php"
            && work.url.query() == Some(format!("t={}", work.reference.id).as_str())
            && work.url.fragment().is_none()
    } else {
        public_url(&work.url)
    };
    if !valid_publication_url || work.cover.as_ref().is_some_and(|url| !public_url(url)) {
        return Err("the work contains a nonpublic URL");
    }

    let mut known = HashSet::new();
    for variant in &work.variants {
        if variant.reference.publication != work.reference {
            return Err("a variant belongs to another publication");
        }
        variant
            .reference
            .validate()
            .map_err(|_| "an exact variant selector is invalid")?;
        if !known.insert(&variant.reference) {
            return Err("the work contains duplicate exact variants");
        }
        let consistent = match (&work.reference.source, &variant.reference.selector) {
            (Source::AniLiberty, VariantSelector::Torrent { .. }) => {
                variant.transport == Transport::Torrent
            }
            (Source::AniLiberty, _) => variant.transport == Transport::Video,
            (Source::YummyAnime, VariantSelector::YummyAnimeVideo { .. }) => {
                variant.transport == Transport::Video
            }
            (Source::Rutube, VariantSelector::RutubeVideo) => variant.transport == Transport::Video,
            (Source::MangaDex, VariantSelector::MangaTranslation { .. }) => {
                variant.transport == Transport::Manga
            }
            (Source::MangaLib, VariantSelector::LibTranslation { .. }) => {
                variant.transport == Transport::Manga
            }
            (
                Source::RuTor | Source::RuTracker | Source::Nyaa | Source::FitGirl,
                VariantSelector::TorrentHash { info_hash },
            ) => {
                variant.transport == Transport::Torrent
                    && variant.info_hash.as_deref() == Some(info_hash)
            }
            _ => false,
        };
        if !consistent {
            return Err("the variant transport does not match its source");
        }
    }
    Ok(())
}

// Other catalog and poster URLs contain no query or fragment. RuTracker's
// public topic identity is checked separately above; credentials are rejected.
fn public_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}
