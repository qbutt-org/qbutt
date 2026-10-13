//! Bounded public search -> inspect -> nonfirst chapter. Only metadata and
//! counts are printed; no account, cookies or copyrighted chapter text is logged.

use std::error::Error;
use std::time::Duration;

use qbutt_content::{
    LibSession, PublicationRef, RanobeLib, Source, TextSpan, Transport, VariantSelector,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args().nth(1).as_deref() == Some("--browser") {
        #[cfg(windows)]
        {
            let discovery = std::env::args_os()
                .nth(2)
                .ok_or("pass the owned browser discovery path after --browser")?;
            return browser_read(discovery.into()).await;
        }
        #[cfg(not(windows))]
        return Err("RanobeLib browser acceptance requires Windows".into());
    }
    let source = RanobeLib::new()?;
    let mut session = LibSession::anonymous(Source::RanobeLib)?;
    assert!(source.search("\n", &mut session).await.is_err());
    assert!(source.search(&"x".repeat(501), &mut session).await.is_err());
    let reference = PublicationRef {
        source: Source::RanobeLib,
        id: "94231--rezero-kara-hajimeru-isekai-seikatsu-outo-no-ichinichi-hen".into(),
    };
    let cancelled = tokio::time::timeout(
        Duration::from_millis(1),
        source.search("Re:Zero", &mut session),
    )
    .await;
    assert!(
        cancelled.is_err(),
        "expected an in-flight request to be cancelled"
    );
    let found = source.search("Re:Zero", &mut session).await?;
    assert!(
        found
            .iter()
            .any(|work| work.reference == reference && work.needs_inspection)
    );
    let work = source.inspect(&reference, &mut session).await?;
    assert!(!work.needs_inspection);
    let variant = work
        .variants
        .iter()
        .find(|variant| {
            variant.reference.selector
                == VariantSelector::LibTranslation {
                    branch: Some(20944),
                }
        })
        .ok_or("known public translation missing")?;
    assert_eq!(variant.transport, Transport::Text);
    let chapters = source.chapters(&variant.reference, &mut session).await?;
    let selected = chapters.get(1).ok_or("public nonfirst chapter missing")?;
    let chapter = source
        .chapter_text(&selected.reference, &mut session)
        .await?;
    assert_eq!(chapter.reference, selected.reference);
    assert!(chapter.blocks.len() > 20 && chapter.text_bytes() > 5_000);

    // This older public upload uses the API's HTML format at the same location.
    let alternative = work
        .variants
        .iter()
        .find(|variant| {
            variant.reference.selector
                == VariantSelector::LibTranslation {
                    branch: Some(18996),
                }
        })
        .ok_or("second public translation missing")?;
    let alternative_chapters = source
        .chapters(&alternative.reference, &mut session)
        .await?;
    let alternative_chapter = alternative_chapters
        .iter()
        .skip(1)
        .find(|chapter| chapter.volume == selected.volume && chapter.number == selected.number)
        .ok_or("same nonfirst chapter missing from the second translation")?;
    let alternative_text = source
        .chapter_text(&alternative_chapter.reference, &mut session)
        .await?;
    assert_eq!(alternative_text.reference, alternative_chapter.reference);
    assert_ne!(alternative_text.reference, chapter.reference);
    assert!(alternative_text.text_bytes() > 5_000);

    let mut missing = selected.reference.clone();
    missing.id = u64::MAX.to_string();
    assert!(matches!(
        source.chapter_text(&missing, &mut session).await,
        Err(qbutt_content::Error::VariantUnavailable)
    ));
    let mut missing_branch = variant.reference.clone();
    missing_branch.selector = VariantSelector::LibTranslation {
        branch: Some(u64::MAX),
    };
    assert!(matches!(
        source.chapters(&missing_branch, &mut session).await,
        Err(qbutt_content::Error::VariantUnavailable)
    ));
    let mut foreign = reference.clone();
    foreign.source = Source::MangaLib;
    assert!(matches!(
        source.inspect(&foreign, &mut session).await,
        Err(qbutt_content::Error::InvalidReference)
    ));
    println!(
        "PASS RanobeLib public search/inspect/exact branch/nonfirst chapter; variants={} chapters={} chapter={} blocks={} text_bytes={} alternative_chapter={} alternative_text_bytes={} cancelled_request_reused=ok absent_branch_and_upload=rejected",
        work.variants.len(),
        chapters.len(),
        selected.reference.id,
        chapter.blocks.len(),
        chapter.text_bytes(),
        alternative_text.reference.id,
        alternative_text.text_bytes()
    );
    Ok(())
}

