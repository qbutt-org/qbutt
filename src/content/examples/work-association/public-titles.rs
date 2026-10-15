//! Public labels checked on 2026-10-14. Identities, payloads and fault decorations
//! below are generated; shared hashes do not describe the real site's releases.
//! https://anilibria.top/api/v1/app/search/releases?query=Frieren
//! https://anilibria.top/api/v1/app/search/releases?query=Berserk
//! https://www.viz.com/frieren-beyond-journeys-end
//! https://nyaa.si/?f=0&c=1_0&q=Suzume+no+Tojimari&s=id&o=desc
//! https://nyaa.si/?f=0&c=1_0&q=Sousou+no+Frieren&s=id&o=desc
//! https://rutor.info/search/0/10/100/0/Suzume/

use qbutt_content::{Category, Source, VariantSelector, Work, WorkAssociation};

use super::Result;

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
const FRIEREN_RU: &str = "Провожающая в последний путь Фрирен";
const FRIEREN_EN: &str = "Frieren: Beyond Journey’s End";
const FRIEREN_ROMAJI: &str = "Sousou no Frieren";
const SUZUME: &str = "Suzume no Tojimari";
const SUZUME_RUTOR: &str = "Судзумэ, закрывающая двери / Suzume no Tojimari (2022) BDRip 1080p от FortunaTV | D | Crunchyroll";
const SUZUME_WEB: &str =
    "Судзумэ, закрывающая двери / Suzume no tojimari (2022) WEB-DL 1080p | D | Flarrow Films";
const SUZUME_NYAA: &str = "Suzume no Tojimari (2022) [UHD.BD.2160p.HDR.x265.FLAC.ITA.JPN.Subs.Ita.Eng.Jpn] [16.8GB] [stress]";
const FRIEREN_FIRST: &str = "Frieren: Beyond Journey's End (Sousou no Frieren) - S01 (YT WEB-DL 720p VP9 Opus) (Muse/Telesuccess Filipino English dub)";
const FRIEREN_COLLECTION: &str =
    "[Xspitfire911] Sousou No Frieren S01 + S02 BDRIP 1080p X265 10bit VOSTFR";

