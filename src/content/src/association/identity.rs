use std::collections::HashMap;

use crate::{Category, PublicationRef, Work};

pub(super) struct ProductTitle {
    pub normalized: Vec<String>,
    pub display: String,
}

struct GameRelease<'a> {
    work: &'a Work,
    title: &'a str,
    base: &'a str,
    builds: Vec<&'a str>,
    year: Option<u16>,
}

pub(super) fn product_titles(publications: &[Work]) -> HashMap<PublicationRef, ProductTitle> {
    let games: Vec<_> = publications
        .iter()
        .map(|work| (work.category == Category::Games).then(|| game_release(work)))
        .collect();
    let mut titles = HashMap::new();

    for (index, work) in publications.iter().enumerate() {
        let title = release_title(&work.title, work.category);
        let mut display = title;
        if let Some(game) = &games[index] {
            display = game.base;
            if game.base == game.title {
                // A subtitle without an Edition/Bundle label is significant.
                // Associate it only when an explicit edition of the shorter
                // product reports the same build and original release year.
                let mut candidates = games
                    .iter()
                    .flatten()
                    .filter(|edition| {
                        let prefix = game.title.get(..edition.base.len());
                        let suffix = game.title.get(edition.base.len()..);
                        edition.base != edition.title
                            && game.year.is_some()
                            && game.year == edition.year
                            && !super::conflicting_identity([game.work, edition.work])
                            && prefix
                                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(edition.base))
                            && suffix.is_some_and(|suffix| {
                                suffix.starts_with(' ')
                                    && !suffix.chars().any(|character| character.is_ascii_digit())
                            })
                            && game
                                .builds
                                .iter()
                                .any(|build| edition.builds.contains(build))
                    })
                    .map(|edition| edition.base);
                if let Some(base) = candidates.next()
                    && candidates.all(|candidate| candidate == base)
                {
                    display = base;
                }
            }
        }

        let mut normalized = Vec::new();
        for title in std::iter::once(display).chain(work.aliases.iter().map(|alias| {
            let title = release_title(alias, work.category);
            if work.category == Category::Games {
                edition_base(title)
            } else {
                title
            }
        })) {
            // Tracker labels explicitly report localized/original names with
            // spaced slashes. A slash within Fate/stay night remains literal.
            for alias in title.split(" / ") {
                let alias = normalized_title(alias);
                if !alias.is_empty() && !normalized.contains(&alias) {
                    normalized.push(alias);
                }
            }
        }
        titles.insert(
            work.reference.clone(),
            ProductTitle {
                normalized,
                display: display.into(),
            },
        );
    }

    titles
}

fn game_release(work: &Work) -> GameRelease<'_> {
    // A provider can report a canonical alias beside its page decoration,
    // while retaining the complete source label on the exact publication.
    let source_title = work
        .aliases
        .iter()
        .find(|alias| {
            !alias.is_empty()
                && (work.title.starts_with(alias.as_str()) || work.title.ends_with(alias.as_str()))
        })
        .unwrap_or(&work.title);
    let title = release_title(source_title, work.category);
    let builds = version_metadata(source_title)
        .map(|(_, content)| {
            content
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '.')
                .map(|build| build.trim_start_matches('v'))
                .filter(|build| build.len() >= 5 && build.bytes().all(|byte| byte.is_ascii_digit()))
                .collect()
        })
        .unwrap_or_default();
    let base = edition_base(title);

    GameRelease {
        work,
        title,
        base,
        builds,
        year: work
            .year
            .or_else(|| year_labels(&work.title).next().map(|(_, _, year)| year)),
    }
}

