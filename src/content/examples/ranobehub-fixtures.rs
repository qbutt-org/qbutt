//! Generated legal HTTPS data through production provider operations and a real
//! isolated SQLite profile. The private client exists only in this executable.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
pub use qbutt_content::{Error, Result};
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;
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
mod ranobehub {
    include!("../src/ranobehub.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> RanobeHub {
        RanobeHub { http, origin }
    }
}

#[path = "ranobehub/fixture.rs"]
mod fixture;

use std::sync::atomic::Ordering;
use std::time::Duration;

use fixture::Fixture;

type CheckResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> CheckResult<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
        let profile = args.next().ok_or("missing profile")?;
        let block_id = args.next().ok_or("missing block ID")?;
        verify_reopened(std::path::Path::new(&profile), &block_id.to_string_lossy())?;
        return Ok(());
    }

    let fixture = Fixture::start().await?;
    let source = ranobehub::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let found = source.search("generated & legal").await?;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].reference.id, "7");
    assert!(found[0].variants.is_empty());
    assert!(found[0].needs_inspection);
    assert!(found[0].cover.as_ref().unwrap().query().is_none());
    assert!(source.search("empty").await?.is_empty());
    let work = source.inspect(&found[0].reference).await?;
    assert_eq!(work.reference, found[0].reference);
    assert_eq!(work.aliases, ["Original title"]);
    assert_eq!(work.year, Some(2026));
    assert_eq!(work.variants.len(), 2);
    assert!(!work.needs_inspection);
    assert_eq!(work.variants[0].transport, Transport::Text);
    let selected = &work
        .variants
        .iter()
        .find(|variant| {
            matches!(
                &variant.reference.selector,
                VariantSelector::RanobeHubTranslation { branch } if branch == "main"
            )
        })
        .unwrap()
        .reference;
    let chapters = source.chapters(selected).await?;
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].reference.id, "100");
    assert_eq!(chapters[0].number.as_deref(), Some("2.5"));
    assert_eq!(chapters[1].reference.id, "101");
    assert_eq!(chapters[1].number.as_deref(), Some("2"));
    let chapter = &chapters[1].reference;
    let text = source.chapter_text(chapter).await?;
    assert_eq!(text.reference, *chapter);
    assert_eq!(text.blocks.len(), 2);
    assert_eq!(text.blocks[0].plain_text(), "First generated paragraph.");
    assert_eq!(text.blocks[1].plain_text(), "語(ご) second\nline");
    assert!(
        text.blocks[1]
            .spans
            .iter()
            .any(|span| matches!(span, TextSpan::Ruby { .. }))
    );
    assert!(
        text.blocks[1]
            .spans
            .iter()
            .any(|span| matches!(span, TextSpan::Text { italic: true, .. }))
    );
    persist(
        &serde_json::to_string(&work)?,
        &serde_json::to_string(&text)?,
    )?;
    let alternate = &work
        .variants
        .iter()
        .find(|variant| {
            matches!(
                &variant.reference.selector,
                VariantSelector::RanobeHubTranslation { branch } if branch == "alternate"
            )
        })
        .unwrap()
        .reference;
    let alternate_chapters = source.chapters(alternate).await?;
    assert_eq!(alternate_chapters[1].reference.id, "201");
    assert_eq!(
        source
            .chapter_text(&alternate_chapters[1].reference)
            .await?
            .reference,
        alternate_chapters[1].reference
    );

    for fault in [
        "book-id",
        "chapter-parent",
        "duplicate",
        "partial",
        "pagination",
        "duplicate-branches",
    ] {
        fixture.case(fault);
        assert!(
            source.inspect(&found[0].reference).await.is_err(),
            "accepted {fault}"
        );
    }
    fixture.case("too-many");
    assert!(matches!(
        source.chapters(selected).await,
        Err(Error::ResponseTooLarge)
    ));
    for fault in [
        "paid-list",
        "draft-list",
        "ai-list",
        "unknown-ai",
        "missing-branch",
    ] {
        fixture.case(fault);
        assert!(
            matches!(
                source.chapters(selected).await,
                Err(Error::VariantUnavailable)
            ),
            "accepted {fault}"
        );
    }
    for fault in ["body-id", "body-book", "body-branch", "empty-text"] {
        fixture.case(fault);
        assert!(
            matches!(
                source.chapter_text(chapter).await,
                Err(Error::InvalidResponse(_))
            ),
            "accepted {fault}"
        );
    }
    for fault in ["paid-body", "draft-body"] {
        fixture.case(fault);
        assert!(
            matches!(
                source.chapter_text(chapter).await,
                Err(Error::VariantUnavailable)
            ),
            "accepted {fault}"
        );
    }
    fixture.case("normal");
    assert!(matches!(
        source.search("http").await,
        Err(Error::HttpStatus(503))
    ));
    assert!(matches!(
        source.search("invalid-json").await,
        Err(Error::Json(_))
    ));
    assert!(matches!(
        source.search("huge").await,
        Err(Error::ResponseTooLarge)
    ));
    for query in ["redirect-private", "redirect-offsite"] {
        assert!(matches!(
            source.search(query).await,
            Err(Error::UnsafeAddress)
        ));
    }
    assert!(matches!(source.search("delay").await, Err(Error::Timeout)));
    let mut cancelled = Box::pin(source.search("cancel"));
    tokio::select! {
        result = &mut cancelled => panic!("request ended before cancellation: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(2), fixture.started.notified()) => { result?; }
    }
    drop(cancelled);
    tokio::time::timeout(Duration::from_secs(2), fixture.closed.notified()).await?;
    let hits = fixture.hits.load(Ordering::SeqCst);
    assert!(matches!(source.search("").await, Err(Error::InvalidQuery)));
    let mut invalid = chapter.clone();
    invalid.id = "../101".into();
    assert!(matches!(
        source.chapter_text(&invalid).await,
        Err(Error::InvalidReference)
    ));
    assert_eq!(hits, fixture.hits.load(Ordering::SeqCst));
    fixture.stop().await?;
    println!(
        "PASS ranobehub_fixtures search=ok pagination=ok source_order=ok exact_translation=ok paragraphs_ruby=ok active_markup=removed identity_faults=rejected paid_ai_unknown=excluded transport_bounds=ok cancellation=ok library_reopen=ok"
    );
    Ok(())
}

