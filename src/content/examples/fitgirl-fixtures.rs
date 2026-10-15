//! Generated legal HTTPS publications exercise the production provider, parser
//! and transport. The loopback TLS override exists only in this executable.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
#[path = "../src/html_text.rs"]
mod html_text;
#[path = "../src/query.rs"]
mod query;
#[cfg(windows)]
pub use qbutt_content::webview;

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

    pub(super) fn fixture(http: SourceHttp, origin: Url, client: reqwest::Client) -> FitGirl {
        let torrent_pastes = privatebin::HOSTS
            .iter()
            .map(|host| {
                super::http::fixture(
                    client.clone(),
                    Url::parse(&format!("https://{host}")).unwrap(),
                )
            })
            .collect();
        FitGirl {
            http,
            origin,
            torrent_pastes,
        }
    }
}

#[path = "fitgirl/fixture.rs"]
mod fixture;
#[path = "fitgirl/paste.rs"]
mod paste_fixture;

use fixture::Fixture;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::start().await?;
    let source = fitgirl::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
        fixture.client.clone(),
    );
    let mut results = source.search("generated & legal", Category::Games).await?;
    assert_eq!(results.len(), 2);
    results.retain(|work| query::matches("generated & legal", work));
    assert_eq!(results.len(), 1);
    let found = &results[0];
    assert_eq!(found.reference.id, "generated-legal-game");
    assert_eq!(found.title, "Generated & Legal: Complete Edition – v2.0");
    assert!(found.needs_inspection && found.variants.is_empty());
    check_search_relevance(found);
    let work = source.inspect(&found.reference).await?;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.variants.len(), 2);
    assert!(!work.needs_inspection);
    let description = work.description.as_deref().expect("published description");
    assert!(description.contains("Repack Features\n- Generated installation needs 4 GB of RAM."));
    assert!(description.contains("Generated legal game description."));
    assert!(description.contains("Backwards Compatibility\nReuse the unchanged generated files."));
    assert!(!description.contains("POISON"));
    assert!(work.notes.is_empty());
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
    for case in ["normal", "mirror"] {
        let work = source
            .inspect(&publication(&format!("attachment-{case}")))
            .await?;
        assert_eq!(work.variants.len(), 1);
        let selected = &work.variants[0].reference;
        let ResolvedContent::TorrentFile {
            reference,
            metadata,
        } = source.resolve(selected).await?
        else {
            panic!("published attachment did not produce torrent metadata");
        };
        assert_eq!(reference, *selected);
        assert_eq!(metadata.bytes(), paste_fixture::metadata(false));
        assert_eq!(torrent::info_hash(metadata.bytes())?, paste_fixture::hash());
        let serialized = serde_json::to_string(&work)?;
        assert!(!serialized.contains("paste.fitgirl") && !serialized.contains(paste_fixture::KEY));
        check_library(&work)?;

        let mut missing = selected.clone();
        missing.selector = VariantSelector::TorrentHash {
            info_hash: "0".repeat(40),
        };
        let before = fixture.hits.load(Ordering::SeqCst);
        assert!(matches!(
            source.resolve(&missing).await,
            Err(Error::VariantUnavailable)
        ));
        assert_eq!(fixture.hits.load(Ordering::SeqCst), before + 1);
    }
    for case in ["expired", "unavailable", "delay", "challenge"] {
        let work = source
            .inspect(&publication(&format!("attachment-{case}")))
            .await?;
        let selected = &work.variants[0].reference;
        let ResolvedContent::Torrent {
            reference, magnet, ..
        } = source.resolve(selected).await?
        else {
            panic!("unavailable attachment disabled the exact published magnet");
        };
        assert_eq!(reference, *selected);
        assert_eq!(torrent::magnet(magnet.as_str())?.1, paste_fixture::hash());
    }
    for (case, expected) in [
        (
            "hash",
            Error::InvalidResponse("FitGirl torrent hash changed"),
        ),
        (
            "auth",
            Error::InvalidResponse("FitGirl torrent paste authentication failed"),
        ),
        (
            "work",
            Error::InvalidResponse("unsupported FitGirl torrent paste format"),
        ),
        ("huge", Error::ResponseTooLarge),
        ("expanded", Error::ResponseTooLarge),
        (
            "type",
            Error::InvalidResponse("FitGirl attachment is not a torrent"),
        ),
        (
            "identity",
            Error::InvalidResponse("FitGirl torrent paste identity changed"),
        ),
    ] {
        let work = source
            .inspect(&publication(&format!("attachment-{case}")))
            .await?;
        let Err(error) = source.resolve(&work.variants[0].reference).await else {
            panic!("accepted attachment {case}");
        };
        assert_eq!(
            error.to_string(),
            expected.to_string(),
            "wrong rejection for attachment {case}"
        );
    }
    for case in ["foreign", "key", "ambiguous"] {
        let before = fixture.hits.load(Ordering::SeqCst);
        assert!(
            source
                .inspect(&publication(&format!("attachment-{case}")))
                .await
                .is_err()
        );
        assert_eq!(fixture.hits.load(Ordering::SeqCst), before + 1);
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
    fixture.cancelled.store(0, Ordering::SeqCst);
    let work = source.inspect(&publication("attachment-delay")).await?;
    let mut request = Box::pin(source.resolve(&work.variants[0].reference));
    tokio::select! {
        started = tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.cancelled.load(Ordering::SeqCst) != 1 { tokio::task::yield_now().await; }
        }) => started?,
        _ = &mut request => panic!("attachment cancellation completed early"),
    }
    drop(request);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert_eq!(fixture.payload.load(Ordering::SeqCst), 0);
    fixture.stop().await;
    println!(
        "PASS fitgirl_fixtures generated_https=ok lazy_inspection=ok all_hashes=ok old_title=preserved duplicate_mirrors=deduped torrent_attachments=exact same_hash_magnet_recovery=ok integrity_identity_mismatch=rejected work_factor_compressed_expanded_bounds=ok changed_missing=rejected identity_challenge=rejected redirects=bounded timeout_cancel=ok sqlite=ok game_payload=none direct_game_archives=unsupported"
    );
    Ok(())
}

