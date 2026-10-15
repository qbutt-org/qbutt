//! Actual public titles with generated exact references; no network or payload.

use qbutt_content::{Category, Work, WorkAssociation, WorkGroup};

pub(super) fn run() {
    let publications = [
        publication(
            501,
            "Ready or Not - The Deadline is coming!",
            Category::Games,
        ),
        publication(
            502,
            "Я иду искать 2 / Ready or Not 2: Here I Come (2026)",
            Category::Movies,
        ),
        publication(
            503,
            "Ready or Not [v 117216 + 6 DLC] (2023) PC | Portable от Pioneer",
            Category::Games,
        ),
        publication(
            504,
            "Я иду искать / Ready or Not (2019) UHD WEB-DL 2160p | 4K | SDR | D, P2, A",
            Category::Movies,
        ),
        publication(
            505,
            "Ready or Not: Digital Deluxe Edition [v 112610] (2023) PC",
            Category::Games,
        ),
        publication(
            506,
            "Ready or Not Boiling Point [v 112610 + DLCs] (2023) PC | RePack от селезень",
            Category::Games,
        ),
    ];
    let expected = [504, 503, 502, 501];
    for reverse in [false, true] {
        for offset in 0..publications.len() {
            let mut ordered = publications.to_vec();
            if reverse {
                ordered.reverse();
            }
            ordered.rotate_left(offset);
            let mut association = WorkAssociation::default();
            for (index, publication) in ordered.iter().enumerate() {
                association.update(publication.clone());
                let groups = association.search_groups("ready or not");
                let games: Vec<_> = groups
                    .iter()
                    .filter(|group| group.publications[0].category == Category::Games)
                    .collect();
                if ordered[..=index]
                    .iter()
                    .any(|work| work.reference.id == "503" || work.reference.id == "505")
                {
                    assert_ne!(games[0].publications[0].reference.id, "501");
                }
            }
            let groups = association.search_groups("READY-or NOT!");
            assert_eq!(identities(&groups), expected);
            for publication in &publications {
                let group = groups
                    .iter()
                    .find(|group| group.references.contains(&publication.reference))
                    .unwrap();
                assert_eq!(group.publication(&publication.reference), Some(publication));
            }
            let anchors: Vec<_> = groups.iter().map(|group| group.anchor.clone()).collect();
            association.update(publications[2].clone());
            assert_eq!(
                identities(&association.search_groups("ready or not")),
                expected
            );
            assert_eq!(
                association
                    .search_groups("ready or not")
                    .iter()
                    .map(|group| group.anchor.clone())
                    .collect::<Vec<_>>(),
                anchors
            );
            let partial = association.search_groups("ready");
            assert_eq!(identities(&partial), expected);
            assert_eq!(identities(&association.search_groups("---")), expected);
            assert_eq!(
                identities(&association.search_groups("ready or not boiling point"))[0],
                503,
                "confirmed original title alias retains exact relevance"
            );
        }
    }

    let mut association = WorkAssociation::default();
    let mut alias = publication(510, "Созданное произведение", Category::Books);
    alias.aliases = vec!["Generated Work".into()];
    for work in [
        super::anime(
            qbutt_content::Source::AniLiberty,
            511,
            "Generated Work season 2",
            "511",
        ),
        publication(512, "Generated Work 2", Category::Games),
        alias.clone(),
    ] {
        association.update(work);
    }
    assert_eq!(
        identities(&association.search_groups("generated-work")),
        [510, 512, 511]
    );
    alias.aliases.clear();
    association.update(alias);
    assert_eq!(
        identities(&association.search_groups("generated-work")),
        [512, 511, 510]
    );

    let mut association = WorkAssociation::default();
    for (id, category, year) in [
        (523, Category::Games, 2024),
        (522, Category::Games, 2023),
        (521, Category::Movies, 2023),
    ] {
        let mut work = publication(id, "Generated Work", category);
        work.year = Some(year);
        association.update(work);
    }
    assert_eq!(
        identities(&association.search_groups("generated work")),
        [521, 522, 523]
    );
    println!(
        "search_order=exact_product_and_alias_before_partial sequel_episode=lower deterministic_ties=title_category_ref arrival_orders=12 incremental_and_repeated_updates=stable exact_publications_and_anchors=preserved"
    );
}

fn identities(groups: &[WorkGroup]) -> Vec<usize> {
    groups
        .iter()
        .map(|group| {
            group
                .references
                .iter()
                .map(|reference| reference.id.parse().unwrap())
                .min()
                .unwrap()
        })
        .collect()
}

fn publication(id: usize, title: &str, category: Category) -> Work {
    let mut work = super::torrent(
        qbutt_content::Source::RuTor,
        id,
        title,
        category,
        &format!("{id:040x}"),
    );
    work.year = None;
    work
}
