use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    ChapterRef, Source, Transport, VariantRef, VariantSelector, Work, WorkAssociation, WorkGroup,
};

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

/// A position within original content. Text anchors survive width/font changes;
/// image page offsets remain relative to the displayed image height.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReaderPosition {
    Page { page: u32, scroll_offset: f64 },
    Text { block_id: String, offset: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderProgress {
    pub chapter: ChapterRef,
    pub position: ReaderPosition,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryEntry {
    pub group: WorkGroup,
    pub preferences: VariantPreferences,
    pub favorite: bool,
    pub last_opened: Option<LastOpened>,
    pub reader_progress: Option<ReaderProgress>,
}

impl LibraryEntry {
    pub(super) fn validate(&self) -> std::result::Result<(), &'static str> {
        if self.group.references.is_empty() || !self.group.references.contains(&self.group.anchor) {
            return Err("the work group has no resolved anchor publication");
        }
        let catalog = self
            .group
            .catalog_work()
            .ok_or("the work group has no resolved anchor publication")?;
        if !self.group.is_confirmed() {
            return Err("the resolved publications lack a confirmed association");
        }
        let mut references = HashSet::new();
        for reference in &self.group.references {
            reference
                .validate()
                .map_err(|_| "an associated publication reference is invalid")?;
            if self.group.publication(reference).is_none()
                && !reference.source.categories().contains(&catalog.category)
            {
                return Err("an associated source does not support the work category");
            }
            if !references.insert(reference) {
                return Err("an associated publication reference is repeated");
            }
        }
        let mut publications = HashSet::new();
        for work in &self.group.publications {
            validate_work(work)?;
            if !references.contains(&work.reference) || !publications.insert(&work.reference) {
                return Err("an associated publication is missing or repeated");
            }
        }

        if let Some(selected) = &self.preferences.selected {
            selected
                .validate()
                .map_err(|_| "the selected variant reference is invalid")?;
            if !references.contains(&selected.publication) {
                return Err("the selected variant belongs to another work group");
            }
        }
        for selected in self.last_opened.iter().map(|opened| &opened.variant).chain(
            self.reader_progress
                .iter()
                .map(|progress| &progress.chapter.variant),
        ) {
            if !self.group.publications.iter().any(|work| {
                work.variants
                    .iter()
                    .any(|variant| &variant.reference == selected)
            }) {
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
            let transport = self
                .group
                .publications
                .iter()
                .flat_map(|work| &work.variants)
                .find(|variant| variant.reference == progress.chapter.variant)
                .map(|variant| variant.transport);
            let offset = match &progress.position {
                ReaderPosition::Page {
                    page,
                    scroll_offset,
                } => {
                    if transport != Some(Transport::Manga) {
                        return Err("the saved page position does not belong to manga");
                    }
                    if *page > i32::MAX as u32 {
                        return Err("the saved page index exceeds the reader limit");
                    }
                    *scroll_offset
                }
                ReaderPosition::Text { block_id, offset } => {
                    if transport != Some(Transport::Text) {
                        return Err("the saved text position does not belong to text");
                    }
                    if block_id.is_empty()
                        || block_id.len() > 128
                        || block_id.chars().any(char::is_control)
                    {
                        return Err("the saved text anchor is invalid");
                    }
                    *offset
                }
            };
            if !offset.is_finite() || !(0.0..=1.0).contains(&offset) {
                return Err("the saved scroll position is invalid");
            }
        }
        Ok(())
    }

    /// An explicit work snapshot owns membership; provider metadata can still
    /// be partial. Preserve detached saved intents before replacing that set.
    pub(super) fn reconcile(&mut self, group: &WorkGroup) -> Vec<Self> {
        let detached = self.detached_entries(group);
        self.group
            .publications
            .retain(|work| group.references.contains(&work.reference));
        self.group.references.clone_from(&group.references);
        self.preferences.selected = self
            .preferences
            .selected
            .take()
            .filter(|selected| group.references.contains(&selected.publication));
        self.last_opened = self
            .last_opened
            .take()
            .filter(|opened| group.references.contains(&opened.variant.publication));
        self.reader_progress = self.reader_progress.take().filter(|progress| {
            group
                .references
                .contains(&progress.chapter.variant.publication)
        });

        for work in &group.publications {
            if let Some(previous) = self
                .group
                .publications
                .iter_mut()
                .find(|previous| previous.reference == work.reference)
            {
                refresh_publication(previous, work);
            } else {
                self.group.publications.push(work.clone());
            }
        }
        detached
    }

    fn detached_entries(&self, current: &WorkGroup) -> Vec<Self> {
        let mut association = WorkAssociation::default();
        for work in &self.group.publications {
            if !current.references.contains(&work.reference) {
                association.update(work.clone());
            }
        }
        let mut entries = Vec::new();
        for mut group in association.groups() {
            let selected = self
                .preferences
                .selected
                .clone()
                .filter(|selected| group.references.contains(&selected.publication));
            let last_opened = self
                .last_opened
                .clone()
                .filter(|opened| group.references.contains(&opened.variant.publication));
            let reader_progress = self.reader_progress.clone().filter(|progress| {
                group
                    .references
                    .contains(&progress.chapter.variant.publication)
            });
            let exact = last_opened
                .as_ref()
                .map(|opened| &opened.variant)
                .or_else(|| {
                    reader_progress
                        .as_ref()
                        .map(|progress| &progress.chapter.variant)
                })
                .or(selected.as_ref())
                .cloned();
            let Some(exact) = exact else {
                continue;
            };
            group.anchor = exact.publication.clone();
            let preferences = match selected {
                Some(selected) => VariantPreferences {
                    selected: Some(selected),
                    filters: self.preferences.filters.clone(),
                },
                None => VariantPreferences {
                    selected: Some(exact),
                    ..Default::default()
                },
            };
            entries.push(Self {
                group,
                preferences,
                favorite: false,
                last_opened,
                reader_progress,
            });
        }
        entries
    }
}

fn refresh_publication(previous: &mut Work, work: &Work) {
    let mut refreshed = work.clone();
    // Search metadata is partial; an inspected publication is authoritative.
    if work.needs_inspection {
        refreshed.description = refreshed
            .description
            .or_else(|| previous.description.clone());
        refreshed.cover = refreshed.cover.or_else(|| previous.cover.clone());
        refreshed.year = work.year.or(previous.year);
        for alias in &previous.aliases {
            if !refreshed.aliases.contains(alias) {
                refreshed.aliases.push(alias.clone());
            }
        }
        for (key, value) in &previous.identifiers {
            refreshed
                .identifiers
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    let known = refreshed
        .variants
        .iter()
        .map(|variant| variant.reference.clone())
        .collect::<HashSet<_>>();
    for variant in &previous.variants {
        if !known.contains(&variant.reference) {
            refreshed.variants.push(variant.clone());
        }
    }
    *previous = refreshed;
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
    let valid_publication_url = if work.reference.source == Source::LocalBook {
        work.url.scheme() == "qbutt-local"
            && work.url.host_str() == Some(work.reference.id.as_str())
            && work.url.port().is_none()
            && work.url.username().is_empty()
            && work.url.password().is_none()
            && work.url.path() == ""
            && work.url.query().is_none()
            && work.url.fragment().is_none()
    } else if work.reference.source == Source::Nhentai {
        work.url.as_str() == format!("https://nhentai.net/g/{}/", work.reference.id)
    } else if matches!(work.reference.source, Source::RuTracker | Source::NnmClub) {
        let host = if work.reference.source == Source::RuTracker {
            "rutracker.org"
        } else {
            "nnmclub.to"
        };
        work.url.scheme() == "https"
            && work.url.host_str() == Some(host)
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
            (Source::AniTokyo, VariantSelector::AniTokyoVideo { .. }) => {
                variant.transport == Transport::Video
            }
            (Source::AnimeSpirit, VariantSelector::AnimeSpiritKodik { .. }) => {
                variant.transport == Transport::Video
            }
            (Source::Rutube, VariantSelector::RutubeVideo) => variant.transport == Transport::Video,
            (Source::VkVideo, VariantSelector::VkVideo) => variant.transport == Transport::Video,
            (
                Source::RoyalRoad | Source::Kakuyomu | Source::Syosetu | Source::Estar,
                VariantSelector::OriginalText,
            ) => variant.transport == Transport::Text,
            (Source::LocalBook, VariantSelector::OriginalText) => {
                variant.transport == Transport::Text
            }
            (Source::LocalBook, VariantSelector::LocalPages) => {
                variant.transport == Transport::Manga
            }
            (Source::MangaDex, VariantSelector::MangaTranslation { .. })
            | (Source::WeebCentral, VariantSelector::WeebCentralPages)
            | (Source::Nhentai, VariantSelector::NhentaiPages) => {
                variant.transport == Transport::Manga
            }
            (Source::MangaLib, VariantSelector::LibTranslation { .. }) => {
                variant.transport == Transport::Manga
            }
            (Source::RanobeLib, VariantSelector::LibTranslation { .. }) => {
                variant.transport == Transport::Text
            }
            (Source::RanobeHub, VariantSelector::RanobeHubTranslation { .. }) => {
                variant.transport == Transport::Text
            }
            (
                Source::RuTor
                | Source::RuTracker
                | Source::NnmClub
                | Source::Nyaa
                | Source::FitGirl
                | Source::OnlineFix
                | Source::FreeTP
                | Source::ByRut
                | Source::Igruha
                | Source::TheLastGame
                | Source::LimeTorrents
                | Source::X1337,
                VariantSelector::TorrentHash { info_hash },
            ) => {
                variant.transport == Transport::Torrent
                    && variant.info_hash.as_deref() == Some(info_hash)
            }
            (Source::Dodi, VariantSelector::DodiTorrent { .. }) => {
                variant.transport == Transport::Torrent && variant.info_hash.is_none()
            }
            _ => false,
        };
        if !consistent {
            return Err("the variant transport does not match its source");
        }
    }
    Ok(())
}

// Other catalog and poster URLs contain no query or fragment. Forum topic
// identities are checked separately above; credentials are rejected.
fn public_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}
