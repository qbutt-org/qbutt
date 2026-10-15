//! The public LimeTorrents title, hash and reported size were read on 2026-10-15.
//! Other publication references and payloads are generated; no game is acquired.
//! https://www.limetorrents.fun/Ready-or-Not-Boiling-Point-Update-v117216-RUNE-torrent-19785524.html

use qbutt_content::{Category, Source, VariantSelector, Work, WorkAssociation, WorkGroup};
use url::Url;

const UPDATE: &str = "Ready or Not Boiling Point Update v117216-RUNE";
const UPDATE_HASH: &str = "fce375ce87d9637a9b8c967fb3a95f3d2d37c5f1";

pub(super) fn run(rutor: &[Work]) {
    let mut update = game(300, UPDATE);
    update.reference.source = Source::LimeTorrents;
    update.reference.id =
        "Ready-or-Not-Boiling-Point-Update-v117216-RUNE-torrent-19785524.html".into();
    update.url = Url::parse(&format!(
        "https://www.limetorrents.fun/{}",
        update.reference.id
    ))
    .unwrap();
    update.variants[0].reference.publication = update.reference.clone();
    update.variants[0].reference.selector = VariantSelector::TorrentHash {
        info_hash: UPDATE_HASH.into(),
    };
    update.variants[0].info_hash = Some(UPDATE_HASH.into());
    update.variants[0].size = Some(2_491_081_031);
    let mut full_release = game(301, "Ready or Not Boiling Point v142058-RUNE");
    full_release.reference.source = Source::LimeTorrents;
    full_release.reference.id =
        "Ready-or-Not-Boiling-Point-v142058-RUNE-torrent-20020797.html".into();
    full_release.url = Url::parse(&format!(
        "https://www.limetorrents.fun/{}",
        full_release.reference.id
    ))
    .unwrap();
    full_release.variants[0].reference.publication = full_release.reference.clone();

    let mut publications = rutor.to_vec();
    publications.push(update.clone());
    publications.push(full_release.clone());
    for reverse in [false, true] {
        for offset in 0..publications.len() {
            let mut ordered = publications.clone();
            if reverse {
                ordered.reverse();
            }
            ordered.rotate_left(offset);
            let mut association = WorkAssociation::default();
            for publication in &ordered {
                association.update(publication.clone());
            }
            let groups = association.groups();
            assert_eq!(groups.len(), 1);
            let group = &groups[0];
            assert_eq!(group.display_title(), "Ready or Not (2023)");
            assert_eq!(group.anchor, ordered[0].reference);
            assert!(group.is_confirmed());
            assert_eq!(
                group.publication(&full_release.reference).unwrap().year,
                None
            );
            assert_eq!(group.variants(None).len(), publications.len());
            let variants = group.variants(None);
            let variant = variants
                .iter()
                .find(|variant| variant.reference.publication == update.reference)
                .unwrap();
            assert_eq!(variant.title.as_deref(), Some(UPDATE));
            assert_eq!(variant.size, update.variants[0].size);
            assert_eq!(variant.info_hash.as_deref(), Some(UPDATE_HASH));
            assert_eq!(variant.reference, update.variants[0].reference);
            for publication in &publications {
                assert_eq!(group.publication(&publication.reference), Some(publication));
            }
        }
    }
    assert_eq!(
        WorkGroup::single(update.clone()).variants(None)[0]
            .title
            .as_deref(),
        Some(UPDATE)
    );
    assert!(
        update.variants[0].title.is_none(),
        "projection must preserve the publication snapshot"
    );

    let base = game(400, "Generated Game [v 123456] (2023) PC");
    let edition = game(401, "Generated Game: Deluxe Edition [v 123456] (2023) PC");
    let original = game(402, "Generated Game New Story [v 123456] (2023) PC");
    let update = game(403, "Generated Game New Story Update v654321-RUNE");
    let unverified = game(406, "Generated Game New Story v999999-RUNE");
    for title in [
        "Generated Game 2 Update v654321-RUNE",
        "Generated Game Other Story Update v654321-RUNE",
        "Generated Game New Story Update",
    ] {
        let mut association = WorkAssociation::default();
        for publication in [&base, &edition, &original] {
            association.update(publication.clone());
        }
        association.update(game(404, title));
        assert_eq!(
            association.groups().len(),
            2,
            "no exact release witness: {title}"
        );
    }
    for conflict in ["year", "catalog"] {
        let mut association = WorkAssociation::default();
        for publication in [&base, &edition, &original] {
            association.update(publication.clone());
        }
        let mut conflicting = update.clone();
        if conflict == "year" {
            conflicting.year = Some(2024);
        } else {
            let mut identified = original.clone();
            identified
                .identifiers
                .insert("kinopoisk".into(), "101".into());
            association.update(identified);
            conflicting
                .identifiers
                .insert("kinopoisk".into(), "102".into());
        }
        association.update(conflicting);
        assert_eq!(association.groups().len(), 2, "conflicting {conflict}");
    }

    let mut association = WorkAssociation::default();
    association.update(update.clone());
    assert_eq!(
        association.groups()[0].display_title(),
        "Generated Game New Story Update"
    );
    for publication in [&base, &edition, &original, &unverified] {
        association.update(publication.clone());
    }
    assert_eq!(
        association.groups().len(),
        1,
        "late exact original resolves the update"
    );
    let mut conflicting = original.clone();
    conflicting.reference.id = "405".into();
    conflicting.variants[0].reference.publication = conflicting.reference.clone();
    conflicting.title = "Generated Game New Story [v 123456] (2024) PC".into();
    association.update(conflicting.clone());
    assert_eq!(
        association.groups().len(),
        4,
        "two remake years make the update ambiguous"
    );
    for unresolved in [&update, &unverified] {
        let groups = association.groups();
        let group = groups
            .iter()
            .find(|group| group.publication(&unresolved.reference).is_some())
            .unwrap();
        assert_eq!(
            group.publications.len(),
            1,
            "unknown-year alias must not choose a remake"
        );
    }
    conflicting.title = "Generated Game Other Story [v 123456] (2023) PC".into();
    association.update(conflicting);
    assert_eq!(
        association.groups().len(),
        1,
        "corrected witness resolves ambiguity"
    );
    let mut changed = original;
    changed.title = "Generated Game Different Story [v 123456] (2023) PC".into();
    association.update(changed);
    assert_eq!(
        association.groups().len(),
        2,
        "withdrawn exact title splits the update again"
    );
    println!(
        "game_update_corpus=real_Lime_full+update+RuTor arrival_orders=14 exact_label_hash_size=preserved sequel_subtitle_year_catalog_ambiguity=guarded unknown_year_alias=confirmed late_witness=resolved withdrawn_witness=split"
    );
}

fn game(id: usize, title: &str) -> Work {
    let mut work = super::torrent(
        Source::RuTor,
        id,
        title,
        Category::Games,
        &format!("{id:040x}"),
    );
    work.year = None;
    work
}
