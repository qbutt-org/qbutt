use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{PublicationRef, Transport, Variant, Work};

const CATALOG_IDENTIFIERS: [&str; 3] = ["mal", "shikimori", "kinopoisk"];

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
        // A generic publication can match two conflicting remakes or seasons.
        // Check every current candidate before assigning it to an earlier row.
        let ambiguous: HashSet<_> = self
            .publications
            .iter()
            .filter(|publication| {
                conflicting_identity(
                    self.publications
                        .iter()
                        .filter(|candidate| confirmed_match(publication, candidate)),
                )
            })
            .map(|publication| publication.reference.clone())
            .collect();
        let mut groups: Vec<WorkGroup> = Vec::new();

        for publication in &self.publications {
            if ambiguous.contains(&publication.reference) {
                groups.push(WorkGroup::single(publication.clone()));
                continue;
            }

            let candidates: Vec<usize> = groups
                .iter()
                .enumerate()
                .filter(|(_, group)| {
                    !ambiguous.contains(&group.anchor)
                        && group
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
    if left.category != right.category {
        return false;
    }

    let shared_catalog_id = CATALOG_IDENTIFIERS.into_iter().any(|namespace| {
        left.identifiers
            .get(namespace)
            .zip(right.identifiers.get(namespace))
            .is_some_and(|(a, b)| a == b)
    });
    let shared_payload = left.variants.iter().any(|a| {
        a.transport == Transport::Torrent
            && a.info_hash.as_deref().is_some_and(|hash| {
                hash.len() == 40
                    && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && right.variants.iter().any(|b| {
                        b.transport == Transport::Torrent && b.info_hash.as_deref() == Some(hash)
                    })
            })
    });
    if (!shared_catalog_id && !shared_payload) || conflicting_identity([left, right]) {
        return false;
    }
    if shared_catalog_id {
        return true;
    }

    // A shared payload can contain a collection. Require a matching source
    // title or alias as well, without inferring translations from similar text.
    let left_titles: Vec<_> = std::iter::once(&left.title)
        .chain(&left.aliases)
        .map(|title| normalized_title(title))
        .filter(|title| !title.is_empty())
        .collect();
    std::iter::once(&right.title)
        .chain(&right.aliases)
        .any(|title| left_titles.contains(&normalized_title(title)))
}

fn conflicting_identity<'a>(publications: impl IntoIterator<Item = &'a Work>) -> bool {
    let mut years = Vec::new();
    let mut seasons = Vec::new();
    let mut identifiers = [None; CATALOG_IDENTIFIERS.len()];

    for work in publications {
        years.extend(work.year);
        for title in std::iter::once(&work.title).chain(&work.aliases) {
            years.extend(year_labels(title).map(|(_, _, year)| year));
            seasons.extend(explicit_seasons(title));
        }
        for (index, namespace) in CATALOG_IDENTIFIERS.into_iter().enumerate() {
            if let Some(id) = work.identifiers.get(namespace)
                && identifiers[index]
                    .replace(id)
                    .is_some_and(|previous| previous != id)
            {
                return true;
            }
        }
    }

    years.windows(2).any(|pair| pair[0] != pair[1])
        || seasons.windows(2).any(|pair| pair[0] != pair[1])
}

fn explicit_seasons(title: &str) -> Vec<u16> {
    let normalized = title.to_lowercase();
    let words = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty());
    let mut seasons = Vec::new();
    let mut previous = "";
    for word in words {
        if matches!(previous, "season" | "сезон") {
            if let Ok(season) = word.parse() {
                seasons.push(season);
            }
        }
        if matches!(word, "season" | "сезон") {
            let number = ["st", "nd", "rd", "th"]
                .into_iter()
                .find_map(|suffix| previous.strip_suffix(suffix))
                .unwrap_or(previous);
            if let Ok(season) = number.parse() {
                seasons.push(season);
            }
        }
        if let Some(season) = word.strip_prefix('s') {
            if let Ok(season) = season.parse() {
                seasons.push(season);
            }
            if let Some((season, episode)) = season.split_once('e')
                && !episode.is_empty()
                && episode.bytes().all(|byte| byte.is_ascii_digit())
                && let Ok(season) = season.parse()
            {
                seasons.push(season);
            }
        }
        previous = word;
    }
    seasons
}

fn year_labels(title: &str) -> impl Iterator<Item = (usize, &str, u16)> {
    title.match_indices('(').filter_map(move |(start, _)| {
        let (number, suffix) = title[start + 1..].split_once(')')?;
        let year = number.parse::<u16>().ok()?;
        (number.len() == 4 && (1900..=2099).contains(&year)).then_some((start, suffix, year))
    })
}

/// Only observed source tags after a parenthesized year bound a release tail.
/// Unknown title suffixes stay significant; years remain conflict evidence.
fn release_title(title: &str) -> &str {
    for (start, suffix, _) in year_labels(title) {
        let suffix = suffix.trim();
        let metadata = suffix.strip_prefix('[').unwrap_or(suffix);
        let first = metadata
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if suffix.is_empty()
            || matches!(first.as_str(), "bdrip" | "web-dl" | "web-dlrip")
            || first.starts_with("uhd.bd.")
        {
            return title[..start].trim_end();
        }
    }
    title
}

fn normalized_title(title: &str) -> String {
    release_title(title)
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
