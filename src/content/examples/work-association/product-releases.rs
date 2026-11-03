//! Real user-reported release labels with generated publication references and
//! independent payloads. These fixtures exercise product identity, not downloads.

use qbutt_content::{Category, Source, Work, WorkAssociation};

use super::Result;

const GAME_RELEASES: [&str; 5] = [
    "Ready or Not [v 117216 + 6 DLC] (2023) PC | Portable от Pioneer",
    "Ready or Not Boiling Point [v 112610 + DLCs] (2023) PC | RePack от селезень",
    "Ready or Not: Digital Deluxe Edition - CENSORED [v 112610/v 1.4.1 + DLC's] (2023) PC | RePack от FitGirl",
    "Ready or Not: LSPD Bundle [v 66884 + DLC's] (2023) PC | RePack от FitGirl",
    "Ready or Not: Supporter Edition [v 40173 + DLC] (2023) PC | RePack от Chovka",
];

pub(super) fn run() -> Result<()> {
    let games: Vec<_> = GAME_RELEASES
        .into_iter()
        .enumerate()
        .map(|(index, title)| release(index + 1, title, Category::Games))
        .collect();
    super::cross_source_games::run(&games);
    super::game_updates::run(&games);
    let movies = [
        release(
            20,
            "Я иду искать / Ready or Not (2019) BDRip 1080p от селезень | iTunes",
            Category::Movies,
        ),
        release(
            21,
            "Я иду искать / Ready or Not (2019) BDRip-AVC L1",
            Category::Movies,
        ),
    ];
    let mut orderings = 0;
    permutations(&mut games.clone(), 0, &mut |ordered| {
        let mut association = WorkAssociation::default();
        for game in ordered {
            association.update(game.clone());
        }
        for movie in &movies {
            association.update(movie.clone());
        }
        let groups = association.groups();
        assert_eq!(groups.len(), 2, "game and film remain separate");
        let game = groups
            .iter()
            .find(|group| group.anchor_work().unwrap().category == Category::Games)
            .unwrap();
        assert_eq!(game.display_title(), "Ready or Not (2023)");
        assert_eq!(game.variants(None).len(), GAME_RELEASES.len());
        assert!(game.is_confirmed());
        for publication in &games {
            assert_eq!(game.publication(&publication.reference), Some(publication));
        }
        let movie = groups
            .iter()
            .find(|group| group.anchor_work().unwrap().category == Category::Movies)
            .unwrap();
        assert_eq!(movie.display_title(), "Я иду искать / Ready or Not (2019)");
        assert_eq!(movie.variants(None).len(), 2);
        assert!(movie.is_confirmed());
        orderings += 1;
    });
    assert_eq!(orderings, 120);

    let base = release(
        30,
        "Generated Game [v 112610] (2023) PC | RePack",
        Category::Games,
    );
    let edition = release(
        31,
        "Generated Game: Deluxe Edition [v 112610] (2023) PC | RePack",
        Category::Games,
    );
    for (label, title) in [
        (
            "numbered sequel",
            "Generated Game 2 [v 112610] (2023) PC | RePack",
        ),
        (
            "unknown subtitle",
            "Generated Game New Story [v 987654] (2023) PC | RePack",
        ),
        (
            "derivative's own edition",
            "Generated Game: New Story - Deluxe Edition [v 112610] (2023) PC | RePack",
        ),
        (
            "different year",
            "Generated Game New Story [v 112610] (2024) PC | RePack",
        ),
        (
            "unknown year",
            "Generated Game New Story [v 112610] PC | RePack",
        ),
    ] {
        let mut association = WorkAssociation::default();
        association.update(base.clone());
        association.update(edition.clone());
        association.update(release(32, title, Category::Games));
        assert_eq!(association.groups().len(), 2, "{label}");
    }
    let mut association = WorkAssociation::default();
    association.update(base.clone());
    association.update(release(
        33,
        "Generated Game New Story [v 112610] (2023) PC | RePack",
        Category::Games,
    ));
    assert_eq!(
        association.groups().len(),
        2,
        "build without an explicit edition is not evidence"
    );
    association.update(edition.clone());
    assert_eq!(
        association.groups().len(),
        1,
        "late edition supplies build evidence"
    );
    let mut changed = edition;
    changed.title = "Generated Game: Deluxe Edition [v 987654] (2023) PC | RePack".into();
    association.update(changed);
    assert_eq!(
        association.groups().len(),
        2,
        "withdrawn build evidence splits the product again"
    );

    let mut association = WorkAssociation::default();
    association.update(release(
        40,
        "Generated Game New Story [v 1.0] (2023) PC | RePack",
        Category::Games,
    ));
    association.update(release(
        41,
        "Generated Game: Deluxe Edition [v 1.0] (2023) PC | RePack",
        Category::Games,
    ));
    assert_eq!(
        association.groups().len(),
        2,
        "common semantic version is not build evidence"
    );

    let mut association = WorkAssociation::default();
    let mut subtitle = release(
        42,
        "generated game New Story [v 112610] (2023) PC | RePack",
        Category::Games,
    );
    subtitle
        .identifiers
        .insert("kinopoisk".into(), "101".into());
    let mut edition = release(
        43,
        "Generated Game: Deluxe Edition [v 112610] (2023) PC | RePack",
        Category::Games,
    );
    edition.identifiers.insert("kinopoisk".into(), "102".into());
    association.update(subtitle.clone());
    association.update(edition.clone());
    assert_eq!(
        association.groups().len(),
        2,
        "catalog conflict rejects build evidence"
    );
    edition.identifiers.clear();
    association.update(edition);
    assert_eq!(
        association.groups().len(),
        1,
        "build evidence is insensitive to ASCII case"
    );

    let mut association = WorkAssociation::default();
    association.update(release(44, "Generated Film", Category::Movies));
    association.update(release(
        45,
        "Generated Film (2020) BDRip 1080p",
        Category::Movies,
    ));
    assert_eq!(
        association.groups()[0].display_title(),
        "Generated Film (2020)"
    );

    let mut association = WorkAssociation::default();
    association.update(games[0].clone());
    let anchor = association.groups()[0].anchor.clone();
    for game in &games[2..] {
        association.update(game.clone());
        assert_eq!(association.groups()[0].anchor, anchor);
    }
    association.update(games[1].clone());
    assert_eq!(association.groups()[0].anchor, anchor);
    assert_eq!(association.groups()[0].variants(None).len(), 5);

    // Current public CoD labels share a product while keeping exact payloads.
    let mut association = WorkAssociation::default();
    let mut releases = Vec::new();
    for (index, title) in [
        "Call of Duty: Modern Warfare 4 (2026)",
        "Call of Duty: Modern Warfare 4 [Build 25909941 | Campaign Only] (2026)",
        "Call of Duty: Modern Warfare 4 (2026) (CrackFix V2) – Campaign Only [DODI Repack]",
        "Call of Duty: Modern Warfare 4 (2026) (CrackFix v2) – Campaign Only [DODI Repack]",
        "Call of Duty Modern Warfare 4 [DODI Repack]",
        "Call of Duty Modern Warfare 4 [Repack by SeleZen]",
    ]
    .into_iter()
    .enumerate()
    {
        let work = release(50 + index, title, Category::Games);
        association.update(work.clone());
        releases.push(work);
    }
    let groups = association.groups();
    assert_eq!(
        groups.len(),
        1,
        "explicit release annotations are not products"
    );
    let group = &groups[0];
    assert_eq!(
        group.display_title(),
        "Call of Duty: Modern Warfare 4 (2026)"
    );
    assert_eq!(group.variants(None).len(), releases.len());
    for work in &releases {
        assert_eq!(group.publication(&work.reference), Some(work));
    }
    for (index, title) in [
        "Call of Duty: Modern Warfare 4 (2025) [DODI Repack]",
        "Call of Duty: Modern Warfare 4 Campaign (2026) [DODI Repack]",
        "Call of Duty: Modern Warfare 4 [Build Your Own Campaign] (2026)",
        "Call of Duty: Modern Warfare 4 [CrackFix Story] (2026)",
        "Call of Duty: Modern Warfare 4 [New Story] (2026)",
    ]
    .into_iter()
    .enumerate()
    {
        association.update(release(60 + index, title, Category::Games));
    }
    assert_eq!(
        association.groups().len(),
        8,
        "years and subtitles stay separate; yearless repacks cannot choose a remake"
    );
    let mut movie = release(
        70,
        "Call of Duty: Modern Warfare 4 [DODI Repack]",
        Category::Movies,
    );
    movie.year = Some(2026);
    association.update(movie);
    assert_eq!(
        association.groups().len(),
        9,
        "game annotations do not cross categories"
    );
    println!(
        "release_metadata=build,crackfix,repack exact_labels_preserved conflicts_and_unknown_subtitles_separate"
    );

    println!(
        "product_release_corpus=game5+movie2 arrival_orders=120 exact_variants=preserved incremental_anchor=stable"
    );
    println!(
        "game_suffix_controls=sequel,derivative_edition,unknown_subtitle,year,missing_year,missing_witness,withdrawn_witness,semver,catalog_conflict split=ok"
    );
    Ok(())
}

fn release(id: usize, title: &str, category: Category) -> Work {
    let mut work = super::torrent(Source::RuTor, id, title, category, &format!("{id:040x}"));
    work.year = None;
    work.variants[0].title = Some(title.into());
    work
}

fn permutations(values: &mut [Work], index: usize, check: &mut impl FnMut(&[Work])) {
    if index == values.len() {
        check(values);
        return;
    }
    for next in index..values.len() {
        values.swap(index, next);
        permutations(values, index + 1, check);
        values.swap(index, next);
    }
}