#[cfg(windows)]
async fn browser_read(discovery: std::path::PathBuf) -> Result<(), Box<dyn Error>> {
    use qbutt_content::browser::{BrowserBroker, BrowserChoice, BrowserCommand};

    let broker = BrowserBroker::start(discovery).await?;
    let browser = broker.handle();
    let result = tokio::time::timeout(Duration::from_secs(150), async {
        browser
            .request(
                BrowserChoice::Edge,
                BrowserCommand::Cookies {
                    url: "https://ranobelib.me/".parse()?,
                },
            )
            .await
            .map_err(|error| format!("RanobeLib browser connection failed: {error}"))?;

        let source = RanobeLib::new()?;
        let reference = PublicationRef {
            source: Source::RanobeLib,
            id: "252431--aglyeong".into(),
        };
        let mut anonymous = LibSession::anonymous(Source::RanobeLib)?;
        let anonymous_search = source
            .search("Злой дух", &mut anonymous)
            .await
            .map_err(|error| format!("RanobeLib anonymous search failed: {error}"))?;
        let anonymous_found = anonymous_search
            .iter()
            .any(|work| work.reference == reference);

        let mut session =
            LibSession::from_browser(Source::RanobeLib, Some(&browser), BrowserChoice::Edge)
                .await
                .map_err(|error| format!("RanobeLib session import failed: {error}"))?;
        let found = source
            .search("Злой дух", &mut session)
            .await
            .map_err(|error| format!("RanobeLib session search failed: {error}"))?;
        if !found.iter().any(|work| work.reference == reference) {
            return Err("selected RanobeLib publication missing from session search".into());
        }
        let work = source
            .inspect(&reference, &mut session)
            .await
            .map_err(|error| format!("RanobeLib session inspection failed: {error}"))?;
        let variant = work
            .variants
            .iter()
            .find(|variant| {
                variant.reference.selector == VariantSelector::LibTranslation { branch: None }
            })
            .ok_or("selected free translation missing")?;
        assert_eq!(variant.transport, Transport::Text);
        let chapters = source
            .chapters(&variant.reference, &mut session)
            .await
            .map_err(|error| format!("RanobeLib session TOC failed: {error}"))?;
        let selected = chapters.get(1).ok_or("free nonfirst chapter missing")?;
        assert_eq!(selected.reference.id, "3871011");
        assert_eq!(selected.volume.as_deref(), Some("1"));
        assert_eq!(selected.number.as_deref(), Some("2"));
        let chapter = source
            .chapter_text(&selected.reference, &mut session)
            .await
            .map_err(|error| format!("RanobeLib session chapter failed: {error}"))?;
        assert_eq!(chapter.reference, selected.reference);
        assert!(chapter.blocks.len() > 20 && chapter.text_bytes() > 5_000);
        let underlined_spans = chapter
            .blocks
            .iter()
            .flat_map(|block| &block.spans)
            .filter(|span| {
                matches!(
                    span,
                    TextSpan::Text {
                        underline: true,
                        ..
                    }
                )
            })
            .count();
        assert!(underlined_spans > 0);

        // The same chapter determines whether this is account-required access,
        // rather than a public chapter read with a connected browser session.
        let anonymous_chapter = source
            .chapter_text(&selected.reference, &mut anonymous)
            .await;
        let anonymous_chapter_available = anonymous_chapter.is_ok();
        let anonymous_error = anonymous_chapter.err().map(|error| error.to_string());
        println!(
            "{}",
            serde_json::json!({
                "status": "passed",
                "publication": reference.id,
                "variant": variant.reference,
                "chapter": selected.reference,
                "chapterOrdinal": 1,
                "chapters": chapters.len(),
                "blocks": chapter.blocks.len(),
                "textBytes": chapter.text_bytes(),
                "underlinedSpans": underlined_spans,
                "volume": selected.volume, "number": selected.number,
                "anonymousSearchFound": anonymous_found,
                "anonymousChapterAvailable": anonymous_chapter_available,
                "anonymousChapterError": anonymous_error,
                "nativeReader": "not checked by this provider example"
            })
        );
        Ok::<(), Box<dyn Error>>(())
    })
    .await;
    broker.shutdown().await?;
    result.map_err(|_| "RanobeLib browser acceptance exceeded its deadline")?
}