fn edition_base(title: &str) -> &str {
    for (start, separator) in title
        .match_indices(": ")
        .chain(title.match_indices(" - "))
        .chain(title.match_indices(" – "))
        .chain(title.match_indices(" — "))
    {
        let suffix = &title[start + separator.len()..];
        let label = suffix.split(" - ").next().unwrap_or_default();
        let words: Vec<_> = label.split_whitespace().collect();
        let Some((kind, descriptors)) = words.split_last() else {
            continue;
        };
        let bundle = kind.eq_ignore_ascii_case("bundle");
        if !bundle && !kind.eq_ignore_ascii_case("edition") {
            continue;
        }
        if !descriptors.is_empty()
            && descriptors.iter().all(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "standard"
                        | "digital"
                        | "deluxe"
                        | "ultimate"
                        | "gold"
                        | "complete"
                        | "definitive"
                        | "collector's"
                        | "collector’s"
                        | "supporter"
                        | "special"
                        | "premium"
                        | "anniversary"
                        | "enhanced"
                        | "goty"
                        | "game"
                        | "of"
                        | "the"
                        | "year"
                ) || (bundle && word.bytes().all(|byte| byte.is_ascii_uppercase()))
            })
        {
            return title[..start].trim_end();
        }
    }
    title
}

pub(super) fn year_labels(title: &str) -> impl Iterator<Item = (usize, &str, u16)> {
    let parenthesized = title.match_indices('(').filter_map(move |(start, _)| {
        let (number, suffix) = title[start + 1..].split_once(')')?;
        let year = number.parse::<u16>().ok()?;
        (number.len() == 4 && (1900..=2099).contains(&year)).then_some((start, suffix, year))
    });
    let release_year = title.match_indices(' ').filter_map(move |(start, _)| {
        let (number, suffix) = title[start + 1..].split_once(' ')?;
        let year = number.parse::<u16>().ok()?;
        (number.len() == 4 && (1900..=2099).contains(&year) && video_metadata(suffix))
            .then_some((start, suffix, year))
    });
    parenthesized.chain(release_year)
}

/// Recognized metadata bounds a release tail. Unknown suffixes remain part of
/// the product title, and every reported year remains conflict evidence.
fn release_title(title: &str, category: Category) -> &str {
    let mut product = title;
    for (start, suffix, _) in year_labels(title) {
        let suffix = suffix.trim();
        let first = suffix.split_whitespace().next().unwrap_or_default();
        if suffix.is_empty()
            || video_metadata(suffix)
            || (category == Category::Games
                && ["pc", "mac", "linux"]
                    .iter()
                    .any(|platform| first.eq_ignore_ascii_case(platform)))
        {
            product = title[..start].trim_end();
            break;
        }
    }
    if category == Category::Games
        && let Some((start, _)) = version_metadata(product)
    {
        return product[..start].trim_end();
    }
    product
}

fn video_metadata(suffix: &str) -> bool {
    let suffix = suffix.trim_start_matches('[');
    let mut words = suffix.split_whitespace();
    let first = words.next().unwrap_or_default();
    let first = if first.eq_ignore_ascii_case("uhd") {
        words.next().unwrap_or_default()
    } else {
        first
    };
    let first = first.to_ascii_lowercase();
    if matches!(
        first.as_str(),
        "480p" | "576p" | "720p" | "1080p" | "1080i" | "1440p" | "2160p" | "4320p"
    ) {
        return true;
    }
    [
        "bdrip",
        "bdremux",
        "hdrip",
        "web-dl",
        "web-dlrip",
        "webrip",
        "bluray",
        "blu-ray",
        "uhd.bd",
    ]
    .iter()
    .any(|tag| {
        first
            .strip_prefix(tag)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.']))
    })
}

fn version_metadata(title: &str) -> Option<(usize, &str)> {
    for (start, _) in title.match_indices('v') {
        let previous = title[..start].chars().next_back();
        if previous
            .is_some_and(|character| !character.is_whitespace() && !matches!(character, '[' | '('))
        {
            continue;
        }
        let content = title[start + 1..].trim_start();
        if !content.starts_with(|character: char| character.is_ascii_digit()) {
            continue;
        }
        let prefix = title[..start]
            .trim_end()
            .trim_end_matches([',', '-', '–', '—', '[', '('])
            .trim_end();
        return Some((prefix.len(), content));
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
