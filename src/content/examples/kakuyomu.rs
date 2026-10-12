//! Bounded live check; chapter contents are never printed or saved.

use qbutt_content::{ChapterRef, Error, Kakuyomu, PublicationRef, Source};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = Kakuyomu::new()?;
    let works = provider.search("桃太郎").await?;
    let work = works
        .iter()
        .find(|work| work.reference.id == "11054822663254506405")
        .ok_or("live acceptance work is absent from search")?;
    let inspected = provider.inspect(&work.reference).await?;
    assert_eq!(inspected.reference, work.reference);
    assert!(!inspected.needs_inspection);
    let chapters = provider.chapters(&inspected.variants[0].reference).await?;
    let selected = chapters
        .get(1)
        .ok_or("live acceptance requires a nonfirst chapter")?;
    let text = provider.chapter_text(&selected.reference).await?;
    assert_eq!(text.reference, selected.reference);
    assert!(text.blocks.len() > 2);
    assert!(text.text_bytes() > 100);
    assert!(text.blocks.iter().any(|block| {
        block
            .plain_text()
            .chars()
            .any(|character| ('\u{3040}'..='\u{9fff}').contains(&character))
    }));
    assert!(
        provider
            .search("qbutt-noresults-7ca346a42")
            .await?
            .is_empty()
    );
    let mut invalid = selected.reference.clone();
    invalid.id = "1/2".into();
    assert!(matches!(
        provider.chapter_text(&invalid).await,
        Err(Error::InvalidReference)
    ));
    let missing = PublicationRef {
        source: Source::Kakuyomu,
        id: "1".into(),
    };
    assert!(matches!(
        provider.inspect(&missing).await,
        Err(Error::VariantUnavailable)
    ));
    let invalid = ChapterRef {
        variant: selected.reference.variant.clone(),
        id: "invalid".into(),
    };
    assert!(invalid.validate().is_err());
    println!(
        "PASS kakuyomu live search={} work={} chapters={} nonfirst={} blocks={} original_bytes={} noresult=confirmed invalid_parent=rejected missing=unavailable contents=not_logged",
        works.len(),
        inspected.reference.id,
        chapters.len(),
        selected.reference.id,
        text.blocks.len(),
        text.text_bytes()
    );
    Ok(())
}
