//! Generated legal HTTPS publications exercise the production provider, parser
//! and transport. The loopback TLS override exists only in this executable.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
pub use qbutt_content::{Error, Result};

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[path = "../src/torrent.rs"]
mod torrent;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod fitgirl {
    include!("../src/fitgirl.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> FitGirl {
        FitGirl { http, origin }
    }
}

#[path = "fitgirl/fixture.rs"]
mod fixture;

use fixture::Fixture;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::start().await?;
    let source = fitgirl::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let results = source.search("generated & legal", Category::Games).await?;
    assert_eq!(results.len(), 1);
    let found = &results[0];
    assert_eq!(found.reference.id, "generated-legal-game");
    assert_eq!(found.title, "Generated & Legal: Complete Edition – v2.0");
    assert!(found.needs_inspection && found.variants.is_empty());
    let work = source.inspect(&found.reference).await?;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.variants.len(), 2);
    assert!(!work.needs_inspection);
    assert!(
        work.variants[0]
            .title
            .as_ref()
            .unwrap()
            .contains("Complete Edition (v2.0)")
    );
    assert!(work.variants[1].title.as_ref().unwrap().contains("v1.0"));
    for variant in &work.variants {
        assert!(
            variant.size.is_none() && variant.seeders.is_none() && variant.languages.is_empty()
        );
        let ResolvedContent::Torrent {
            reference, magnet, ..
        } = source.resolve(&variant.reference).await?
        else {
            panic!("wrong transport");
        };
        assert_eq!(reference, variant.reference);
        assert!(magnet.query_pairs().any(|(key, value)| key == "xt"
            && value == format!("urn:btih:{}", variant.info_hash.as_ref().unwrap())));
    }
    fixture.changed.store(true, Ordering::SeqCst);
    assert!(matches!(
        source.resolve(&work.variants[0].reference).await,
        Err(Error::VariantUnavailable)
    ));
    assert!(source.resolve(&work.variants[1].reference).await.is_ok());
    fixture.changed.store(false, Ordering::SeqCst);
    for slug in ["missing", "gone", "redirected", "identity", "announcement"] {
        assert!(
            matches!(
                source.inspect(&publication(slug)).await,
                Err(Error::VariantUnavailable)
            ),
            "accepted {slug}"
        );
    }
    assert!(matches!(
        source.inspect(&publication("direct-only")).await,
        Err(Error::UnsupportedAcquisition(_))
    ));
    for slug in ["challenge", "bad-magnet", "duplicate-canonical"] {
        assert!(
            source.inspect(&publication(slug)).await.is_err(),
            "accepted {slug}"
        );
    }
    assert!(source.search("empty", Category::Games).await?.is_empty());
    assert!(
        source
            .search("announcements", Category::Games)
            .await?
            .is_empty()
    );
    for fault in [
        "challenge",
        "identity",
        "duplicate",
        "foreign",
        "missing-action",
        "mime",
        "utf8",
    ] {
        assert!(
            source.search(fault, Category::Games).await.is_err(),
            "accepted {fault}"
        );
    }
    assert!(matches!(
        source.search("huge", Category::Games).await,
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        source.search("delay", Category::Games).await,
        Err(Error::Timeout)
    ));
    assert!(matches!(
        source.search("foreign-redirect", Category::Games).await,
        Err(Error::UnsafeAddress)
    ));
    assert!(matches!(
        source.search("redirect-loop", Category::Games).await,
        Err(Error::InvalidRedirect)
    ));

    let before = fixture.hits.load(Ordering::SeqCst);
    for category in [
        Category::Anime,
        Category::Movies,
        Category::Tv,
        Category::Books,
        Category::Software,
        Category::Manga,
    ] {
        assert!(source.search("generated", category).await?.is_empty());
    }
    for query in ["", "bad\nquery", &"x".repeat(501)] {
        assert!(matches!(
            source.search(query, Category::Games).await,
            Err(Error::InvalidQuery)
        ));
    }
    for slug in [
        "",
        "../private",
        "Mixed",
        "query?key=value",
        "a/b",
        "-game",
        "game-",
        &"a".repeat(201),
    ] {
        assert!(matches!(
            source.inspect(&publication(slug)).await,
            Err(Error::InvalidReference)
        ));
    }
    let mut wrong_source = found.reference.clone();
    wrong_source.source = Source::Nyaa;
    assert!(matches!(
        source.inspect(&wrong_source).await,
        Err(Error::InvalidReference)
    ));
    assert_eq!(fixture.hits.load(Ordering::SeqCst), before);

    let mut request = Box::pin(source.search("cancel", Category::Games));
    tokio::select! {
        started = tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.cancelled.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        }) => started?,
        _ = &mut request => panic!("cancellation request completed early"),
    }
    drop(request);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    check_library(&work)?;
    assert_eq!(fixture.payload.load(Ordering::SeqCst), 0);
    fixture.stop().await;
    println!(
        "PASS fitgirl_fixtures generated_https=ok lazy_inspection=ok all_hashes=ok old_title=preserved duplicate_mirrors=deduped changed_missing=rejected identity_challenge=rejected redirects=bounded timeout_cancel=ok sqlite=ok game_payload=none direct_mirrors=unsupported"
    );
    Ok(())
}

fn publication(slug: &str) -> PublicationRef {
    PublicationRef {
        source: Source::FitGirl,
        id: slug.into(),
    }
}

fn check_library(work: &Work) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let work: qbutt_content::Work = serde_json::from_value(serde_json::to_value(work)?)?;
    let profile =
        std::env::temp_dir().join(format!("qbutt-fitgirl-library-{}", std::process::id()));
    std::fs::create_dir(&profile)?;
    {
        let mut library = qbutt_content::Library::open(&profile)?;
        library.set_favorite(&work, true)?;
    }
    {
        let library = qbutt_content::Library::open(&profile)?;
        assert_eq!(library.get(&work.reference)?.unwrap().work, work);
    }
    std::fs::remove_dir_all(profile)?;
    Ok(())
}
