// SPDX-License-Identifier: GPL-3.0-or-later
//! Generated manga metadata and a generated one-pixel image over isolated TLS.

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
mod weebcentral {
    include!("../src/weebcentral.rs");
    pub(super) fn fixture(http: SourceHttp, origin: Url) -> WeebCentral {
        WeebCentral { http, origin }
    }
}

#[path = "weebcentral/fixture.rs"]
mod fixture;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture::Fixture::start().await?;
    let provider = weebcentral::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let found = provider.search("Generated").await?;
    assert_eq!(found.len(), 1);
    assert!(found[0].needs_inspection);
    let work = provider.inspect(&found[0].reference).await?;
    assert!(!work.needs_inspection);
    assert_eq!(work.title, "Generated Adventure");
    assert_eq!(work.year, Some(2026));
    assert_eq!(work.variants[0].languages, ["en"]);
    let variant = &work.variants[0].reference;
    let chapters = provider.chapters(variant).await?;
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[1].number.as_deref(), Some("2"));
    assert!(chapters.iter().all(|chapter| chapter.title.is_none()));
    let selected = &chapters[1].reference;
    for fault in ["", "legacy_cdn"] {
        fixture.set_fault(fault);
        let pages = provider.chapter_pages(selected).await?;
        assert_eq!(pages.images.len(), 2);
        assert_eq!(
            provider.image_bytes(&pages.images[0]).await?,
            fixture::png()
        );
    }
    fixture.set_fault("");
    assert!(provider.search("empty").await?.is_empty());
    assert!(matches!(
        provider.search("malformed").await,
        Err(Error::InvalidResponse(_))
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
    let pending = provider.search("timeout");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), pending)
            .await
            .is_err()
    );
    assert_eq!(provider.search("Generated").await?.len(), 1);
    assert!(matches!(
        provider.search(" ").await,
        Err(Error::InvalidQuery)
    ));

    for fault in ["adult", "missing_adult"] {
        fixture.set_fault(fault);
        let inspected = provider.inspect(&work.reference).await?;
        assert_eq!(inspected.reference, work.reference);
        assert_eq!(inspected.variants[0].reference, *variant);
    }
    fixture.set_fault("changed_publication");
    assert!(matches!(
        provider.inspect(&work.reference).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.set_fault("missing_publication");
    assert!(matches!(
        provider.inspect(&work.reference).await,
        Err(Error::HttpStatus(404))
    ));
    fixture.set_fault("duplicate_chapter");
    assert!(matches!(
        provider.chapters(variant).await,
        Err(Error::InvalidResponse(_))
    ));
    fixture.set_fault("large_chapters");
    assert_eq!(provider.chapters(variant).await?.len(), 2);
    fixture.set_fault("oversized_chapters");
    assert!(matches!(
        provider.chapters(variant).await,
        Err(Error::ResponseTooLarge)
    ));
    for fault in ["changed_chapter", "foreign_publication"] {
        fixture.set_fault(fault);
        assert!(matches!(
            provider.chapter_pages(selected).await,
            Err(Error::VariantUnavailable)
        ));
    }
    for fault in ["missing_images", "duplicate_image", "page_order"] {
        fixture.set_fault(fault);
        assert!(matches!(
            provider.chapter_pages(selected).await,
            Err(Error::InvalidResponse(_))
        ));
    }
    for fault in ["unsafe_image", "foreign_image", "image_port"] {
        fixture.set_fault(fault);
        assert!(matches!(
            provider.chapter_pages(selected).await,
            Err(Error::UnsafeAddress)
        ));
    }
    fixture.set_fault("");
    let mut wrong_branch = variant.clone();
    wrong_branch.selector = VariantSelector::MangaTranslation {
        language: "ru".into(),
        groups: Vec::new(),
    };
    assert!(matches!(
        provider.chapters(&wrong_branch).await,
        Err(Error::InvalidReference)
    ));
    let mut invalid = work.reference.clone();
    invalid.id = "01I76XY7E9FNDZ1DBBM6PBJPFK".into();
    assert!(matches!(
        provider.inspect(&invalid).await,
        Err(Error::InvalidReference)
    ));
    assert_eq!(provider.chapter_pages(selected).await?.images.len(), 2);
    fixture.stop().await;
    println!(
        "PASS WeebCentral generated TLS search/empty/inspect/nonfirst chapter/image, current and legacy CDN, exact publication/selector/chapter, optional age metadata, duplicate and unsafe resources, foreign CDN and port rejection, HTTP failure, timeout, cancellation, default 2MiB and chapter-list 4MiB bounds, retry"
    );
    Ok(())
}
