use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{PublicationRef, Transport, Variant, Work};

mod identity;
use identity::{ProductTitle, product_titles, year_labels};

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
        let titles = product_titles(&self.publications);
        self.publications
            .iter()
            .enumerate()
            .all(|(index, publication)| {
                self.publications[..index]
                    .iter()
                    .all(|previous| confirmed_match(previous, publication, &titles))
            })
    }

    pub fn display_title(&self) -> String {
        let title = product_titles(&self.publications)
            .remove(&self.anchor)
            .map(|title| title.display)
            .unwrap_or_default();
        let year = self.publications.iter().find_map(|work| {
            work.year
                .or_else(|| year_labels(&work.title).next().map(|(_, _, year)| year))
        });
        match year {
            Some(year) if !title.is_empty() && year_labels(&title).next().is_none() => {
                format!("{title} ({year})")
            }
            _ => title,
        }
    }

    pub fn variants(&self, preferred_language: Option<&str>) -> Vec<Variant> {
        let variants = self
            .publications
            .iter()
            .flat_map(|publication| {
                publication
                    .variants
                    .iter()
                    .cloned()
                    .map(move |mut variant| {
                        if variant.transport == Transport::Torrent && variant.title.is_none() {
                            variant.title = Some(publication.title.clone());
                        }
                        variant
                    })
            })
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
        let titles = product_titles(&self.publications);
        self.groups_with_titles(&titles)
    }

    /// Exact product names and confirmed aliases precede partial matches.
    /// Ties depend on identity, never provider timing or a source preference.
    pub fn search_groups(&self, query: &str) -> Vec<WorkGroup> {
        let query = identity::normalized_title(query);
        let titles = product_titles(&self.publications);
        let mut groups = self.groups_with_titles(&titles);
        groups.sort_by_cached_key(|group| {
            let names = group
                .references
                .iter()
                .flat_map(|reference| &titles[reference].normalized);
            let exact = names.clone().any(|name| name == &query);
            let name = names.min().cloned().unwrap_or_default();
            let reference = group
                .references
                .iter()
                .map(|reference| (reference.source.id(), reference.id.as_str()))
                .min()
                .unwrap();
            (
                !exact,
                name,
                group.publications[0].category as u8,
                (reference.0, reference.1.to_owned()),
            )
        });
        groups
    }

    fn groups_with_titles(&self, titles: &HashMap<PublicationRef, ProductTitle>) -> Vec<WorkGroup> {
        // A generic publication can match two conflicting remakes or seasons.
        // Check every current candidate before assigning it to an earlier row.
        let ambiguous: HashSet<_> = self
            .publications
            .iter()
            .filter(|publication| {
                conflicting_identity(
                    self.publications
                        .iter()
                        .filter(|candidate| confirmed_match(publication, candidate, titles)),
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
                            .all(|member| confirmed_match(member, publication, titles))
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

fn confirmed_match(
    left: &Work,
    right: &Work,
    titles: &HashMap<PublicationRef, ProductTitle>,
) -> bool {
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
    if conflicting_identity([left, right]) {
        return false;
    }
    if shared_catalog_id {
        return true;
    }

    // Publication/payload identity belongs to the variants. A product can have
    // independent repacks and qualities, each with its own exact reference.
    let left_titles = &titles[&left.reference].normalized;
    titles[&right.reference]
        .normalized
        .iter()
        .any(|title| left_titles.contains(title))
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