fn publication(slug: &str) -> PublicationRef {
    PublicationRef {
        source: Source::FitGirl,
        id: slug.into(),
    }
}

fn check_search_relevance(found: &Work) {
    let cases = [
        (
            "ready or not",
            "Ready or Not: Digital Deluxe Edition, v112610 (v1.4.1)",
            true,
        ),
        ("ready or not", "MEMORIAPOLIS – v1.6.8", false),
        ("ready or not", "Don't Give Up: Not Ready to Die", false),
        ("ready or not", "Ready, Steady, Ship!", false),
        ("ready or not", "Just Die Already", false),
        ("READY—OR: NOT", "Ready or Not", true),
        ("ready or n", "Ready or Not", true),
        ("ready", "Already There", false),
        ("resident evil", "Resident Evil 4 Remake", true),
        ("resident evil 4", "Resident Evil 40", false),
        ("86", "86: Eighty-Six, Season 2", true),
        ("86", "868: Generated Work", false),
        ("86", "1986: Generated Work", false),
        ("stalker", "S.T.A.L.K.E.R. 2: Heart of Chornobyl", true),
        ("baldurs gate", "Baldur’s Gate 3", true),
        ("синий ар", "Синий архив", true),
        (" \t ", "Ready or Not", false),
        ("— / .", "Ready or Not", false),
    ];
    for (query, title, expected) in cases {
        let mut work = found.clone();
        work.title = title.into();
        assert_eq!(query::matches(query, &work), expected, "{query:?}: {title}");
        assert!(work.needs_inspection && work.variants.is_empty());
    }

    let mut aliased = found.clone();
    aliased.title = "Blue Archive".into();
    aliased.aliases = vec!["Синий архив".into(), "ブルーアーカイブ".into()];
    assert!(query::matches("синий архив", &aliased));
    assert!(query::matches("Blue Arch", &aliased));
    assert!(query::matches("ブルー", &aliased));
    assert!(!query::matches("синий Blue", &aliased));
}

fn check_library(work: &Work) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let work: qbutt_content::Work = serde_json::from_value(serde_json::to_value(work)?)?;
    let profile =
        std::env::temp_dir().join(format!("qbutt-fitgirl-library-{}", std::process::id()));
    std::fs::create_dir(&profile)?;
    {
        let mut library = qbutt_content::Library::open(&profile)?;
        library.set_favorite(&qbutt_content::WorkGroup::single(work.clone()), true)?;
    }
    {
        let library = qbutt_content::Library::open(&profile)?;
        assert_eq!(
            library
                .get(&work.reference)?
                .unwrap()
                .group
                .anchor_work()
                .unwrap(),
            &work
        );
    }
    std::fs::remove_dir_all(profile)?;
    Ok(())
}
