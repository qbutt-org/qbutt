//! Generated legal fiction over isolated TLS. Production files are unchanged;
//! only this process injects a private CA and controlled origin.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
pub use qbutt_content::{Error, Result};

#[path = "../src/text.rs"]
mod text;
pub use text::*;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");
    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod royalroad {
    include!("../src/royalroad.rs");
    pub(super) fn fixture(http: SourceHttp, origin: Url) -> RoyalRoad {
        RoyalRoad { http, origin }
    }
}

#[path = "royalroad/fixture.rs"]
mod fixture;

use fixture::Fault;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    if let Some(profile) = std::env::args_os().nth(1) {
        let library = qbutt_content::Library::open(profile)?;
        let reference = qbutt_content::PublicationRef {
            source: qbutt_content::Source::RoyalRoad,
            id: "42".into(),
        };
        let entry = library
            .get(&reference)?
            .ok_or("persisted fiction missing")?;
        assert!(entry.favorite);
        assert_eq!(
            entry.work.variants[0].transport,
            qbutt_content::Transport::Text
        );
        assert_eq!(
            entry.preferences.selected.as_ref().unwrap().selector,
            qbutt_content::VariantSelector::OriginalText
        );
        assert_eq!(entry.reader_progress.unwrap().chapter.id, "42/22");
        println!("PASS Royal Road public references and original selection after process restart");
        return Ok(());
    }
    let fixture = fixture::Fixture::start().await?;
    let provider = royalroad::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let search = provider.search("Generated & legal").await?;
    assert_eq!(search.len(), 1);
    assert_eq!(search[0].title, "Generated & legal fiction");
    assert!(search[0].needs_inspection);
    assert!(search[0].variants[0].languages.is_empty());
    assert_eq!(search[0].cover.as_ref().unwrap().query(), None);
    assert!(provider.search("empty").await?.is_empty());
    assert!(provider.search("no-cover").await?[0].cover.is_none());

    let publication = search[0].reference.clone();
    let work = provider.inspect(&publication).await?;
    assert_eq!(work.reference, publication);
    assert!(!work.needs_inspection);
    assert_eq!(work.variants[0].languages, ["en"]);
    let variant = &work.variants[0].reference;
    let chapters = provider.chapters(variant).await?;
    assert_eq!(
        chapters
            .iter()
            .map(|chapter| chapter.reference.id.as_str())
            .collect::<Vec<_>>(),
        ["42/21", "42/22"]
    );
    assert_eq!(chapters[1].title.as_deref(), Some("Second public chapter"));
    let selected = &chapters[1].reference;
    let chapter = provider.chapter_text(selected).await?;
    assert_eq!(chapter.reference, *selected);
    assert_eq!(chapter.blocks.len(), 2);
    assert_eq!(
        chapter.blocks[0].plain_text(),
        "Original paragraph with emphasis."
    );
    assert_eq!(chapter.blocks[1].plain_text(), "Second paragraph.");
    assert!(chapter.blocks[0].spans.iter().any(
        |span| matches!(span, TextSpan::Text { text, italic: true, .. } if text == "emphasis")
    ));
    assert!(
        chapter.blocks[1].spans.iter().any(
            |span| matches!(span, TextSpan::Text { text, bold: true, .. } if text == "Second")
        )
    );
    let repeated = provider.chapter_text(selected).await?;
    assert_eq!(chapter, repeated);

    // Round-trip the compiled fixture models through the public library API.
    let stored_work: qbutt_content::Work = serde_json::from_value(serde_json::to_value(&work)?)?;
    let stored_chapter: qbutt_content::ChapterRef =
        serde_json::from_value(serde_json::to_value(selected)?)?;
    let profile = std::env::temp_dir().join(format!(
        "qbutt-royalroad-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut library = qbutt_content::Library::open(&profile)?;
    library.set_favorite(&stored_work, true)?;
    let preferences = qbutt_content::VariantPreferences {
        selected: Some(stored_chapter.variant.clone()),
        ..Default::default()
    };
    let progress = qbutt_content::ReaderProgress {
        chapter: stored_chapter,
        page: 0,
        scroll_offset: 0.25,
    };
    library.save_reader_progress(&stored_work, preferences, progress)?;
    let mut invalid_work = stored_work.clone();
    invalid_work.variants[0].transport = qbutt_content::Transport::Manga;
    assert!(library.set_favorite(&invalid_work, false).is_err());
    drop(library);
    let status = std::process::Command::new(std::env::current_exe()?)
        .arg(&profile)
        .status()?;
    std::fs::remove_dir_all(&profile)?;
    assert!(status.success(), "library reopen process failed");

    let mut unavailable = selected.clone();
    unavailable.id = "42/23".into();
    assert!(matches!(
        provider.chapter_text(&unavailable).await,
        Err(Error::VariantUnavailable)
    ));
    unavailable.id = "42/999".into();
    assert!(matches!(
        provider.chapter_text(&unavailable).await,
        Err(Error::VariantUnavailable)
    ));
    *fixture.fault.lock().unwrap() = Fault::WrongFiction;
    assert!(matches!(
        provider.chapter_text(selected).await,
        Err(Error::VariantUnavailable)
    ));
    *fixture.fault.lock().unwrap() = Fault::NowPaid;
    assert!(matches!(
        provider.chapter_text(selected).await,
        Err(Error::VariantUnavailable)
    ));
    for fault in [
        Fault::WrongChapter,
        Fault::MissingBody,
        Fault::EmptyBody,
        Fault::DuplicateChapter,
        Fault::InvalidManifest,
        Fault::WrongLanguage,
        Fault::MissingSubscription,
        Fault::InvalidSubscription,
        Fault::MissingManifest,
        Fault::UnsafeChapter,
    ] {
        *fixture.fault.lock().unwrap() = fault;
        assert!(
            provider.chapter_text(selected).await.is_err(),
            "metadata fault {fault:?}"
        );
    }
    *fixture.fault.lock().unwrap() = Fault::None;
    for id in ["404", "410"] {
        assert!(matches!(
            provider
                .inspect(&PublicationRef {
                    source: Source::RoyalRoad,
                    id: id.into()
                })
                .await,
            Err(Error::VariantUnavailable)
        ));
    }
    let hits = fixture.hits.load(Ordering::SeqCst);
    for id in ["0", "042", "42/extra", "-42", "18446744073709551616"] {
        assert!(matches!(
            provider
                .inspect(&PublicationRef {
                    source: Source::RoyalRoad,
                    id: id.into()
                })
                .await,
            Err(Error::InvalidReference)
        ));
    }
    for id in ["22", "43/22", "42/022", "42/0", "42/22/extra"] {
        unavailable.id = id.into();
        assert!(matches!(
            provider.chapter_text(&unavailable).await,
            Err(Error::InvalidReference)
        ));
    }
    for query in ["", "\n", "bad\u{7f}query", &"x".repeat(501)] {
        assert!(matches!(
            provider.search(query).await,
            Err(Error::InvalidQuery)
        ));
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits);
    for fault in [
        "challenge",
        "mime",
        "utf8",
        "missing",
        "duplicate",
        "unsafe-row",
    ] {
        assert!(
            provider.search(fault).await.is_err(),
            "HTTP metadata fault {fault}"
        );
    }
    for fault in ["huge-length", "huge-stream"] {
        assert!(matches!(
            provider.search(fault).await,
            Err(Error::ResponseTooLarge)
        ));
    }
    for fault in ["foreign", "local", "downgrade"] {
        assert!(matches!(
            provider.search(fault).await,
            Err(Error::UnsafeAddress)
        ));
    }
    assert!(matches!(
        provider.search("loop").await,
        Err(Error::InvalidRedirect)
    ));
    assert!(matches!(
        provider.search("delay").await,
        Err(Error::Timeout)
    ));
    let mut operation = Box::pin(provider.search("cancel"));
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        _ = &mut operation => panic!("request completed before cancellation"),
    }
    drop(operation);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(provider.chapter_text(selected).await?, chapter);
    fixture.stop().await;
    println!(
        "PASS Royal Road generated TLS search/inspect/public TOC/nonfirst original, stable blocks, emphasis, hidden noise, paid/missing/changed identity, invalid metadata/references, bounded HTTP/redirect/timeout/cancel/retry"
    );
    Ok(())
}
