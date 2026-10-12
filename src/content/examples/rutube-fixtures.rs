//! Generated HTTPS integration through the production Rutube operations.
//! The private TLS client belongs only to this executable.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
pub use qbutt_content::{Error, Result};

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod rutube {
    include!("../src/rutube.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> Rutube {
        Rutube { http, origin }
    }
}

#[path = "rutube/fixture.rs"]
mod fixture;

use std::sync::atomic::Ordering;
use std::time::Duration;

use fixture::Fixture;

type CheckResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> CheckResult<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
        verify_library(std::path::Path::new(
            &args.next().ok_or("missing fixture profile")?,
        ))?;
        return Ok(());
    }
    let fixture = Fixture::start().await?;
    let source = rutube::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let rows = source.search("generated & legal", Category::Movies).await?;
    assert_eq!(rows.len(), 1);
    let found = &rows[0];
    assert!(found.variants[0].quality.is_none());
    assert_eq!(found.reference.id, fixture::ID);
    assert_eq!(found.category, Category::Movies);
    let work = source.inspect(&found.reference).await?;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.variants[0].reference, found.variants[0].reference);
    assert_eq!(work.variants[0].quality.as_deref(), Some("1080p"));
    assert!(!work.needs_inspection);
    let ResolvedContent::DirectVideo {
        reference,
        quality,
        url,
        ..
    } = source.resolve(&found.variants[0].reference).await?
    else {
        panic!("expected a direct video");
    };
    assert_eq!(reference, found.variants[0].reference);
    assert_eq!(quality, VideoQuality::P1080);
    assert_eq!(url, fixture.origin.join("/1080.m3u8")?);
    let encoded = serde_json::to_string(&work)?;
    assert!(!encoded.contains("transient-signature"));
    assert!(!encoded.contains("m3u8"));
    assert!(work.url.query().is_none());
    persist_work(&encoded)?;
    for category in [Category::Tv, Category::Anime] {
        let rows = source.search("generated legal", category).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].category, category);
    }
    for fault in [
        "paid",
        "locked",
        "hidden",
        "deleted",
        "identity",
        "access",
        "effective",
        "drm",
        "limited",
        "no-video",
        "no-stream",
        "bad-playlist",
        "no-resolution",
        "unsupported-quality",
        "missing-uri",
        "private-stream",
        "private-child",
        "ambiguous-quality",
        "ambiguous-maximum",
        "child-master",
        "child-challenge",
        "child-private-redirect",
        "external-audio",
        "truncated-child",
    ] {
        fixture.case(fault);
        assert!(
            source.inspect(&found.reference).await.is_err(),
            "inspect accepted {fault}"
        );
        assert!(
            source.resolve(&found.variants[0].reference).await.is_err(),
            "resolve accepted {fault}"
        );
    }
    for case in ["mirror", "ambiguous-lower", "child-redirect"] {
        fixture.case(case);
        let ResolvedContent::DirectVideo { quality, url, .. } =
            source.resolve(&found.variants[0].reference).await?
        else {
            panic!("expected a concrete media playlist");
        };
        assert_eq!(quality, VideoQuality::P1080);
        let path = if case == "child-redirect" {
            "/final1080.m3u8"
        } else {
            "/1080.m3u8"
        };
        assert_eq!(url, fixture.origin.join(path)?);
    }
    fixture.case("normal");
    assert_eq!(source.search("multiline", Category::Movies).await?.len(), 1);
    for id in ["f".repeat(32), "e".repeat(32)] {
        assert!(matches!(
            source
                .inspect(&PublicationRef {
                    source: Source::Rutube,
                    id
                })
                .await,
            Err(Error::VariantUnavailable)
        ));
    }
    for query in ["empty", "unwanted", "unknown-category"] {
        assert!(source.search(query, Category::Movies).await?.is_empty());
    }
    for query in [
        "challenge",
        "duplicate",
        "wrong-url",
        "missing-policy",
        "page",
        "too-many",
        "redirect-private",
        "redirect-origin",
    ] {
        assert!(
            source.search(query, Category::Movies).await.is_err(),
            "search accepted {query}"
        );
    }
    assert!(matches!(
        source.search("huge", Category::Movies).await,
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        source.search("delay", Category::Movies).await,
        Err(Error::Timeout)
    ));
    let mut request = Box::pin(source.search("cancel", Category::Movies));
    tokio::select! {
        started = tokio::time::timeout(Duration::from_secs(2), fixture.started.notified()) => started?,
        _ = &mut request => panic!("cancel fixture completed early"),
    }
    drop(request);
    tokio::time::timeout(Duration::from_secs(2), fixture.closed.notified()).await?;
    let before = fixture.hits.load(Ordering::SeqCst);
    for query in [
        "",
        "bad\nquery",
        &"x".repeat(501),
        "https://rutube.ru/video/bad/",
        "https://other.example/video/0123456789abcdef0123456789abcdef/",
    ] {
        assert!(source.search(query, Category::Movies).await.is_err());
    }
    for category in [
        Category::Manga,
        Category::Books,
        Category::Software,
        Category::Games,
    ] {
        assert!(source.search("generated", category).await?.is_empty());
    }
    let mut other_source = found.variants[0].reference.clone();
    other_source.publication.source = Source::AniLiberty;
    assert!(source.resolve(&other_source).await.is_err());
    assert_eq!(fixture.hits.load(Ordering::SeqCst), before);
    for prefix in ["video", "embed", "play/embed"] {
        let query = format!("https://rutube.ru/{prefix}/{}/?source=public", fixture::ID);
        let exact = source.search(&query, Category::Movies).await?;
        assert_eq!(exact[0].reference, found.reference);
        assert_eq!(exact[0].variants[0].reference, found.variants[0].reference);
    }
    assert_eq!(fixture.payload.load(Ordering::SeqCst), 0);
    fixture.stop().await?;
    println!(
        "PASS rutube_fixtures generated_https=ok exact_id=ok category=source_reported paid_locked=rejected changed_missing=rejected quality=exact_child maximum_ambiguity=rejected identical_mirrors=first_uri redirects=guarded challenge=rejected bounds=ok timeout=ok cancel=connection_closed persistent_urls=public media_payload=none"
    );
    Ok(())
}

