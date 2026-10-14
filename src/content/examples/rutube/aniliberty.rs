use std::sync::atomic::Ordering;

use super::{
    CheckResult, Error, PublicationRef, ResolvedContent, Source, VariantRef, VariantSelector,
    VideoQuality, aniliberty, fixture, http, rutube,
};

pub async fn verify(fixture: &fixture::Fixture) -> CheckResult<()> {
    let source = aniliberty::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
        rutube::fixture(
            http::fixture(fixture.client.clone(), fixture.origin.clone()),
            fixture.origin.clone(),
        ),
    );
    let publication = PublicationRef {
        source: Source::AniLiberty,
        id: "1".into(),
    };
    let work = source.inspect(&publication).await?;
    assert_eq!(work.variants.len(), 2);
    let selected = &work.variants[1].reference;
    assert_eq!(
        selected.selector,
        VariantSelector::Rutube {
            episode_id: "generated-episode-2".into(),
            rutube_id: fixture::ID.into(),
        }
    );
    let ResolvedContent::DirectVideo {
        reference,
        title,
        url,
        quality,
    } = source.resolve(selected).await?
    else {
        panic!("AniLiberty Rutube did not resolve to direct media");
    };
    assert_eq!(&reference, selected);
    assert_eq!(title, work.title);
    assert_eq!(url, fixture.origin.join("/1080.m3u8")?);
    assert_eq!(quality, VideoQuality::P1080);
    persist_work(&serde_json::to_string(&work)?)?;

    for fault in [
        "episode-changed",
        "episode-missing",
        "rutube-changed",
        "rutube-missing",
    ] {
        fixture.case(fault);
        let before = fixture.hits.load(Ordering::SeqCst);
        assert!(matches!(
            source.resolve(selected).await,
            Err(Error::VariantUnavailable)
        ));
        assert_eq!(fixture.hits.load(Ordering::SeqCst), before + 1);
    }
    fixture.case("release-changed");
    assert!(matches!(
        source.resolve(selected).await,
        Err(Error::InvalidResponse("publication identity changed"))
    ));
    fixture.case("rutube-invalid");
    assert!(source.inspect(&publication).await.is_err());
    for fault in [
        "paid",
        "identity",
        "effective",
        "drm",
        "child-private-redirect",
    ] {
        fixture.case(fault);
        assert!(source.resolve(selected).await.is_err(), "accepted {fault}");
    }

    fixture.case("normal");
    let mut invalid = selected.clone();
    invalid.selector = VariantSelector::Rutube {
        episode_id: "generated-episode-2".into(),
        rutube_id: "invalid".into(),
    };
    let before = fixture.hits.load(Ordering::SeqCst);
    assert!(matches!(
        source.resolve(&invalid).await,
        Err(Error::InvalidReference)
    ));
    assert_eq!(fixture.hits.load(Ordering::SeqCst), before);
    let mut incomplete = serde_json::to_value(selected)?;
    incomplete["selector"]["rutube"]
        .as_object_mut()
        .ok_or("unexpected selector encoding")?
        .remove("rutube_id");
    assert!(serde_json::from_value::<VariantRef>(incomplete).is_err());
    println!(
        "PASS aniliberty_rutube generated_https=ok nonfirst_episode=exact original_reference=preserved fresh_episode_video_change=rejected public_identity=restart_and_share access=guarded media_payload=none"
    );
    Ok(())
}

fn persist_work(encoded: &str) -> CheckResult<()> {
    use qbutt_content::{Library, SharedWork, VariantPreferences, Work, WorkGroup};

    assert!(!encoded.contains("m3u8"));
    assert!(!encoded.contains("transient-signature"));
    let mut work: Work = serde_json::from_str(encoded)?;
    work.url = url::Url::parse("https://anilibria.top/anime/releases/release/generated-release")?;
    let selected = work.variants[1].reference.clone();
    let share = SharedWork::new(vec![work.reference.clone()], Some(selected.clone()))?;
    assert_eq!(
        SharedWork::from_url(&share.to_url()?)?.preferred(),
        Some(&selected)
    );
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let profile = std::env::temp_dir().join(format!(
        "qbutt-aniliberty-rutube-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&profile)?;
    let mut library = Library::open(&profile)?;
    library.record_opened(
        &WorkGroup::single(work),
        selected.clone(),
        VariantPreferences {
            selected: Some(selected),
            ..Default::default()
        },
    )?;
    drop(library);
    let status = std::process::Command::new(std::env::current_exe()?)
        .arg("--reopen-aniliberty")
        .arg(&profile)
        .status()?;
    assert!(
        status.success(),
        "AniLiberty Rutube identity was not restored"
    );
    std::fs::remove_dir_all(profile)?;
    Ok(())
}

pub fn verify_library(profile: &std::path::Path) -> CheckResult<()> {
    use qbutt_content::{Library, LibraryView, Source, VariantSelector};

    let library = Library::open(profile)?;
    let entries = library.entries(LibraryView::History)?;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    let selected = entry
        .preferences
        .selected
        .as_ref()
        .ok_or("missing exact selection")?;
    assert_eq!(selected.publication.source, Source::AniLiberty);
    assert_eq!(selected.publication.id, "1");
    assert_eq!(
        selected.selector,
        VariantSelector::Rutube {
            episode_id: "generated-episode-2".into(),
            rutube_id: fixture::ID.into(),
        }
    );
    assert_eq!(&entry.last_opened.as_ref().unwrap().variant, selected);
    Ok(())
}
