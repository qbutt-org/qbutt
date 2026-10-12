//! Bounded live catalog acceptance. Resolves descriptors without downloading
//! videos, starting a browser/player or opening an application profile.

use std::collections::HashSet;

use qbutt_content::{Error, PublicationRef, ResolvedContent, Source, VariantSelector, YummyAnime};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = YummyAnime::new()?;
    let works = catalog.search("Frieren").await?;
    let publication = works
        .iter()
        .find(|work| work.reference.id == "10661")
        .ok_or("the live catalog no longer contains the selected publication")?;
    assert!(publication.needs_inspection && publication.variants.is_empty());
    let work = catalog.inspect(&publication.reference).await?;
    assert_eq!(work.reference, publication.reference);
    assert!(!work.needs_inspection);
    assert!(
        work.aliases
            .iter()
            .any(|alias| alias == "Sousou no Frieren")
    );
    assert_eq!(
        work.variants
            .iter()
            .map(|variant| &variant.reference)
            .collect::<HashSet<_>>()
            .len(),
        work.variants.len()
    );
    let encoded = serde_json::to_string(&work)?;
    for private_field in [
        "iframe_url",
        "mpegFullHdUrl",
        "qualities",
        "Cookie",
        "Authorization",
    ] {
        assert!(!encoded.contains(private_field));
    }

    for player in ["Aksor", "CVH"] {
        let variant = work
            .variants
            .iter()
            .find(|variant| {
                variant.player.as_deref() == Some(player)
                    && variant.episode.as_deref() == Some("1")
                    && variant.translation.as_deref() == Some("AniLibria")
            })
            .ok_or("the selected live video is no longer available")?;
        match catalog.resolve(&variant.reference).await? {
            ResolvedContent::DirectVideo {
                reference,
                url,
                quality,
                ..
            } => {
                assert_eq!(reference, variant.reference);
                assert_eq!(url.scheme(), "https");
                assert_eq!(quality.height(), 1080);
                println!("player={player} exact_episode_voice=ok direct_descriptor=1080p");
            }
            _ => return Err("the exact direct video resolved as a different transport".into()),
        }
    }

    let kodik = work
        .variants
        .iter()
        .find(|variant| {
            variant.player.as_deref() == Some("Kodik")
                && variant.episode.as_deref() == Some("1")
                && variant.translation.as_deref() == Some("AniLibria")
        })
        .ok_or("the exact live Kodik video is no longer available")?;
    match catalog.resolve(&kodik.reference).await? {
        ResolvedContent::DirectVideo {
            reference,
            url,
            quality,
            ..
        } => {
            assert_eq!(reference, kodik.reference);
            assert_eq!(url.scheme(), "https");
            assert_eq!(quality.height(), 720);
            println!("player=Kodik exact_episode_voice=ok direct_descriptor=720p");
        }
        _ => return Err("the exact Kodik video resolved as a different transport".into()),
    }

    let invalid = PublicationRef {
        source: Source::YummyAnime,
        id: "010661".into(),
    };
    assert!(matches!(
        catalog.inspect(&invalid).await,
        Err(Error::InvalidReference)
    ));
    let movie = catalog
        .inspect(&PublicationRef {
            source: Source::YummyAnime,
            id: "1463".into(),
        })
        .await?;
    let shiza = movie
        .variants
        .iter()
        .find(|variant| {
            matches!(
                variant.reference.selector,
                VariantSelector::YummyAnimeVideo { id: 1310557, .. }
            )
        })
        .ok_or("the selected SHIZA movie is no longer available")?;
    assert_eq!(shiza.translation.as_deref(), Some("SHIZA Project"));
    assert_eq!(shiza.episode.as_deref(), Some("1"));
    let resolved = catalog.resolve(&shiza.reference).await?;
    assert!(
        matches!(resolved, ResolvedContent::DirectVideo { reference, .. } if reference == shiza.reference)
    );

    let subtitles = movie
        .variants
        .iter()
        .find(|variant| {
            matches!(
                variant.reference.selector,
                VariantSelector::YummyAnimeVideo { id: 1315397, .. }
            )
        })
        .ok_or("the selected subtitle movie is no longer available")?;
    assert_eq!(subtitles.translation.as_deref(), Some("Субтитры SubVost"));
    assert_eq!(subtitles.episode.as_deref(), Some("1"));
    assert_eq!(subtitles.languages, ["ja", "ru"]);
    let resolved = catalog.resolve(&subtitles.reference).await?;
    assert!(
        matches!(resolved, ResolvedContent::DirectVideo { reference, .. } if reference == subtitles.reference)
    );

    for field in 0..5 {
        let mut changed = kodik.reference.clone();
        let VariantSelector::YummyAnimeVideo {
            id,
            episode,
            dubbing,
            player,
            player_id,
        } = &mut changed.selector
        else {
            return Err("unexpected selector".into());
        };
        match field {
            0 => *id = u64::MAX,
            1 => *episode = "nonexistent".into(),
            2 => *dubbing = "Changed voice".into(),
            3 => *player = "Changed player".into(),
            _ => *player_id = u64::MAX,
        }
        assert!(matches!(
            catalog.resolve(&changed).await,
            Err(Error::VariantUnavailable)
        ));
    }
    println!(
        "search={} inspected={} variants={} cvh_display_alias=ok subtitle_movie=ok kodik_direct=ok changed_id_episode_voice_player=rejected",
        works.len(),
        work.reference.id,
        work.variants.len()
    );
    Ok(())
}
