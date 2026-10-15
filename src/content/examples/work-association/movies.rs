//! Public RuTor and LimeTorrents labels fetched on 2026-10-15. Publication
//! references and payload identities are generated; no video is acquired.
//! https://rutor.info/search/0/1/100/0/ready%20or%20not/
//! https://www.limetorrents.fun/search/all/ready%20or%20not/date/1/

use qbutt_content::{Category, Source, WorkAssociation};

pub(super) fn run() {
    let originals = [
        "Я иду искать / Ready or Not (2019) UHD WEB-DL 2160p | 4K | SDR | D, P2, A",
        "Я иду искать / Ready Or Not (2019) BDRip-HEVC 1080p | D, A",
        "Я иду искать / Ready Or Not (2019) WEB-DL-HEVC 2160p | 4K | HDR | Dolby Vision Profile 8 | D, A, L1 | iTunes",
        "Я иду искать / Ready or Not (2019) WEB-DLRip-AVC | КПК | D | Open Matte",
        "Я иду искать / Ready or Not (2019) HDRip от Scarabey | iTunes",
        "Я иду искать / Ready or Not (2019) BDRemux 1080p от селезень | iTunes",
        "Ready or Not 2019 1080p DSNP WEB-DL DDP5 1 H 264-BLOOM",
    ];
    let sequels = [
        "Я иду искать 2 / Ready or Not 2: Here I Come (2026) HDRip-AVC | КПК | D | MovieDalen",
        "Я иду искать 2 / Ready or Not 2: Here I Come (2026) WEBRip-AVC от ExKinoRay | D",
        "Ready Or Not 2 Here I Come 2026 1080p DUAL WEB-DL x264 EAC3 5 1 Atmos - HdT",
        "Ready or Not 2 Here I Come 2026 PL 1080p BDRp XviD AC3-MAXX",
        "Ready or Not 2 Here I Come [2026] Dual YG",
        "Ready or Not 2 Here I Come (2026) [Samara Weaving] BluRay H264 DD 5 1 nickarad",
        "Ready or Not 2 - Here I Come (2026) (1080p BluRay x265 SDR DDP Atmos 7 1 En",
    ];
    let unnumbered = [
        "Ready Or Not Here I Come (2026)",
        "Ready or Not Here I Come 2026 m1080p BluRay x264-GeneMige",
    ];
    let publications: Vec<_> = originals
        .into_iter()
        .chain(sequels)
        .chain(unnumbered)
        .enumerate()
        .map(|(index, title)| {
            let source = if title.starts_with("Я иду искать") {
                Source::RuTor
            } else {
                Source::LimeTorrents
            };
            let mut work = super::torrent(
                source,
                500 + index,
                title,
                Category::Movies,
                &format!("{:040x}", 500 + index),
            );
            work.year = None;
            if source == Source::LimeTorrents {
                work.reference.id = format!("/Generated-Movie-torrent-{}.html", 500 + index);
                work.variants[0].reference.publication = work.reference.clone();
            }
            work.variants[0].title = Some(title.into());
            work
        })
        .collect();
    for reverse in [false, true] {
        for offset in 0..publications.len() {
            let mut ordered = publications.clone();
            if reverse {
                ordered.reverse();
            }
            ordered.rotate_left(offset);
            let mut association = WorkAssociation::default();
            for work in &ordered {
                association.update(work.clone());
            }
            let groups = association.groups();
            assert_eq!(
                groups.len(),
                3,
                "original, sequel and unnumbered name stay separate"
            );
            for group in &groups {
                assert!(group.is_confirmed());
                let title = group.display_title();
                if title.ends_with("(2019)") {
                    assert_eq!(group.variants(None).len(), originals.len());
                } else if group
                    .publication(&publications[originals.len()].reference)
                    .is_some()
                {
                    assert!(title.ends_with("(2026)"));
                    assert_eq!(group.variants(None).len(), sequels.len());
                } else {
                    assert!(title.ends_with("(2026)"));
                    assert_eq!(group.variants(None).len(), unnumbered.len());
                }
            }
            for work in &publications {
                assert_eq!(
                    groups
                        .iter()
                        .find_map(|group| group.publication(&work.reference)),
                    Some(work)
                );
            }
        }
    }
    println!(
        "movie_release_corpus=RuTor+LimeTorrents original2019=7 sequel2026=7 unnumbered2026=2 arrival_orders=32 exact_variants=preserved"
    );
}