pub(super) fn run() -> Result<()> {
    let known_frieren = release(1, FRIEREN_RU, &[FRIEREN_EN, FRIEREN_ROMAJI]);
    check_pair(
        "reported RU/EN aliases and punctuation",
        known_frieren.clone(),
        release(2, "FRIEREN-BEYOND-JOURNEY'S-END", &[]),
        1,
    )?;
    check_pair(
        "reported romaji alias",
        known_frieren.clone(),
        release(2, "SOUSOU-NO-FRIEREN", &[]),
        1,
    )?;
    check_pair(
        "RuTor release tails identify one work",
        release(1, SUZUME_RUTOR, &[]),
        release(2, SUZUME_WEB, &[]),
        1,
    )?;
    check_pair(
        "Nyaa release tail and exact title",
        release(1, SUZUME_NYAA, &[]),
        release(2, SUZUME, &[]),
        1,
    )?;
    check_pair(
        "RuTor WEB-DLRip release tail",
        release(1, SUZUME_RUTOR, &[]),
        release(
            2,
            "Судзумэ, закрывающая двери / Suzume no tojimari (2022) WEB-DLRip | D | Flarrow Films",
            &[],
        ),
        1,
    )?;

    let mut other_payload = release(2, SUZUME_WEB, &[]);
    other_payload.variants[0].info_hash = Some("f".repeat(40));
    other_payload.variants[0].reference.selector = VariantSelector::TorrentHash {
        info_hash: "f".repeat(40),
    };
    check_pair(
        "independent public releases of one work",
        release(1, SUZUME_RUTOR, &[]),
        other_payload,
        1,
    )?;
    check_pair(
        "ordinal season in a reported alias",
        release(1, FRIEREN_FIRST, &[FRIEREN_ROMAJI]),
        release(
            2,
            "Провожающая в последний путь Фрирен 2",
            &["Sousou no Frieren 2nd Season", FRIEREN_ROMAJI],
        ),
        2,
    )?;
    check_pair(
        "season collection cannot identify one season",
        release(1, FRIEREN_COLLECTION, &[FRIEREN_ROMAJI]),
        release(2, FRIEREN_FIRST, &[FRIEREN_ROMAJI]),
        2,
    )?;
    check_pair(
        "same explicit season with ordinal and numeric labels",
        release(
            1,
            "Провожающая в последний путь Фрирен 2",
            &["Sousou no Frieren 2nd Season", FRIEREN_ROMAJI],
        ),
        release(2, "Sousou no Frieren Season 2", &[FRIEREN_ROMAJI]),
        1,
    )?;
    check_pair(
        "remake years survive a shared base alias and collection hash",
        release(1, "Берсерк (2016)", &["Berserk"]),
        release(2, "Berserk (1997)", &["Berserk"]),
        2,
    )?;
    check_pair(
        "a multi-year collection cannot identify one remake",
        release(1, "Berserk (1997) + Berserk (2016)", &["Berserk"]),
        release(2, "Berserk (2016)", &["Berserk"]),
        2,
    )?;
    check_pair(
        "unrecognized derivative suffix remains part of the title",
        release(1, "Suzume no Tojimari (2022) OVA [1080p]", &[]),
        release(2, SUZUME, &[]),
        2,
    )?;
    check_pair(
        "unrecognized edition suffix remains part of the title",
        release(1, "Suzume no Tojimari (2022) [Director's Cut]", &[]),
        release(2, SUZUME, &[]),
        2,
    )?;
    check_pair(
        "a number in the title is not a release year",
        release(1, "Blade Runner 2049", &[]),
        release(2, "Blade Runner", &[]),
        2,
    )?;
    check_pair(
        "a title prefix does not identify a derivative",
        release(1, "Fate/stay night: Unlimited Blade Works", &[]),
        release(2, "Fate/stay night", &[]),
        2,
    )?;

    let mut original_id = release(1, "Berserk", &[]);
    original_id.identifiers.insert("mal".into(), "33".into());
    let mut remake_id = release(2, "Berserk", &[]);
    remake_id.identifiers.insert("mal".into(), "32379".into());
    check_pair("conflicting catalog identifiers", original_id, remake_id, 2)?;

    let mut ambiguous = release(2, FRIEREN_EN, &[]);
    ambiguous.variants.clear();
    check_pair(
        "known aliases identify a work without payload identity",
        known_frieren,
        ambiguous,
        1,
    )?;

    let mut original = release(1, "Berserk (1997)", &["Berserk"]);
    original.identifiers.insert("mal".into(), "33".into());
    let ambiguous = release(2, "Berserk", &[]);
    let mut remake = release(3, "Berserk (2016)", &["Berserk"]);
    remake.identifiers.insert("mal".into(), "32379".into());
    for publications in [
        [&original, &ambiguous, &remake],
        [&original, &remake, &ambiguous],
        [&ambiguous, &original, &remake],
        [&ambiguous, &remake, &original],
        [&remake, &original, &ambiguous],
        [&remake, &ambiguous, &original],
    ] {
        let mut association = WorkAssociation::default();
        for work in publications {
            association.update(work.clone());
        }
        if association.groups().len() != 3 {
            return Err("a late conflicting remake must undo an ambiguous association".into());
        }
        let mut corrected = remake.clone();
        corrected.title = "Berserk (1997)".into();
        corrected.identifiers.insert("mal".into(), "33".into());
        association.update(corrected);
        assert_eq!(
            association.groups().len(),
            1,
            "ambiguity must be reversible"
        );
    }
    println!("PASS late ambiguous remake in all six arrival orders");

    println!("public_title_corpus=17_pairs+1_triple orderings=40 exact_publications=preserved");
    Ok(())
}

fn release(id: usize, title: &str, aliases: &[&str]) -> Work {
    let source = if id == 1 {
        Source::RuTor
    } else {
        Source::RuTracker
    };
    let mut work = super::torrent(source, id, title, Category::Anime, HASH);
    work.year = None;
    work.aliases = aliases.iter().map(|value| (*value).into()).collect();
    work
}

fn check_pair(label: &str, left: Work, right: Work, expected: usize) -> Result<()> {
    for publications in [[left.clone(), right.clone()], [right.clone(), left.clone()]] {
        let mut association = WorkAssociation::default();
        for work in &publications {
            association.update(work.clone());
        }
        let groups = association.groups();
        if groups.len() != expected {
            return Err(
                format!("{label}: expected {expected} groups, got {}", groups.len()).into(),
            );
        }
        assert!(groups.iter().all(|group| group.is_confirmed()), "{label}");
        for work in &publications {
            assert_eq!(
                groups
                    .iter()
                    .find_map(|group| group.publication(&work.reference)),
                Some(work),
                "{label}: grouping changed an exact publication"
            );
        }
    }
    println!("PASS {label}");
    Ok(())
}
