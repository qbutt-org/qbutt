use serde::{Deserialize, Serialize};

use crate::{PublicationRef, Transport, Variant, Work};

/// A search row is a view over exact source publications. It never replaces
/// their identities, metadata, or independent inspection state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkGroup {
    pub anchor: PublicationRef,
    pub references: Vec<PublicationRef>,
    pub publications: Vec<Work>,
}

impl WorkGroup {
    pub fn single(work: Work) -> Self {
        Self {
            anchor: work.reference.clone(),
            references: vec![work.reference.clone()],
            publications: vec![work],
        }
    }

    pub fn anchor_work(&self) -> Option<&Work> {
        self.publications
            .iter()
            .find(|publication| publication.reference == self.anchor)
    }

    pub fn publication(&self, reference: &PublicationRef) -> Option<&Work> {
        self.publications
            .iter()
            .find(|publication| &publication.reference == reference)
    }

    pub fn is_confirmed(&self) -> bool {
        self.publications
            .iter()
            .enumerate()
            .all(|(index, publication)| {
                self.publications[..index]
                    .iter()
                    .all(|previous| confirmed_match(previous, publication))
            })
    }

    pub fn variants(&self, preferred_language: Option<&str>) -> Vec<Variant> {
        let variants = self
            .publications
            .iter()
            .flat_map(|publication| publication.variants.iter().cloned())
            .collect();
        rank_variants(variants, preferred_language)
    }
}

/// Owns one search generation's source snapshots. Rebuilding from the exact
/// publications lets a later conflicting release split an earlier association.
#[derive(Default)]
pub struct WorkAssociation {
    publications: Vec<Work>,
}

impl WorkAssociation {
    pub fn clear(&mut self) {
        self.publications.clear();
    }

    pub fn update(&mut self, work: Work) {
        if let Some(previous) = self
            .publications
            .iter_mut()
            .find(|previous| previous.reference == work.reference)
        {
            *previous = work;
        } else {
            self.publications.push(work);
        }
    }

    pub fn groups(&self) -> Vec<WorkGroup> {
        let mut groups: Vec<WorkGroup> = Vec::new();

        for publication in &self.publications {
            let candidates: Vec<usize> = groups
                .iter()
                .enumerate()
                .filter(|(_, group)| {
                    group
                        .publications
                        .iter()
                        .all(|member| confirmed_match(member, publication))
                })
                .map(|(index, _)| index)
                .collect();

            if let [index] = candidates.as_slice() {
                groups[*index]
                    .references
                    .push(publication.reference.clone());
                groups[*index].publications.push(publication.clone());
            } else {
                groups.push(WorkGroup::single(publication.clone()));
            }
        }

        groups
    }
}

fn confirmed_match(left: &Work, right: &Work) -> bool {
    if left.reference == right.reference {
        return true;
    }
    if left.category != right.category
        || left.year.zip(right.year).is_some_and(|(a, b)| a != b)
        || explicit_season(&left.title)
            .zip(explicit_season(&right.title))
            .is_some_and(|(a, b)| a != b)
    {
        return false;
    }

    let mut shared_id = false;
    for namespace in ["mal", "shikimori", "kinopoisk"] {
        if let (Some(a), Some(b)) = (
            left.identifiers.get(namespace),
            right.identifiers.get(namespace),
        ) {
            if a != b {
                return false;
            }
            shared_id = true;
        }
    }
    if shared_id {
        return true;
    }

    // A shared payload can contain a collection. Require a matching source
    // title or alias as well, without inferring translations from similar text.
    let left_titles: Vec<_> = std::iter::once(&left.title)
        .chain(&left.aliases)
        .map(|title| normalized_title(title))
        .filter(|title| !title.is_empty())
        .collect();
    let shared_title = std::iter::once(&right.title)
        .chain(&right.aliases)
        .any(|title| left_titles.contains(&normalized_title(title)));
    if !shared_title {
        return false;
    }
    left.variants.iter().any(|a| {
        a.transport == Transport::Torrent
            && a.info_hash.as_deref().is_some_and(|hash| {
                hash.len() == 40
                    && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && right.variants.iter().any(|b| {
                        b.transport == Transport::Torrent && b.info_hash.as_deref() == Some(hash)
                    })
            })
    })
}

fn explicit_season(title: &str) -> Option<u16> {
    let normalized = title.to_lowercase();
    let words = normalized.split(|character: char| !character.is_alphanumeric());
    let mut previous = "";
    for word in words {
        if matches!(previous, "season" | "сезон") {
            if let Ok(season) = word.parse() {
                return Some(season);
            }
        }
        if matches!(word, "season" | "сезон") {
            if let Ok(season) = previous.parse() {
                return Some(season);
            }
        }
        if let Some(season) = word.strip_prefix('s') {
            if let Ok(season) = season.parse() {
                return Some(season);
            }
            if let Some((season, episode)) = season.split_once('e')
                && !episode.is_empty()
                && episode.bytes().all(|byte| byte.is_ascii_digit())
                && let Ok(season) = season.parse()
            {
                return Some(season);
            }
        }
        previous = word;
    }
    None
}

fn normalized_title(title: &str) -> String {
    title
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn rank_variants(mut variants: Vec<Variant>, preferred_language: Option<&str>) -> Vec<Variant> {
    variants.sort_by(|left, right| {
        language_rank(right, preferred_language)
            .cmp(&language_rank(left, preferred_language))
            .then_with(|| left.reported_video_ads.cmp(&right.reported_video_ads))
            .then_with(|| quality_height(right).cmp(&quality_height(left)))
    });

    // Seeders order only torrent slots within an otherwise equal score band.
    let mut start = 0;
    while start < variants.len() {
        let key = (
            language_rank(&variants[start], preferred_language),
            variants[start].reported_video_ads,
            quality_height(&variants[start]),
        );
        let mut end = start + 1;
        while end < variants.len()
            && (
                language_rank(&variants[end], preferred_language),
                variants[end].reported_video_ads,
                quality_height(&variants[end]),
            ) == key
        {
            end += 1;
        }
        let mut torrents: Vec<_> = variants[start..end]
            .iter()
            .filter(|variant| variant.transport == Transport::Torrent)
            .cloned()
            .collect();
        torrents.sort_by(|left, right| right.seeders.cmp(&left.seeders));
        let mut torrents = torrents.into_iter();
        for variant in &mut variants[start..end] {
            if variant.transport == Transport::Torrent {
                *variant = torrents.next().expect("same torrent count");
            }
        }
        start = end;
    }
    variants
}

fn language_rank(variant: &Variant, preferred: Option<&str>) -> u8 {
    let Some(preferred) = preferred else {
        return 0;
    };
    let preferred = preferred.split('-').next().unwrap_or(preferred);
    if variant.languages.is_empty() {
        return 1;
    }
    if variant
        .languages
        .iter()
        .any(|language| language.split('-').next() == Some(preferred))
    {
        2
    } else {
        0
    }
}

fn quality_height(variant: &Variant) -> u16 {
    let Some(quality) = variant.quality.as_deref() else {
        return 0;
    };
    let label = quality.trim().to_ascii_lowercase();
    match label.as_str() {
        "4k" => 2160,
        "2k" => 1440,
        _ => label
            .trim_end_matches(['p', 'i'])
            .parse::<u16>()
            .ok()
            .filter(|height| (144..=4320).contains(height))
            .unwrap_or(0),
    }
}
