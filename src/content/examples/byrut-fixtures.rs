//! Generated legal HTTPS search, article and torrent traffic through ByRut.
//! The listener never serves game payload or leaves the loopback interface.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
#[cfg(windows)]
pub use qbutt_content::webview;

#[allow(dead_code)]
#[path = "../src/torrent.rs"]
mod torrent;

#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod byrut {
    include!("../src/byrut.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> ByRut {
        ByRut { http, origin }
    }
}

#[path = "byrut/fixture.rs"]
mod fixture;

use std::sync::atomic::Ordering;
use std::time::Duration;

use fixture::{Fixture, alternate_torrent, changed_torrent, legal_torrent};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--live") {
        return live().await;
    }

    let fixture = Fixture::start().await?;
    let provider = byrut::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );

    let found = provider
        .search(
            "Generated Legal",
            Category::Games,
            None,
            browser::BrowserChoice::Auto,
        )
        .await?;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].reference.id, "42-generated-legal");
    assert!(found[0].needs_inspection);
    assert_eq!(found[0].description.as_deref(), Some("v 1.2"));
    let work = provider
        .inspect(&found[0].reference, None, browser::BrowserChoice::Auto)
        .await?;
    assert_eq!(work.variants.len(), 3);
    assert!(work.aliases.is_empty());
    let coop = provider
        .inspect(
            &PublicationRef {
                source: Source::ByRut,
                id: "49-generated-legal-po-seti-na-piratke".into(),
            },
            None,
            browser::BrowserChoice::Auto,
        )
        .await?;
    assert_eq!(coop.title, "Generated Legal по сети");
    assert_eq!(coop.aliases, ["Generated Legal"]);
    assert_eq!(
        work.variants[0].info_hash.as_deref(),
        Some("8aa48ccd171ff9e64325b54367e9aedcc2ce6af6")
    );
    assert_eq!(
        torrent::info_hash(changed_torrent())?,
        "c6efc810a63332cbaf4ad8384e0ee6ed3b473f4d"
    );
    assert!(torrent::info_hash(b"not bencoded metadata").is_err());
    assert!(
        work.variants[0]
            .title
            .as_ref()
            .unwrap()
            .contains("Полная версия")
    );
    assert!(work.variants[0].title.as_ref().unwrap().contains("v 1.2"));
    let selected = &work.variants[0].reference;
    let resolved = provider
        .resolve(selected, None, browser::BrowserChoice::Auto)
        .await?;
    let ResolvedContent::TorrentFile {
        reference,
        metadata,
    } = resolved
    else {
        panic!("ByRut did not provide torrent metadata");
    };
    assert_eq!(reference, *selected);
    assert_eq!(metadata.bytes(), legal_torrent());

    for (variant, expected, label) in [
        (
            &work.variants[1],
            changed_torrent(),
            "Generated RePack — v 1.1",
        ),
        (
            &work.variants[2],
            alternate_torrent(),
            "Generated Portable — v 1.0",
        ),
    ] {
        assert_eq!(variant.title.as_deref(), Some(label));
        assert_eq!(variant.transport, Transport::Torrent);
        assert_eq!(variant.reference.publication, work.reference);
        let resolved = provider
            .resolve(&variant.reference, None, browser::BrowserChoice::Auto)
            .await?;
        let ResolvedContent::TorrentFile {
            reference,
            metadata,
        } = resolved
        else {
            panic!("alternate ByRut torrent did not resolve");
        };
        assert_eq!(reference, variant.reference);
        assert_eq!(metadata.bytes(), expected);
    }

    for mode in [1, 2] {
        fixture.alternate_mode.store(mode, Ordering::SeqCst);
        assert!(matches!(
            provider
                .resolve(
                    &work.variants[1].reference,
                    None,
                    browser::BrowserChoice::Auto
                )
                .await,
            Err(Error::VariantUnavailable)
        ));
    }
    fixture.alternate_mode.store(0, Ordering::SeqCst);

    fixture.alternate_mode.store(3, Ordering::SeqCst);
    let repeated = provider
        .inspect(&work.reference, None, browser::BrowserChoice::Auto)
        .await?;
    assert_eq!(repeated.variants.len(), 2);
    assert_eq!(repeated.variants[0].reference, work.variants[0].reference);
    fixture.alternate_mode.store(0, Ordering::SeqCst);

    fixture.changed.store(true, Ordering::SeqCst);
    let downloads = fixture.downloads.load(Ordering::SeqCst);
    assert!(matches!(
        provider
            .resolve(selected, None, browser::BrowserChoice::Auto)
            .await,
        Err(Error::VariantUnavailable)
    ));
    assert_eq!(fixture.downloads.load(Ordering::SeqCst), downloads + 3);
    fixture.changed.store(false, Ordering::SeqCst);

    for query in ["empty", "unreleased"] {
        assert!(
            provider
                .search(query, Category::Games, None, browser::BrowserChoice::Auto)
                .await?
                .is_empty()
        );
    }
    let mixed = provider
        .search("mixed", Category::Games, None, browser::BrowserChoice::Auto)
        .await?;
    assert_eq!(mixed.len(), 1);
    assert_eq!(mixed[0].reference.id, "42-generated-legal");
    for query in [
        "mismatch",
        "foreign",
        "duplicate",
        "challenge",
        "source-error",
        "missing-results",
    ] {
        assert!(
            provider
                .search(query, Category::Games, None, browser::BrowserChoice::Auto)
                .await
                .is_err(),
            "accepted {query}"
        );
    }
    for id in ["43-identity", "44-challenge", "45-missing"] {
        let reference = PublicationRef {
            source: Source::ByRut,
            id: id.into(),
        };
        assert!(
            provider
                .inspect(&reference, None, browser::BrowserChoice::Auto)
                .await
                .is_err(),
            "accepted {id}"
        );
    }
    for id in ["46-unreleased", "47-storage-only"] {
        let reference = PublicationRef {
            source: Source::ByRut,
            id: id.into(),
        };
        let downloads = fixture.downloads.load(Ordering::SeqCst);
        assert!(matches!(
            provider
                .inspect(&reference, None, browser::BrowserChoice::Auto)
                .await,
            Err(Error::UnsupportedAcquisition(_))
        ));
        assert_eq!(fixture.downloads.load(Ordering::SeqCst), downloads);
    }
    assert!(matches!(
        provider
            .inspect(
                &PublicationRef {
                    source: Source::ByRut,
                    id: "48-malformed".into(),
                },
                None,
                browser::BrowserChoice::Auto,
            )
            .await,
        Err(Error::InvalidResponse("missing ByRut torrent link"))
    ));
    let storage_main = provider
        .inspect(
            &PublicationRef {
                source: Source::ByRut,
                id: "50-storage-with-alternate".into(),
            },
            None,
            browser::BrowserChoice::Auto,
        )
        .await?;
    assert_eq!(storage_main.variants.len(), 1);
    assert_eq!(
        storage_main.variants[0].title.as_deref(),
        Some("Generated RePack — v 1.1")
    );
    for id in [
        "51-invalid-block",
        "52-alternate-identity",
        "53-duplicate-block",
        "54-foreign-alternate",
        "55-alternate-challenge",
        "56-alternate-mime",
        "57-alternate-missing-link",
        "58-alternate-oversize",
        "59-alternate-bad-torrent",
        "60-excessive-alternates",
    ] {
        let error = provider
            .inspect(
                &PublicationRef {
                    source: Source::ByRut,
                    id: id.into(),
                },
                None,
                browser::BrowserChoice::Auto,
            )
            .await
            .err()
            .expect("invalid alternate was accepted");
        assert!(
            matches!(
                (id, error),
                (
                    "51-invalid-block" | "53-duplicate-block" | "60-excessive-alternates",
                    Error::InvalidResponse("invalid ByRut alternate block")
                ) | (
                    "52-alternate-identity",
                    Error::InvalidResponse("ByRut alternate identity changed")
                ) | (
                    "54-foreign-alternate",
                    Error::InvalidResponse("foreign ByRut URL")
                ) | ("55-alternate-challenge", Error::Challenge)
                    | (
                        "56-alternate-mime",
                        Error::InvalidResponse("ByRut document is not HTML")
                    )
                    | (
                        "57-alternate-missing-link",
                        Error::InvalidResponse("missing ByRut metadata")
                    )
                    | ("58-alternate-oversize", Error::ResponseTooLarge)
                    | (
                        "59-alternate-bad-torrent",
                        Error::InvalidResponse("invalid torrent metadata")
                    )
            ),
            "unexpected failure category for {id}"
        );
    }
    for id in ["../private", "42-Mixed", "42-", "0-game", "42-game?query"] {
        let reference = PublicationRef {
            source: Source::ByRut,
            id: id.into(),
        };
        assert!(matches!(
            provider
                .inspect(&reference, None, browser::BrowserChoice::Auto)
                .await,
            Err(Error::InvalidReference)
        ));
    }
    let mut wrong = selected.clone();
    wrong.selector = VariantSelector::TorrentHash {
        info_hash: "0123456789abcdef0123456789abcdef01234567".into(),
    };
    assert!(matches!(
        provider
            .resolve(&wrong, None, browser::BrowserChoice::Auto)
            .await,
        Err(Error::VariantUnavailable)
    ));
    assert!(
        provider
            .search(
                "Generated Legal",
                Category::Books,
                None,
                browser::BrowserChoice::Auto
            )
            .await?
            .is_empty()
    );
    let transport = http::fixture(fixture.client.clone(), fixture.origin.clone());
    let mut session = http::SourceSession::anonymous(fixture.origin.clone())?;
    let mut cancelled = Box::pin(transport.session_ajax_document(
        fixture.origin.join("/cancel-ajax")?,
        "block=features2&id=42",
        &mut session,
    ));
    tokio::select! {
        _ = fixture.ajax_started.notified() => {},
        result = &mut cancelled => panic!("AJAX completed before cancellation: {:?}", result.err()),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("AJAX cancellation request did not start"),
    }
    drop(cancelled);
    tokio::time::timeout(Duration::from_secs(3), fixture.ajax_closed.notified()).await?;
    fixture.stop().await;
    println!(
        "PASS byrut_fixtures generated_https search unavailable_rows_omitted main_and_alternates exact_versions exact_torrents changed_and_removed_alternate_rejected storage_not_requested ajax_cancel_drained faults_rejected game_payload=none"
    );
    Ok(())
}

