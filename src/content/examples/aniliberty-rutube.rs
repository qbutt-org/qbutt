//! Live, read-only metadata and manifest check for an exact AniLiberty episode.
//! The default release reports licensed, free Rutube viewing. No segments are fetched.

use qbutt_content::{AniLiberty, PublicationRef, ResolvedContent, Source, VariantSelector};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let publication = PublicationRef {
        source: Source::AniLiberty,
        id: std::env::args().nth(1).unwrap_or_else(|| "9467".into()),
    };
    let episode = std::env::args().nth(2).unwrap_or_else(|| "2".into());
    let source = AniLiberty::new()?;
    let work = source.inspect(&publication).await?;
    let found = source.search(&work.title).await?;
    assert!(found.iter().any(|item| item.reference == publication));
    let variant = work
        .variants
        .iter()
        .find(|variant| {
            variant.episode.as_deref() == Some(episode.as_str())
                && matches!(variant.reference.selector, VariantSelector::Rutube { .. })
        })
        .ok_or("this episode has no Rutube variant")?;

    println!(
        "{}",
        serde_json::json!({
            "status": "inspected",
            "title": work.title,
            "episode": episode,
            "reference": variant.reference,
        })
    );
    let ResolvedContent::DirectVideo {
        reference, quality, ..
    } = source.resolve(&variant.reference).await?
    else {
        return Err("Rutube episode resolved as a different transport".into());
    };
    assert_eq!(reference, variant.reference);
    println!(
        "PASS aniliberty_rutube_live publication={} episode={} quality={}p exact_reference=ok media_payload=none native_playback=unverified",
        publication.id,
        episode,
        quality.height()
    );
    Ok(())
}
