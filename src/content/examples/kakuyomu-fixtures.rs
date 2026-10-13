//! Generated legal prose and real HTTPS requests through unchanged provider code.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
#[cfg(windows)]
pub use qbutt_content::webview;

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
mod kakuyomu {
    include!("../src/kakuyomu.rs");
    pub(super) fn fixture(http: SourceHttp, origin: Url) -> Kakuyomu {
        Kakuyomu { http, origin }
    }
}

#[path = "kakuyomu/fixture.rs"]
mod fixture;
#[path = "kakuyomu/pages.rs"]
mod pages;

use std::sync::atomic::Ordering;
use std::time::Duration;

type CheckResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> CheckResult<()> {
    let fixture = fixture::Fixture::start().await?;
    let provider = kakuyomu::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let works = provider.search("generated").await?;
    assert_eq!(works.len(), 1);
    let work = provider.inspect(&works[0].reference).await?;
    assert_eq!(work.title, "生成した物語 & 原文");
    assert_eq!(work.variants[0].languages, ["ja"]);
    assert_eq!(work.variants[0].transport, Transport::Text);
    let chapters = provider.chapters(&work.variants[0].reference).await?;
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[1].volume.as_deref(), Some("第二章"));
    let selected = &chapters[1].reference;
    let text = provider.chapter_text(selected).await?;
    assert_eq!(text.reference, *selected);
    assert_eq!(text.blocks.len(), 2);
    assert!(text.blocks[0].spans.iter().any(|span| matches!(span,
        TextSpan::Ruby { text, reading } if text == "漢字" && reading == "かんじ")));
    assert!(text.blocks[0].spans.iter().any(|span| matches!(span,
        TextSpan::Text { text, bold: true, .. } if text == "強調")));
    assert!(text.blocks[1].plain_text().contains("二つ目"));
    assert!(
        !text
            .blocks
            .iter()
            .any(|block| block.plain_text().contains("unsafe"))
    );

    fixture.case("empty");
    assert!(provider.search("generated").await?.is_empty());
    fixture.case("next");
    assert!(provider.search("generated").await?.is_empty());
    assert!(matches!(
        provider.inspect(&work.reference).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.case("normal");
    let hits = fixture.hits.load(Ordering::SeqCst);
    for id in [
        "",
        "0",
        "01",
        "1/2",
        "https://evil.example/",
        &"1".repeat(33),
    ] {
        let invalid = PublicationRef {
            source: Source::Kakuyomu,
            id: id.into(),
        };
        assert!(matches!(
            provider.inspect(&invalid).await,
            Err(Error::InvalidReference)
        ));
    }
    let mut invalid = selected.clone();
    invalid.id = "1/2".into();
    assert!(matches!(
        provider.chapter_text(&invalid).await,
        Err(Error::InvalidReference)
    ));
    invalid = selected.clone();
    invalid.id = format!("{}/999", work.reference.id);
    assert!(matches!(
        provider.chapter_text(&invalid).await,
        Err(Error::VariantUnavailable)
    ));
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 1);

    for case in [
        "challenge",
        "bad-count",
        "wrong-query",
        "missing-policy",
        "changed-node",
        "foreign",
        "local",
        "downgrade",
        "loop",
        "huge",
        "wrong-mime",
        "utf8",
        "error",
    ] {
        fixture.case(case);
        assert!(
            provider.search("generated").await.is_err(),
            "search fault {case}"
        );
    }
    for case in [
        "foreign-work",
        "duplicate-episode",
        "incomplete",
        "huge-heading",
        "changed-episode",
        "foreign-chapter",
        "wrong-body",
        "missing-body",
        "redirect-work",
        "missing",
    ] {
        fixture.case(case);
        assert!(
            provider.chapter_text(selected).await.is_err(),
            "chapter fault {case}"
        );
    }
    fixture.case("redirect-search");
    assert_eq!(provider.search("generated").await?.len(), 1);
    fixture.case("delay");
    assert!(matches!(
        provider.search("generated").await,
        Err(Error::Timeout)
    ));

    fixture.case("cancel");
    let mut pending = Box::pin(provider.search("generated"));
    tokio::select! {
        _ = fixture.started.notified() => {},
        result = &mut pending => panic!("request completed before cancellation: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("request never reached server"),
    }
    drop(pending);
    tokio::time::timeout(Duration::from_secs(2), fixture.closed.notified()).await?;
    fixture.case("normal");
    assert_eq!(provider.chapter_text(selected).await?.blocks.len(), 2);

    let local = url::Url::parse("https://127.0.0.1/")?;
    let guarded = kakuyomu::fixture(
        http::SourceHttp::new(local.clone(), Default::default())?,
        local,
    );
    assert!(matches!(
        guarded.search("generated").await,
        Err(Error::UnsafeAddress)
    ));
    let local = url::Url::parse("https://localhost/")?;
    let guarded = kakuyomu::fixture(
        http::SourceHttp::new(local.clone(), Default::default())?,
        local,
    );
    assert!(matches!(
        guarded.search("generated").await,
        Err(Error::Network(_))
    ));
    fixture.stop().await;
    println!(
        "PASS kakuyomu-fixtures canonical=confirmed nonfirst=exact japanese_ruby_emphasis=preserved search_nodes=scoped toc=ordered next=excluded noresult=distinct invalid_parent=rejected redirects_dns=guarded response=bounded cancellation=connection_closed errors=reported"
    );
    Ok(())
}
