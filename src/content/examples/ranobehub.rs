//! Bounded public search, exact publication, human translation and one nonfirst
//! chapter. Only metadata and text counts are printed, never chapter content.

use qbutt_content::{RanobeHub, Source, Transport, VariantSelector};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let source = RanobeHub::new()?;
    let found = source.search("Overlord").await?;
    let publication = found
        .iter()
        .find(|work| work.reference.id == "206")
        .ok_or("the expected public publication was not found")?;
    let work = source.inspect(&publication.reference).await?;
    assert_eq!(work.reference, publication.reference);
    assert_eq!(work.reference.source, Source::RanobeHub);
    assert!(!work.needs_inspection);
    let translation = work
        .variants
        .iter()
        .find(|variant| {
            matches!(
                &variant.reference.selector,
                VariantSelector::RanobeHubTranslation { branch } if branch == "main"
            )
        })
        .ok_or("the human main translation is unavailable")?;
    assert_eq!(translation.transport, Transport::Text);
    let chapters = source.chapters(&translation.reference).await?;
    let second = chapters.get(1).ok_or("no public nonfirst chapter")?;
    let text = source.chapter_text(&second.reference).await?;
    assert_eq!(text.reference, second.reference);
    assert!(text.blocks.len() >= 20);
    assert!(text.text_bytes() >= 1000);
    println!(
        "PASS ranobehub_live search={} publication={} human_variants={} chapters={} nonfirst={} blocks={} text_bytes={} chapter_text=not_logged native_reader=unverified",
        found.len(),
        work.reference.id,
        work.variants.len(),
        chapters.len(),
        second.reference.id,
        text.blocks.len(),
        text.text_bytes()
    );
    Ok(())
}