fn persist(encoded_work: &str, encoded_chapter: &str) -> CheckResult<()> {
    use qbutt_content::{Library, ReaderPosition, ReaderProgress, VariantPreferences, WorkGroup};

    let work: qbutt_content::Work = serde_json::from_str(encoded_work)?;
    let group = WorkGroup::single(work);
    let chapter: qbutt_content::TextChapter = serde_json::from_str(encoded_chapter)?;
    let block_id = chapter.blocks[0].id.clone();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-ranobehub-{nonce}"));
    std::fs::create_dir(&profile)?;
    let result = (|| -> CheckResult<()> {
        let mut library = Library::open(&profile)?;
        library.set_favorite(&group, true)?;
        let choices = VariantPreferences {
            selected: Some(chapter.reference.variant.clone()),
            ..Default::default()
        };
        let progress = ReaderProgress {
            chapter: chapter.reference,
            position: ReaderPosition::Text {
                block_id: block_id.clone(),
                offset: 0.625,
            },
        };
        library.save_reader_progress(&group, choices, progress)?;
        drop(library);
        let status = std::process::Command::new(std::env::current_exe()?)
            .arg("--reopen")
            .arg(&profile)
            .arg(&block_id)
            .status()?;
        assert!(status.success());
        Ok(())
    })();
    std::fs::remove_dir_all(profile)?;
    result
}

fn verify_reopened(profile: &std::path::Path, block_id: &str) -> CheckResult<()> {
    use qbutt_content::{Library, LibraryView, ReaderPosition, Source, VariantSelector};

    let library = Library::open(profile)?;
    let entries = library.entries(LibraryView::History)?;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.group.anchor.source, Source::RanobeHub);
    assert!(entry.favorite);
    let progress = entry.reader_progress.as_ref().unwrap();
    assert_eq!(progress.chapter.id, "101");
    assert_eq!(
        progress.position,
        ReaderPosition::Text {
            block_id: block_id.into(),
            offset: 0.625,
        }
    );
    assert_eq!(
        entry.preferences.selected.as_ref(),
        Some(&progress.chapter.variant)
    );
    assert!(
        matches!(&progress.chapter.variant.selector, VariantSelector::RanobeHubTranslation { branch } if branch == "main")
    );
    Ok(())
}
