//! Bounded public search -> inspect -> nonfirst chapter. Only metadata and
//! counts are printed; no account, cookies or copyrighted chapter text is logged.

use std::error::Error;
use std::time::Duration;

use qbutt_content::{PublicationRef, RanobeLib, Source, Transport, VariantSelector};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let source = RanobeLib::new()?;
    assert!(source.search("\n").await.is_err());
    assert!(source.search(&"x".repeat(501)).await.is_err());
    let reference = PublicationRef {
        source: Source::RanobeLib,
        id: "94231--rezero-kara-hajimeru-isekai-seikatsu-outo-no-ichinichi-hen".into(),
    };
    let cancelled = tokio::time::timeout(Duration::from_millis(1), source.search("Re:Zero")).await;
    assert!(
        cancelled.is_err(),
        "expected an in-flight request to be cancelled"
    );
    let found = source.search("Re:Zero").await?;
    assert!(
        found
            .iter()
            .any(|work| work.reference == reference && work.needs_inspection)
    );
    let work = source.inspect(&reference).await?;
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
    let chapters = source.chapters(&variant.reference).await?;
    let selected = chapters.get(1).ok_or("public nonfirst chapter missing")?;
    let chapter = source.chapter_text(&selected.reference).await?;
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
    let alternative_chapters = source.chapters(&alternative.reference).await?;
    let alternative_chapter = alternative_chapters
        .iter()
        .skip(1)
        .find(|chapter| chapter.volume == selected.volume && chapter.number == selected.number)
        .ok_or("same nonfirst chapter missing from the second translation")?;
    let alternative_text = source.chapter_text(&alternative_chapter.reference).await?;
    assert_eq!(alternative_text.reference, alternative_chapter.reference);
    assert_ne!(alternative_text.reference, chapter.reference);
    assert!(alternative_text.text_bytes() > 5_000);

    let mut missing = selected.reference.clone();
    missing.id = u64::MAX.to_string();
    assert!(matches!(
        source.chapter_text(&missing).await,
        Err(qbutt_content::Error::VariantUnavailable)
    ));
    let mut missing_branch = variant.reference.clone();
    missing_branch.selector = VariantSelector::LibTranslation {
        branch: Some(u64::MAX),
    };
    assert!(matches!(
        source.chapters(&missing_branch).await,
        Err(qbutt_content::Error::VariantUnavailable)
    ));
    let mut foreign = reference.clone();
    foreign.source = Source::MangaLib;
    assert!(matches!(
        source.inspect(&foreign).await,
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
