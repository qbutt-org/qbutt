//! Live catalog and exact Kodik descriptor acceptance; no video payload is read.

use std::sync::Arc;

use qbutt_content::{AnimeSpirit, Error, ResolvedContent, Source, VariantSelector};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = Arc::new(AnimeSpirit::new()?);
    let search_source = Arc::clone(&source);
    let found =
        tokio::spawn(async move { search_source.search("Magical Explorer").await }).await??;
    let publication = found
        .iter()
        .find(|work| work.reference.source == Source::AnimeSpirit && work.reference.id == "16461")
        .ok_or("live AnimeSpirit search did not return the selected publication")?;
    assert!(publication.needs_inspection && publication.variants.is_empty());
    assert!(
        source
            .search("Магический исследователь")
            .await?
            .iter()
            .any(|work| work.reference == publication.reference)
    );

    let inspect_source = Arc::clone(&source);
    let inspect_reference = publication.reference.clone();
    let inspected =
        tokio::spawn(async move { inspect_source.inspect(&inspect_reference).await }).await??;
    assert_eq!(inspected.reference, publication.reference);
    assert!(!inspected.needs_inspection);
    let selected = inspected
        .variants
        .iter()
        .find(|variant| {
            variant.episode.as_deref() == Some("2")
                && variant.translation.as_deref() == Some("Субтитры AnimeJoy")
                && matches!(
                    variant.reference.selector,
                    VariantSelector::AnimeSpiritKodik { .. }
                )
        })
        .ok_or("exact episode and translation are unavailable")?;
    assert_eq!(selected.languages, ["ja", "ru"]);

    let encoded = serde_json::to_string(&inspected)?;
    assert!(!encoded.contains("kodikplayer.com"));
    assert!(!encoded.contains("only_translations="));

    let resolve_source = Arc::clone(&source);
    let resolve_reference = selected.reference.clone();
    let resolved =
        tokio::spawn(async move { resolve_source.resolve(&resolve_reference).await }).await??;
    match resolved {
        ResolvedContent::DirectVideo {
            reference,
            url,
            quality,
            ..
        } => {
            assert_eq!(reference, selected.reference);
            assert_eq!(url.scheme(), "https");
            println!(
                "AnimeSpirit exact episode and voice: {}p descriptor",
                quality.height()
            );
        }
        _ => return Err("exact AnimeSpirit variant resolved to the wrong transport".into()),
    }

    let mut changed = selected.reference.clone();
    if let VariantSelector::AnimeSpiritKodik { episode_id, .. } = &mut changed.selector {
        *episode_id = "999999999".into();
    }
    assert!(matches!(
        source.resolve(&changed).await,
        Err(Error::VariantUnavailable)
    ));
    Ok(())
}
