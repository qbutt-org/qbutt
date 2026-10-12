//! Live catalog and exact Kodik descriptor acceptance; no video payload is read.

use qbutt_content::{AnimeSpirit, ResolvedContent, Source, VariantSelector};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = AnimeSpirit::new()?;
    let found = source.search("Magical Explorer").await?;
    let publication = found
        .iter()
        .find(|work| work.reference.source == Source::AnimeSpirit && work.reference.id == "16461")
        .ok_or("live AnimeSpirit search did not return the selected publication")?;
    assert!(publication.needs_inspection && publication.variants.is_empty());

    let inspected = source.inspect(&publication.reference).await?;
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

    match source.resolve(&selected.reference).await? {
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
    Ok(())
}
