//! Cross-source title corpus and generated SQLite reopen scenario; no live profile.

#[path = "work-association/cross-source-games.rs"]
mod cross_source_games;
#[path = "work-association/movies.rs"]
mod movies;
#[path = "work-association/product-releases.rs"]
mod product_releases;
#[path = "work-association/public-titles.rs"]
mod public_titles;

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::{
    Category, Library, PublicationRef, Source, Transport, Variant, VariantPreferences, VariantRef,
    VariantSelector, Work, WorkAssociation,
};
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    public_titles::run()?;
    product_releases::run()?;
    movies::run();

    let mut association = WorkAssociation::default();
    let categories = [Category::Movies, Category::Games, Category::Books];

    for (index, category) in categories.into_iter().enumerate() {
        let hash = format!("{index:040x}");
        let title = format!("Generated work {index}");
        let mut publication = torrent(Source::RuTor, 100 + index, &title, category, &hash);
        if category == Category::Movies {
            publication.title = "Созданный фильм".into();
            publication.aliases = vec![title.clone(), "Sozdannyy film".into(), "---".into()];
        }
        association.update(publication);
        association.update(torrent(
            Source::RuTracker,
            200 + index,
            &title.to_uppercase(),
            category,
            &hash,
        ));
    }
    let groups = association.groups();
    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|group| group.publications.len() == 2));

    let movie = &groups[0];
    let preferred = movie.publications[1].variants[0].reference.clone();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-association-{nonce}"));
    std::fs::create_dir(&profile)?;
    let result = (|| -> Result<()> {
        let mut library = Library::open(&profile)?;
        library.save_preferences(
            movie,
            VariantPreferences {
                selected: Some(preferred.clone()),
                ..Default::default()
            },
        )?;
        drop(library);

        let library = Library::open(&profile)?;
        let saved = library.get(&movie.anchor)?.ok_or("missing grouped entry")?;
        assert_eq!(saved.group.references, movie.references);
        assert_eq!(saved.group.publications, movie.publications);
        assert_eq!(saved.preferences.selected, Some(preferred));
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    result?;

    let mut romaji = movie.publications[1].clone();
    romaji.title = "SOZDANNYY-FILM".into();
    association.update(romaji);
    assert_eq!(association.groups().len(), 3);

    let mut alias_only = movie.publications[1].clone();
    alias_only.title = "Generated alternative title".into();
    alias_only.aliases = vec!["Sozdannyy film".into()];
    association.update(alias_only);
    assert_eq!(association.groups().len(), 3);

    let mut remake = movie.publications[1].clone();
    remake.year = Some(2024);
    association.update(remake);
    assert_eq!(association.groups().len(), 4);
    let mut restored = movie.publications[1].clone();
    restored.year = Some(2020);
    association.update(restored);
    assert_eq!(association.groups().len(), 3);

    let mut same_payload_different_work = movie.publications[1].clone();
    same_payload_different_work.title = "Generated sequel".into();
    association.update(same_payload_different_work);
    assert_eq!(association.groups().len(), 4);

    let mut same_alias_different_payload = movie.publications[1].clone();
    same_alias_different_payload.variants[0].info_hash = Some("f".repeat(40));
    same_alias_different_payload.variants[0].reference.selector = VariantSelector::TorrentHash {
        info_hash: "f".repeat(40),
    };
    association.update(same_alias_different_payload);
    assert_eq!(association.groups().len(), 3);

    let mut empty_alias = movie.publications[1].clone();
    empty_alias.title = "...".into();
    empty_alias.aliases = vec!["---".into()];
    association.update(empty_alias);
    assert_eq!(association.groups().len(), 4);

    let mut different_category = movie.publications[1].clone();
    different_category.category = Category::Anime;
    association.update(different_category);
    assert_eq!(association.groups().len(), 4);

    association.clear();
    let mut season_one = anime(Source::AniLiberty, 301, "Example season 1", "991");
    let season_two = anime(Source::YummyAnime, 302, "Example season 2", "991");
    association.update(season_one.clone());
    association.update(season_two);
    assert_eq!(association.groups().len(), 2);
    season_one.title = "Example season 2".into();
    association.update(season_one);
    assert_eq!(association.groups().len(), 1);

    let mut ranked = movie.variants(Some("ru"));
    ranked[0].languages = vec!["en".into()];
    ranked[1].languages = vec!["ru".into()];
    ranked[0].quality = Some("1080p".into());
    ranked[1].quality = Some("720p".into());
    ranked[0].seeders = Some(900);
    ranked[1].seeders = Some(1);
    let ranked = qbutt_content::rank_variants(ranked, Some("ru"));
    assert_eq!(ranked[0].languages[0], "ru");

    println!("categories=movie,game,book grouped=3 exact_refs=preserved reopen=ok");
    println!("year_conflict=split title_conflict=split season_conflict=split reversible=ok");
    println!(
        "aliases=grouped romaji_punctuation=grouped independent_payloads=grouped empty_alias=split category_conflict=split"
    );
    println!("language_before_quality_and_seeders=ok");
    Ok(())
}

fn torrent(source: Source, id: usize, title: &str, category: Category, hash: &str) -> Work {
    let reference = PublicationRef {
        source,
        id: id.to_string(),
    };
    let url = match source {
        Source::RuTracker => format!("https://rutracker.org/forum/viewtopic.php?t={id}"),
        _ => format!("https://rutor.info/torrent/{id}"),
    };
    Work {
        reference: reference.clone(),
        title: title.into(),
        category,
        url: Url::parse(&url).unwrap(),
        cover: None,
        description: None,
        year: Some(2020),
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants: vec![Variant {
            reference: VariantRef {
                publication: reference,
                selector: VariantSelector::TorrentHash {
                    info_hash: hash.into(),
                },
            },
            title: None,
            transport: Transport::Torrent,
            episode: None,
            quality: None,
            translation: None,
            languages: Vec::new(),
            player: None,
            size: None,
            seeders: None,
            info_hash: Some(hash.into()),
            reported_video_ads: false,
        }],
        needs_inspection: false,
    }
}

fn anime(source: Source, id: usize, title: &str, mal: &str) -> Work {
    Work {
        reference: PublicationRef {
            source,
            id: id.to_string(),
        },
        title: title.into(),
        category: Category::Anime,
        url: Url::parse(&format!("https://example.org/{id}")).unwrap(),
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::from([("mal".into(), mal.into())]),
        variants: Vec::new(),
        needs_inspection: true,
    }
}
