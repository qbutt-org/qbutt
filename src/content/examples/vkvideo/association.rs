//! Production VK adapter, generated browser replies and isolated SQLite.

use std::path::Path;

use qbutt_content::browser::{BrowserChoice, BrowserHandle};
use qbutt_content::{
    Category, Library, PublicationRef, Source, Transport, VariantPreferences, VariantSelector,
    VkVideo, Work, WorkAssociation, WorkGroup,
};
use serde_json::json;
use tokio::net::windows::named_pipe::NamedPipeClient;
use url::Url;

use super::{FixtureResult, read, respond, video};

pub(super) async fn run(
    browser: &BrowserHandle,
    edge: &mut NamedPipeClient,
    directory: &Path,
) -> FixtureResult {
    let pending = tokio::spawn({
        let browser = browser.clone();
        async move {
            VkVideo
                .search(
                    "ready or not",
                    Category::Video,
                    &browser,
                    BrowserChoice::Edge,
                )
                .await
        }
    });
    let command = read(edge).await?;
    respond(
        edge,
        &command,
        json!({
            "query": "ready or not",
            "videos": [
                video("-910001_1001", "Я иду искать | Ready or Not (2019)"),
                video("-910001_1002", "Я иду искать 2 | Ready or Not 2: Here I Come (2026)"),
                video("-910001_1003", "Я иду искать | Ready or Not (2019) clip"),
                video("-910001_1004", "Я иду искать | Ready or Not: Extra Story (2019)"),
                video("-910001_1005", "Я иду искать | Ready or Not (2019) Review"),
                video("-910001_1006", "Я иду искать (2019) clip | Ready or Not (2019)"),
                video("-910001_1007", "Отрывок из фильма Я иду искать | Ready or Not (2019)"),
                video("-910001_1008", "Фрагмент фильма Я иду искать | Ready or Not (2019)"),
                video("-910001_1009", "Я иду искать excerpt | Ready or Not (2019)")
            ]
        }),
    )
    .await?;
    let uploads = pending.await??;
    assert_eq!(uploads.len(), 3);
    assert_eq!(uploads[0].aliases, ["Ready or Not (2019)"]);
    assert!(uploads.iter().all(|work| work.category == Category::Video));
    for work in &uploads {
        assert_eq!(work.variants[0].title.as_deref(), Some(work.title.as_str()));
    }
    let original = catalogue(
        &uploads[0],
        910001,
        "Я иду искать / Ready or Not (2019)",
        Category::Movies,
    );
    promotional_aliases(browser, edge, &original).await?;
    let sequel = catalogue(
        &uploads[1],
        910002,
        "Я иду искать 2 / Ready or Not 2: Here I Come (2026)",
        Category::Movies,
    );
    let mut lime = catalogue(
        &uploads[0],
        910005,
        "Ready or Not 2019 1080p DSNP WEB-DL DDP5 1 H 264-BLOOM",
        Category::Movies,
    );
    lime.reference = PublicationRef {
        source: Source::LimeTorrents,
        id: "Generated-Ready-or-Not-torrent-910005.html".into(),
    };
    lime.url = Url::parse(&format!(
        "https://www.limetorrents.fun/{}",
        lime.reference.id
    ))?;
    lime.variants[0].reference.publication = lime.reference.clone();
    let publications: Vec<_> = uploads
        .iter()
        .cloned()
        .chain([original.clone(), sequel, lime])
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
            assert_eq!(groups.len(), 3, "unknown subtitle stays separate");
            for group in &groups {
                assert!(group.is_confirmed());
                assert_eq!(
                    group.anchor,
                    ordered
                        .iter()
                        .find(|work| group.references.contains(&work.reference))
                        .unwrap()
                        .reference
                );
                if group.publications.len() > 1 {
                    assert_eq!(group.catalog_work().unwrap().category, Category::Movies);
                    assert!(!group.display_title().contains(" | "));
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

    let mut association = WorkAssociation::default();
    association.update(uploads[0].clone());
    association.update(original.clone());
    let group = association.groups().remove(0);
    assert_eq!(
        group.anchor, uploads[0].reference,
        "upload-first identity stays stable"
    );
    assert_eq!(group.catalog_work(), Some(&original));
    let selected = uploads[0].variants[0].reference.clone();
    let preferences = VariantPreferences {
        selected: Some(selected.clone()),
        ..Default::default()
    };
    let profile = directory.join("mixed-profile");
    std::fs::create_dir(&profile)?;
    let mut library = Library::open(&profile)?;
    library.set_favorite(&group, true)?;
    library.record_opened(&group, selected.clone(), preferences.clone())?;
    drop(library);
    let mut library = Library::open(&profile)?;
    let saved = library.get(&group.anchor)?.unwrap();
    assert!(saved.favorite);
    assert_eq!(saved.group, group);
    assert_eq!(saved.preferences, preferences);
    assert_eq!(saved.last_opened.unwrap().variant, selected);
    let mut unresolved = group.clone();
    unresolved.references.push(PublicationRef {
        source: Source::VkVideo,
        id: "-910001_1999".into(),
    });
    assert!(
        library.set_favorite(&unresolved, true).is_err(),
        "unresolved upload has no confirmed category relation"
    );
    let mut invalid = original.clone();
    invalid.reference = uploads[0].reference.clone();
    invalid.variants[0].reference.publication = invalid.reference.clone();
    assert!(
        library
            .set_favorite(&WorkGroup::single(invalid), true)
            .is_err(),
        "source must support its own publication category"
    );
    drop(library);
    std::fs::remove_file(profile.join("content-library.sqlite"))?;
    std::fs::remove_dir(profile)?;

    let tv = catalogue(&uploads[0], 910003, "Ready or Not (2019)", Category::Tv);
    for reverse in [false, true] {
        for offset in 0..3 {
            let mut association = WorkAssociation::default();
            let mut ordered = [uploads[0].clone(), original.clone(), tv.clone()];
            if reverse {
                ordered.reverse();
            }
            ordered.rotate_left(offset);
            for work in ordered {
                association.update(work);
            }
            assert_eq!(
                association.groups().len(),
                3,
                "Movie/TV ambiguity splits a prior mixed group"
            );
        }
    }
    for title in [
        "Ready or Not",
        "Ready or Not (2020)",
        "Ready or Not 2 (2019)",
    ] {
        let mut work = uploads[0].clone();
        work.title = title.into();
        work.aliases = vec![title.into()];
        let mut association = WorkAssociation::default();
        association.update(original.clone());
        association.update(work);
        assert_eq!(
            association.groups().len(),
            2,
            "year and sequel guard: {title}"
        );
    }
    let mut episode = uploads[0].clone();
    episode.title = "Generated Show S01E02 (2024)".into();
    episode.aliases = vec!["Generated Show (2024)".into()];
    let mut series = catalogue(
        &episode,
        910004,
        "Generated Show S01E01 (2024)",
        Category::Tv,
    );
    series.aliases = vec!["Generated Show (2024)".into()];
    let mut association = WorkAssociation::default();
    association.update(episode.clone());
    association.update(series.clone());
    assert_eq!(
        association.groups().len(),
        2,
        "shared base alias cannot replace an episode"
    );
    episode.title = series.title.clone();
    association.update(episode);
    assert_eq!(
        association.groups().len(),
        1,
        "same explicitly reported episode is compatible"
    );
    let mut season = uploads[0].clone();
    season.title = "Generated Show season 1 (2024)".into();
    season.aliases = vec!["Generated Show (2024)".into()];
    let whole_series = catalogue(&season, 910006, "Generated Show (2024)", Category::Tv);
    let mut association = WorkAssociation::default();
    association.update(season);
    association.update(whole_series);
    assert_eq!(
        association.groups().len(),
        2,
        "one known season cannot identify a whole series"
    );
    println!(
        "PASS vkvideo_association actual_titles=VK+RuTor+Lime2019,2026 arrival_orders=12 stable_upload_anchor=ok catalog_representative=Movies exact_upload_variant=preserved SQLite_favorite_history_reopen=ok clip_subtitle_year_sequel_season_episode_ambiguity=guarded unresolved_source_category=guarded"
    );
    Ok(())
}

async fn promotional_aliases(
    browser: &BrowserHandle,
    edge: &mut NamedPipeClient,
    original: &Work,
) -> FixtureResult {
    for (query, before, after) in [
        (
            "clip",
            "Я иду искать (2019) CLIPS | Ready or Not (2019)",
            "Я иду искать | Ready or Not (2019) clip",
        ),
        (
            "excerpt",
            "Я иду искать excerpts | Ready or Not (2019)",
            "Я иду искать | Ready or Not (2019) excerpt",
        ),
        (
            "отрывок",
            "Отрывок из фильма Я иду искать | Ready or Not (2019)",
            "Я иду искать | Ready or Not (2019) отрывок",
        ),
        (
            "фрагмент",
            "Фрагмент фильма Я иду искать | Ready or Not (2019)",
            "Я иду искать | Ready or Not (2019) фрагмент",
        ),
    ] {
        let pending = tokio::spawn({
            let browser = browser.clone();
            async move {
                VkVideo
                    .search(query, Category::Video, &browser, BrowserChoice::Edge)
                    .await
            }
        });
        let command = read(edge).await?;
        respond(
            edge,
            &command,
            json!({
                "query": query,
                "videos": [video("-910002_1001", before), video("-910002_1002", after)]
            }),
        )
        .await?;
        let uploads = pending.await??;
        assert_eq!(uploads.len(), 2, "explicit qualifier query retains uploads");
        for work in &uploads {
            assert!(
                work.aliases.is_empty(),
                "qualified upload must not acquire a film alias: {}",
                work.title
            );
            assert_eq!(work.variants[0].title.as_deref(), Some(work.title.as_str()));
        }
        for reverse in [false, true] {
            for offset in 0..3 {
                let mut association = WorkAssociation::default();
                let mut publications = [original.clone(), uploads[0].clone(), uploads[1].clone()];
                if reverse {
                    publications.reverse();
                }
                publications.rotate_left(offset);
                for work in publications {
                    association.update(work);
                }
                assert_eq!(
                    association.groups().len(),
                    3,
                    "qualifier before or after pipe cannot identify the film"
                );
            }
        }
    }
    println!(
        "PASS vkvideo_qualified_alias whole_publication=guarded before_after_pipe=ok explicit_qualifier_queries=preserved film_association=blocked arrival_orders=24"
    );
    Ok(())
}

fn catalogue(upload: &Work, id: usize, title: &str, category: Category) -> Work {
    let mut work = upload.clone();
    work.reference = PublicationRef {
        source: Source::RuTor,
        id: id.to_string(),
    };
    work.title = title.into();
    work.category = category;
    work.aliases.clear();
    work.url = Url::parse(&format!("https://rutor.info/torrent/{id}")).unwrap();
    let hash = format!("{id:040x}");
    let variant = &mut work.variants[0];
    variant.reference.publication = work.reference.clone();
    variant.reference.selector = VariantSelector::TorrentHash {
        info_hash: hash.clone(),
    };
    variant.transport = Transport::Torrent;
    variant.title = Some(title.into());
    variant.info_hash = Some(hash);
    work
}
