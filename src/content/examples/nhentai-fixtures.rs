// SPDX-License-Identifier: GPL-3.0-or-later
//! Generated legal galleries and images through the native provider's real TLS/HTTP path.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
pub use qbutt_content::TorrentMetadata;
#[cfg(windows)]
pub use qbutt_content::webview;
#[cfg(windows)]
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
mod nhentai {
    include!("../src/nhentai.rs");
    pub(super) fn fixture(http: SourceHttp, origin: Url) -> Nhentai {
        Nhentai { http, origin }
    }
}

#[path = "nhentai/fixture.rs"]
mod fixture;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let decoder = args
        .next()
        .ok_or("usage: nhentai-fixtures <qbutt-reader-decode-fixture>")?;
    if decoder == "--reopen" {
        return verify_reopened(&std::path::PathBuf::from(
            args.next().ok_or("missing fixture profile")?,
        ));
    }
    if args.next().is_some() {
        return Err("unexpected fixture argument".into());
    }
    let fixture = fixture::Fixture::start().await?;
    let provider = nhentai::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let found = provider.search("Generated").await?;
    assert_eq!(found.len(), 1, "blacklisted galleries must be excluded");
    assert!(found[0].needs_inspection);
    let work = provider.inspect(&found[0].reference).await?;
    assert!(!work.needs_inspection);
    assert_eq!(work.title, "Generated Adventure");
    assert_eq!(work.aliases.len(), 2);
    assert_eq!(work.identifiers["nhentai"], "123");
    assert_eq!(
        work.cover.as_ref().unwrap().path(),
        "/galleries/456/cover.webp.webp"
    );
    let variant = &work.variants[0].reference;
    assert_eq!(work.variants[0].languages, ["en"]);
    assert_eq!(
        work.variants[0].translation.as_deref(),
        Some("Generated group")
    );
    let chapters = provider.chapters(variant).await?;
    assert_eq!(chapters.len(), 1);
    assert_eq!(chapters[0].pages, 2);
    let selected = &chapters[0].reference;
    assert_eq!(selected.id, work.reference.id);
    let pages = provider.chapter_pages(selected).await?;
    assert_eq!(pages.reference, *selected);
    assert_eq!(pages.images.len(), 2);
    assert!(
        fixture.image_requests.lock().unwrap().is_empty(),
        "metadata must not fetch images"
    );
    let second_page = provider.image_bytes(&pages.images[1]).await?;
    assert_eq!(second_page, fixture::png(2));
    assert_ne!(second_page, fixture::png(1));
    storage(
        &work,
        selected,
        &second_page,
        std::path::Path::new(&decoder),
    )?;
    assert_eq!(
        *fixture.image_requests.lock().unwrap(),
        ["/galleries/456/2.png"]
    );
    assert!(provider.search("empty").await?.is_empty());
    assert!(matches!(
        provider.search(" ").await,
        Err(Error::InvalidQuery)
    ));
    assert!(matches!(
        provider.search("malformed").await,
        Err(Error::Json(_))
    ));
    assert!(matches!(
        provider.search("blocked").await,
        Err(Error::HttpStatus(403))
    ));
    assert!(matches!(
        provider.search("oversized").await,
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        provider.search("timeout").await,
        Err(Error::Timeout)
    ));
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            provider.search("timeout")
        )
        .await
        .is_err()
    );
    assert_eq!(provider.search("Generated").await?.len(), 1);

    fixture.set_fault("changed");
    assert!(matches!(
        provider.inspect(&work.reference).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.set_fault("missing");
    assert!(matches!(
        provider.chapter_pages(selected).await,
        Err(Error::HttpStatus(404))
    ));
    for fault in [
        "missing_pages",
        "empty_pages",
        "excessive_pages",
        "page_gap",
        "duplicate_page",
        "duplicate_path",
        "invalid_media",
    ] {
        fixture.set_fault(fault);
        assert!(
            matches!(
                provider.chapter_pages(selected).await,
                Err(Error::InvalidResponse(_) | Error::Json(_))
            ),
            "{fault}"
        );
    }
    for fault in [
        "foreign_media",
        "absolute_path",
        "traversal",
        "encoded_traversal",
        "image_query",
        "cdn_private",
        "cdn_missing_number",
        "cdn_foreign",
        "cdn_path",
        "cdn_query",
        "cdn_port",
        "cdn_credentials",
        "cdn_http",
    ] {
        fixture.set_fault(fault);
        assert!(
            matches!(
                provider.chapter_pages(selected).await,
                Err(Error::UnsafeAddress)
            ),
            "{fault}"
        );
    }
    fixture.set_fault("cdn_empty");
    assert!(matches!(
        provider.chapter_pages(selected).await,
        Err(Error::InvalidResponse(_))
    ));
    fixture.set_fault("image_redirect");
    assert!(matches!(
        provider.image_bytes(&pages.images[1]).await,
        Err(Error::UnsafeAddress)
    ));
    fixture.set_fault("image_mime");
    assert!(matches!(
        provider.image_bytes(&pages.images[1]).await,
        Err(Error::InvalidResponse(_))
    ));
    fixture.set_fault("image_oversized");
    assert!(matches!(
        provider.image_bytes(&pages.images[1]).await,
        Err(Error::ResponseTooLarge)
    ));
    fixture.set_fault("");
    let mut wrong = selected.clone();
    wrong.id = "124".into();
    assert!(matches!(
        provider.chapter_pages(&wrong).await,
        Err(Error::InvalidReference)
    ));
    wrong = selected.clone();
    wrong.variant.selector = VariantSelector::WeebCentralPages;
    assert!(matches!(
        provider.chapter_pages(&wrong).await,
        Err(Error::InvalidReference)
    ));
    let mut invalid = work.reference.clone();
    invalid.id = "0123".into();
    assert!(matches!(
        provider.inspect(&invalid).await,
        Err(Error::InvalidReference)
    ));
    assert_eq!(
        provider
            .image_bytes(&provider.chapter_pages(selected).await?.images[1])
            .await?,
        fixture::png(2)
    );
    fixture.stop().await;
    println!(
        "PASS nhentai generated TLS search/empty/blacklist/inspect/exact gallery chapter/page 2/Qt decode+Referer; malformed/missing/identity/order/path/CDN/redirect/MIME/24MiB guards; timeout/cancel/retry; opt-in/share/Library page2+scroll/second-process reopen"
    );
    Ok(())
}

fn storage(
    work: &Work,
    chapter: &ChapterRef,
    image: &[u8],
    decoder: &std::path::Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    use qbutt_content::{
        Library, ReaderPosition, ReaderProgress, SharedWork, VariantPreferences, WorkGroup,
    };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let profile =
        std::env::temp_dir().join(format!("qbutt-nhentai-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&profile)?;
    let result = (|| -> std::result::Result<(), Box<dyn std::error::Error>> {
        let page = profile.join("page-2.png");
        let corrupt = profile.join("corrupt.png");
        std::fs::write(&page, image)?;
        std::fs::write(&corrupt, b"generated invalid PNG")?;
        let decoded = std::process::Command::new(decoder)
            .env("QT_FORCE_STDERR_LOGGING", "1")
            .arg(&page)
            .arg(&corrupt)
            .status()?;
        assert!(decoded.success(), "native decoder failed: {decoded}");
        let mut work: qbutt_content::Work = serde_json::from_value(serde_json::to_value(work)?)?;
        work.url = url::Url::parse("https://nhentai.net/g/123/")?;
        let chapter: qbutt_content::ChapterRef =
            serde_json::from_value(serde_json::to_value(chapter)?)?;
        let variant = chapter.variant.clone();
        let shared = SharedWork::new(vec![work.reference.clone()], Some(variant.clone()))?;
        assert_eq!(SharedWork::from_url(&shared.to_url()?)?, shared);
        let mut library = Library::open(&profile)?;
        let settings = library
            .source_settings()?
            .into_iter()
            .find(|settings| settings.source == qbutt_content::Source::Nhentai)
            .unwrap();
        assert!(!settings.enabled, "an adult catalogue must require opt-in");
        library.set_source_enabled(qbutt_content::Source::Nhentai, true)?;
        let group = WorkGroup::single(work.clone());
        // This is a storage fixture's confirmed-consumer input, not GUI acceptance.
        library.save_reader_progress(
            &group,
            VariantPreferences {
                selected: Some(variant),
                ..Default::default()
            },
            ReaderProgress {
                chapter,
                position: ReaderPosition::Page {
                    page: 1,
                    scroll_offset: 0.25,
                },
            },
        )?;
        let mut foreign = work;
        foreign.url = url::Url::parse("https://nhentai.net/g/124/")?;
        assert!(
            library
                .set_favorite(&WorkGroup::single(foreign), true)
                .is_err()
        );
        drop(library);
        assert!(
            std::process::Command::new(std::env::current_exe()?)
                .arg("--reopen")
                .arg(&profile)
                .status()?
                .success()
        );
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    result
}

fn verify_reopened(
    profile: &std::path::Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    use qbutt_content::{Library, LibraryView, ReaderPosition, Source};

    let library = Library::open(profile)?;
    assert!(
        library
            .source_settings()?
            .into_iter()
            .find(|settings| settings.source == Source::Nhentai)
            .unwrap()
            .enabled
    );
    let history = library.entries(LibraryView::History)?;
    assert_eq!(history.len(), 1);
    let entry = &history[0];
    let progress = entry.reader_progress.as_ref().unwrap();
    assert_eq!(progress.chapter.id, "123");
    assert_eq!(
        progress.chapter.variant.selector,
        qbutt_content::VariantSelector::NhentaiPages
    );
    assert_eq!(
        progress.position,
        ReaderPosition::Page {
            page: 1,
            scroll_offset: 0.25
        }
    );
    assert_eq!(
        entry.preferences.selected.as_ref(),
        Some(&progress.chapter.variant)
    );
    assert_eq!(
        entry.last_opened.as_ref().unwrap().variant,
        progress.chapter.variant
    );
    Ok(())
}
