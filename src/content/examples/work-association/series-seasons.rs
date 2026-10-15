//! Public catalogue labels and aliases saved on 2026-10-15. Variant payloads,
//! envelope URLs and negative decorations are generated; no network is used.
//! https://api.yani.tv/anime/1246?need_videos=false
//! https://my.animespirit.ru/index.php?do=search&subaction=search&story=Re%3AZero

use qbutt_content::{Category, Source, Work, WorkAssociation};

pub(super) fn run() {
    let mut yummy = super::anime(
        Source::YummyAnime,
        1246,
        "Re:Zero. Жизнь с нуля в альтернативном мире 2",
        "39587",
    );
    yummy.year = Some(2020);
    yummy.aliases = [
        "Re: Life in a different world from zero 2nd Season",
        "Re:Zero - Starting Life in Another World 2",
        "Re:ZERO -Starting Life in Another World- Season 2",
        "Re:Zero — жизнь с нуля в другом мире 2",
        "Re:Zero kara Hajimeru Isekai Seikatsu 2nd Season",
        "Re:Zero. Життя з нуля в альтернативному світі 2",
        "Re：ゼロから始める異世界生活 2",
        "ReZero 2nd Season",
        "С нуля: Пособие по выживанию в альтернативном мире 2",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    yummy.identifiers.extend([
        ("shikimori".into(), "39587".into()),
        ("kinopoisk".into(), "971114".into()),
    ]);
    let mut spirit = super::anime(
        Source::AnimeSpirit,
        12633,
        "Re: Zero kara Hajimeru Isekai Seikatsu 2",
        "",
    );
    spirit.year = Some(2020);
    spirit.identifiers.clear();
    spirit.aliases = vec!["Заново: Жизнь в альтернативном мире с нуля [ТВ-2]".into()];
    check_pair(
        "actual second-season catalogue snapshots",
        &yummy,
        &spirit,
        1,
    );

    let mut first = yummy.clone();
    first.reference.id = "1248".into();
    first.title = "Re:Zero. Жизнь с нуля в альтернативном мире".into();
    first.aliases = vec![
        "ReZero".into(),
        "Re:Zero kara Hajimeru Isekai Seikatsu".into(),
    ];
    first.year = Some(2016);
    first.identifiers.insert("mal".into(), "31240".into());
    first.identifiers.insert("shikimori".into(), "31240".into());
    let mut first_spirit = spirit.clone();
    first_spirit.reference.id = "11037".into();
    first_spirit.title = "Re: Zero".into();
    first_spirit.aliases = vec!["Заново: Жизнь в альтернативном мире с нуля [ТВ-1]".into()];
    first_spirit.year = Some(2016);
    let mut director = first.clone();
    director.reference.id = "1249".into();
    director.title = "Re:Zero. Жизнь с нуля в альтернативном мире: Режиссерская версия".into();
    director.year = Some(2020);
    director.aliases.clear();
    let mut part_two = yummy.clone();
    part_two.reference.id = "1695".into();
    part_two.title = "Re:Zero. Жизнь с нуля в альтернативном мире 2 | Часть 2".into();
    part_two.year = Some(2021);
    part_two.aliases.clear();
    part_two.identifiers.clear();
    let actual = [
        first,
        first_spirit,
        director,
        yummy.clone(),
        spirit.clone(),
        part_two,
    ];
    for reverse in [false, true] {
        for offset in 0..actual.len() {
            let mut ordered = actual.clone();
            if reverse {
                ordered.reverse();
            }
            ordered.rotate_left(offset);
            let mut association = WorkAssociation::default();
            for work in &ordered {
                association.update_search(work.clone());
            }
            let groups = association.groups();
            assert_eq!(groups.len(), 4);
            for pair in [[&actual[0], &actual[1]], [&yummy, &spirit]] {
                let group = groups
                    .iter()
                    .find(|group| group.references.contains(&pair[0].reference))
                    .unwrap();
                assert_eq!(group.references.len(), 2);
                assert!(group.references.contains(&pair[1].reference));
                assert!(group.is_confirmed());
            }
            for separate in [&actual[2], &actual[5]] {
                assert_eq!(
                    groups
                        .iter()
                        .find(|group| group.references.contains(&separate.reference))
                        .unwrap()
                        .references
                        .len(),
                    1
                );
            }
            for work in &ordered {
                assert_eq!(
                    groups
                        .iter()
                        .find_map(|group| group.publication(&work.reference)),
                    Some(work)
                );
            }
        }
    }

    for category in [Category::Anime, Category::Tv] {
        let base = publication(1, "Generated Show 2", category);
        for title in [
            "Generated Show 2nd Season",
            "Generated Show Season 2",
            "Generated Show 2 сезон",
            "Generated Show сезон 2",
        ] {
            check_pair(
                "explicit season label retains its number",
                &base,
                &publication(2, title, category),
                1,
            );
        }
        let mut known_season = publication(1, "Generated Show 2nd Season", category);
        known_season.aliases = vec!["Generated Show".into()];
        let mut another_season = publication(2, "Generated Show 3rd Season", category);
        another_season.aliases = known_season.aliases.clone();
        check_pair(
            "a shared base alias cannot hide different seasons",
            &known_season,
            &another_season,
            2,
        );

        for title in [
            "Generated Show 2nd Season Part 1",
            "Generated Show 2nd Season Part 2",
            "Generated Show 2nd Season Cour 2",
            "Generated Show 2nd Season Часть 2",
            "Generated Show 2nd Season [Director's Cut]",
            "Generated Show 2nd Season: Режиссерская версия",
        ] {
            let mut qualified = publication(2, title, category);
            qualified.aliases = vec!["Generated Show Season 2".into()];
            check_pair(
                "a shared season alias cannot hide part/cour/director",
                &base,
                &qualified,
                2,
            );
        }
        let mut part_one = publication(1, "Generated Show 2nd Season Part 1", category);
        let mut part_two = publication(2, "Generated Show 2nd Season Part 2", category);
        part_one.aliases = vec!["Generated Show 2nd Season".into()];
        part_two.aliases = part_one.aliases.clone();
        check_pair(
            "Part 1 and Part 2 retain their difference",
            &part_one,
            &part_two,
            2,
        );
        part_one.title = "Generated Show Season 2 Part 2".into();
        check_pair(
            "the same explicit part can share a reported alias",
            &part_one,
            &part_two,
            1,
        );

        for (left, right) in [
            ("86", "86 2nd Season"),
            ("Generated Show 24", "Generated Show Season 2"),
            ("Generated Show 2", "Generated Show 20th Season"),
            ("Room 2", "Room"),
        ] {
            check_pair(
                "ordinary title digits remain significant",
                &publication(1, left, category),
                &publication(2, right, category),
                2,
            );
        }
    }
    let mut another_year = spirit.clone();
    another_year.year = Some(2021);
    check_pair(
        "same season spelling cannot hide a different year",
        &yummy,
        &another_year,
        2,
    );
    let mut another_catalogue = spirit.clone();
    another_catalogue
        .identifiers
        .insert("mal".into(), "999".into());
    check_pair(
        "same season spelling cannot hide a catalog conflict",
        &yummy,
        &another_catalogue,
        2,
    );
    check_pair(
        "season normalization stays out of movie titles",
        &publication(1, "Generated Show 2", Category::Movies),
        &publication(2, "Generated Show 2nd Season", Category::Movies),
        2,
    );

    let exact = publication(1, "Generated Show 2nd Season", Category::Anime);
    let partial = publication(
        2,
        "A Generated Show 2nd Season documentary",
        Category::Anime,
    );
    let mut association = WorkAssociation::default();
    association.update_search(partial);
    association.update_search(exact.clone());
    assert_eq!(
        association.search_groups("Generated Show 2nd Season")[0].anchor,
        exact.reference
    );
    late_qualifiers();
    println!(
        "PASS series_seasons actual_Yummy1246_AnimeSpirit12633=grouped arrival_orders=12 first_season_director_part2=separate exact_snapshots=preserved season_numbers_years_catalogs_parts_cours_director_title_digits=guarded raw_title_query_rank=preserved late_aliases_and_fresh_inspection=split_and_regroup"
    );
}

fn late_qualifiers() {
    for qualifier in ["Part 2", "Cour 2", "Director's Cut", "Режиссерская версия"]
    {
        let full = publication(2, "Generated Show 2", Category::Anime);
        let mut raw = full.clone();
        raw.needs_inspection = true;
        raw.variants.clear();
        let mut other = publication(1, "Generated Show 2nd Season", Category::Anime);
        other.needs_inspection = true;
        other.variants.clear();
        for ordered in [[&raw, &other], [&other, &raw]] {
            let mut association = WorkAssociation::default();
            for work in ordered {
                association.update_search(work.clone());
            }
            assert_eq!(association.groups().len(), 1);
            let alias = format!("Generated Show 2nd Season {qualifier}");
            let mut metadata = raw.clone();
            metadata.aliases = vec![alias.clone()];
            association.update_search(metadata);
            assert_eq!(
                association.groups().len(),
                2,
                "late {qualifier} alias was ignored"
            );
            association.update_search(raw.clone());
            assert_eq!(
                association.groups().len(),
                1,
                "withdrawn {qualifier} alias left stale split"
            );
            let mut inspected = full.clone();
            inspected.aliases = vec![alias];
            let exact = inspected.variants[0].reference.clone();
            association.update(inspected.clone());
            let groups = association.groups();
            assert_eq!(
                groups.len(),
                2,
                "fresh {qualifier} inspection did not split"
            );
            let saved = groups
                .iter()
                .find_map(|group| group.publication(&full.reference))
                .unwrap();
            assert_eq!(saved, &inspected);
            assert_eq!(saved.variants[0].reference, exact);
            association.update(full.clone());
            assert_eq!(
                association.groups().len(),
                1,
                "fresh corrected inspection left stale {qualifier}"
            );
        }
    }
}

fn publication(id: usize, title: &str, category: Category) -> Work {
    super::torrent(
        Source::RuTor,
        700 + id,
        title,
        category,
        &format!("{:040x}", 700 + id),
    )
}

fn check_pair(label: &str, left: &Work, right: &Work, expected: usize) {
    for publications in [[left, right], [right, left]] {
        let mut association = WorkAssociation::default();
        for work in publications {
            association.update_search(work.clone());
        }
        let groups = association.groups();
        assert_eq!(
            groups.len(),
            expected,
            "{label}: {} / {}",
            left.title,
            right.title
        );
        assert!(groups.iter().all(|group| group.is_confirmed()), "{label}");
        for work in publications {
            assert_eq!(
                groups
                    .iter()
                    .find_map(|group| group.publication(&work.reference)),
                Some(work),
                "{label}: exact publication changed"
            );
        }
    }
}