fn persist_work(encoded: &str) -> CheckResult<()> {
    use qbutt_content::{Library, VariantPreferences, Work};

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-rutube-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&profile)?;
    let mut work: Work = serde_json::from_str(encoded)?;
    work.url = url::Url::parse(&format!("https://rutube.ru/video/{}/", work.reference.id))?;
    let mut library = Library::open(&profile)?;
    library.record_opened(
        &work,
        work.variants[0].reference.clone(),
        VariantPreferences {
            selected: Some(work.variants[0].reference.clone()),
            ..VariantPreferences::default()
        },
    )?;
    drop(library);
    let status = std::process::Command::new(std::env::current_exe()?)
        .arg("--reopen")
        .arg(&profile)
        .status()?;
    assert!(
        status.success(),
        "Rutube identity did not survive process restart"
    );
    std::fs::remove_dir_all(profile)?;
    Ok(())
}

fn verify_library(profile: &std::path::Path) -> CheckResult<()> {
    use qbutt_content::{Library, LibraryView, Source, VariantSelector};

    let library = Library::open(profile)?;
    let entries = library.entries(LibraryView::History)?;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.work.reference.source, Source::Rutube);
    assert_eq!(entry.work.reference.id, fixture::ID);
    assert_eq!(
        entry.work.variants[0].reference.selector,
        VariantSelector::RutubeVideo
    );
    assert_eq!(
        entry.preferences.selected.as_ref(),
        Some(&entry.work.variants[0].reference)
    );
    assert_eq!(
        &entry.last_opened.as_ref().unwrap().variant,
        &entry.work.variants[0].reference
    );
    assert_eq!(
        entry.work.url.as_str(),
        format!("https://rutube.ru/video/{}/", fixture::ID)
    );
    assert_eq!(entry.work.variants[0].quality.as_deref(), Some("1080p"));
    println!(
        "PASS rutube_library exact_publication_variant=restart selection=restart metadata=public"
    );
    Ok(())
}
