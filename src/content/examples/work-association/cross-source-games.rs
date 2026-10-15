//! Public search labels fetched on 2026-10-15 with plain HTTP. References and
//! independent variants below are generated; no game payload is acquired.
//! https://fitgirl-repacks.site/?s=ready+or+not
//! https://dodi-repacks.site/?s=ready+or+not
//! https://www.limetorrents.fun/search/all/ready%20or%20not/date/1/
//! Online-Fix's parser reports its game title without the source's "по сети" label.

use std::collections::HashSet;

use qbutt_content::{Category, Source, VariantSelector, Work, WorkAssociation};
use url::Url;

const FITGIRL: &str =
    "Ready or Not: Digital Deluxe Edition, v112610 (v1.4.1) + v66884 + 6 DLCs/Bonuses";
const DODI: &str = "1829- Ready or Not: LSPD Bundle (v94572 + All DLCs + Bonus Content + Online Multiplayer + MULTi12) (From 23.1 GB) [DODI Repack]";
const DODI_ALIAS: &str = "Ready or Not: LSPD Bundle (v94572 + All DLCs + Bonus Content + Online Multiplayer + MULTi12) (From 23.1 GB) [DODI Repack]";
const LIME: &str = "Ready Or Not v133804 REPACK-KaOs";
const LIME_FITGIRL: &str =
    "Ready or Not: Digital Deluxe Edition, v112610 (v1 4 1) v66884 6 DLCs Bonuses";

pub(super) fn run(rutor: &[Work]) {
    let mut publications = rutor.to_vec();
    publications.extend([
        publication(100, Source::FitGirl, FITGIRL),
        publication(101, Source::Dodi, DODI),
        publication(102, Source::ByRut, "Ready or Not"),
        publication(103, Source::OnlineFix, "Ready or Not"),
        publication(104, Source::LimeTorrents, LIME),
        publication(105, Source::LimeTorrents, LIME_FITGIRL),
        publication(
            106,
            Source::FreeTP,
            "Ready Or Not играть по сети и интернету Онлайн",
        ),
        publication(107, Source::ByRut, "Ready or Not по сети"),
    ]);

    let mut orderings = 0;
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
            assert_eq!(groups.len(), 1, "cross-source game releases");
            let group = &groups[0];
            assert!(
                group
                    .display_title()
                    .eq_ignore_ascii_case("Ready or Not (2023)"),
                "unexpected canonical title: {}",
                group.display_title()
            );
            assert!(group.is_confirmed());
            assert_eq!(group.variants(None).len(), publications.len());
            assert_eq!(group.anchor, ordered[0].reference);
            let sources: HashSet<_> = group
                .publications
                .iter()
                .map(|work| work.reference.source)
                .collect();
            assert_eq!(sources.len(), 7);
            for publication in &publications {
                assert_eq!(group.publication(&publication.reference), Some(publication));
            }
            orderings += 1;
        }
    }
    assert_eq!(orderings, 26);

    for title in [
        "Ready or Not 2 v133804 REPACK-KaOs",
        "Ready or Not: New Story – Deluxe Edition, v112610",
        "Ready or Not - The Deadline is coming!",
    ] {
        let mut association = WorkAssociation::default();
        for publication in &publications {
            association.update(publication.clone());
        }
        association.update(publication(200, Source::LimeTorrents, title));
        assert_eq!(association.groups().len(), 2, "unknown derivative: {title}");
    }

    let mut association = WorkAssociation::default();
    association.update(publication(201, Source::Dodi, DODI));
    assert_eq!(association.groups()[0].display_title(), "Ready or Not");
    println!(
        "cross_source_game_corpus=RuTor,FitGirl,DODI,ByRut,OnlineFix,LimeTorrents,FreeTP publications=13 arrival_orders=26 raw_labels_and_exact_variants=preserved unknown_subtitles=split"
    );
}

fn publication(id: usize, source: Source, title: &str) -> Work {
    let mut work = super::torrent(source, id, title, Category::Games, &format!("{id:040x}"));
    work.year = None;
    work.reference.id = match source {
        Source::Dodi => format!("{id}--ready-or-not"),
        Source::FitGirl => format!("ready-or-not-{id}"),
        Source::ByRut => format!("{id}-ready-or-not"),
        Source::OnlineFix => format!("games/shooter/{id}-ready-or-not.html"),
        Source::FreeTP => id.to_string(),
        Source::LimeTorrents => format!("/Ready-Or-Not-torrent-{id}.html"),
        _ => unreachable!(),
    };
    work.url = Url::parse(&format!("https://example.org/publication/{id}")).unwrap();
    work.variants[0].reference.publication = work.reference.clone();
    work.variants[0].title = Some(title.into());
    if source == Source::Dodi {
        work.aliases = vec![DODI_ALIAS.into()];
        work.variants[0].reference.selector = VariantSelector::DodiTorrent {
            id: "aaaaaaaabbbb".into(),
        };
    }
    if source == Source::FreeTP {
        work.aliases = vec!["Ready Or Not".into()];
    } else if source == Source::ByRut && title.ends_with(" по сети") {
        work.aliases = vec!["Ready or Not".into()];
    }
    work
}
