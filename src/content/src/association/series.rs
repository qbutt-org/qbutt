//! Explicit season labels and significant series publication qualifiers.

use crate::Work;

/// A reported `Name 2nd Season` or `Name Season 2` also identifies `Name 2`.
/// Keep the number, and leave ordinary title digits and other suffixes intact.
pub(super) fn normalized_season_title(title: &str) -> Option<String> {
    let words = words(title);
    let prefix = words.len().checked_sub(2)?;
    if prefix == 0 {
        return None;
    }
    let season = season_pair(&words[prefix], &words[prefix + 1])?;
    Some(format!("{}{season}", words[..prefix].concat()))
}

pub(super) fn explicit_seasons(title: &str) -> Vec<u16> {
    let words = words(title);
    let mut seasons: Vec<_> = words
        .windows(2)
        .filter_map(|pair| season_pair(&pair[0], &pair[1]))
        .collect();
    for word in words {
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
    }
    seasons
}

#[derive(Default, PartialEq, Eq)]
pub(super) struct Qualifiers {
    parts: Vec<u16>,
    cours: Vec<u16>,
    directors_cut: bool,
}

pub(super) fn qualifiers(work: &Work) -> Qualifiers {
    let mut qualifiers = Qualifiers::default();
    for title in std::iter::once(&work.title).chain(&work.aliases) {
        let words = words(title);
        for pair in words.windows(2) {
            if let Ok(number) = pair[1].parse() {
                match pair[0].as_str() {
                    "part" | "часть" => qualifiers.parts.push(number),
                    "cour" => qualifiers.cours.push(number),
                    _ => {}
                }
            }
            qualifiers.directors_cut |= matches!(
                (pair[0].as_str(), pair[1].as_str()),
                ("directors" | "director", "cut") | ("режиссерская" | "режиссёрская", "версия")
            );
        }
        qualifiers.directors_cut |= words
            .windows(3)
            .any(|triple| triple[0] == "director" && triple[1] == "s" && triple[2] == "cut");
    }
    qualifiers.parts.sort_unstable();
    qualifiers.parts.dedup();
    qualifiers.cours.sort_unstable();
    qualifiers.cours.dedup();
    qualifiers
}

fn season_pair(first: &str, second: &str) -> Option<u16> {
    if matches!(first, "season" | "сезон") {
        return second.parse().ok();
    }
    if !matches!(second, "season" | "сезон") {
        return None;
    }
    let number = ["st", "nd", "rd", "th"]
        .into_iter()
        .find_map(|suffix| first.strip_suffix(suffix))
        .unwrap_or(first);
    number.parse().ok()
}

fn words(title: &str) -> Vec<String> {
    title
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}