async fn live() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let provider = byrut::ByRut::new()?;
    for (query, expected) in [
        ("rezero", None),
        ("ready or not", Some("2950-ready-or-not")),
        ("Stardew Valley", Some("2799-stardew-valley")),
    ] {
        let found = provider
            .search(query, Category::Games, None, browser::BrowserChoice::Auto)
            .await?;
        let Some(expected) = expected else {
            assert!(found.is_empty());
            println!("PASS byrut_live query={query} search_results=0 game_payload=none");
            continue;
        };
        let publication = found
            .iter()
            .find(|work| work.reference.id == expected)
            .ok_or("live ByRut search omitted the expected publication")?;
        let work = provider
            .inspect(&publication.reference, None, browser::BrowserChoice::Auto)
            .await?;
        assert_eq!(
            work.variants.len(),
            4,
            "live article alternate torrents were omitted"
        );
        for selected in &work.variants {
            let resolved = provider
                .resolve(&selected.reference, None, browser::BrowserChoice::Auto)
                .await?;
            let ResolvedContent::TorrentFile {
                reference,
                metadata,
            } = resolved
            else {
                return Err("live ByRut resolve did not return torrent metadata".into());
            };
            assert_eq!(reference, selected.reference);
            let hash = torrent::info_hash(metadata.bytes())?;
            assert_eq!(selected.info_hash.as_deref(), Some(hash.as_str()));
            println!(
                "PASS byrut_live {} label={} infohash={hash} metadata_bytes={} game_payload=none",
                work.reference.id,
                selected.title.as_deref().unwrap_or(""),
                metadata.bytes().len()
            );
        }
    }
    for id in [
        "17354-dont-give-up-not-ready-to-die",
        "56825-ready-or-not-the-deadline-is-coming",
    ] {
        let reference = PublicationRef {
            source: Source::ByRut,
            id: id.into(),
        };
        assert!(matches!(
            provider
                .inspect(&reference, None, browser::BrowserChoice::Auto)
                .await,
            Err(Error::UnsupportedAcquisition(_))
        ));
        println!("PASS byrut_live {id} unsupported_acquisition game_payload=none");
    }
    Ok(())
}
