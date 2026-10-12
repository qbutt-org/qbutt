//! Read-only official catalog acceptance. No torrent or game payload is fetched.

use qbutt_content::{Category, Error, FitGirl, ResolvedContent, VariantSelector};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let source = FitGirl::new()?;
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Cyberpunk".into());
    let found = source.search(&query, Category::Games).await?;
    let publication = found.first().ok_or("no official game results")?;
    assert!(publication.needs_inspection && publication.variants.is_empty());
    let work = source.inspect(&publication.reference).await?;
    assert_eq!(work.reference, publication.reference);
    assert_eq!(work.category, Category::Games);
    assert!(!work.needs_inspection && !work.variants.is_empty());
    for variant in &work.variants {
        match source.resolve(&variant.reference).await? {
            ResolvedContent::Torrent {
                reference, magnet, ..
            } => {
                assert_eq!(reference, variant.reference);
                assert!(magnet.query_pairs().any(|(key, value)| {
                    key == "xt"
                        && value.to_ascii_lowercase()
                            == format!("urn:btih:{}", variant.info_hash.as_ref().unwrap())
                }));
            }
            _ => return Err("wrong transport".into()),
        }
    }
    let mut missing = work.variants[0].reference.clone();
    missing.selector = VariantSelector::TorrentHash {
        info_hash: "0".repeat(40),
    };
    assert!(matches!(
        source.resolve(&missing).await,
        Err(Error::VariantUnavailable)
    ));
    assert!(
        source
            .search("qbuttgeneratedunlikelyemptyresult20261012", Category::Games)
            .await?
            .is_empty()
    );
    println!(
        "PASS fitgirl_live search={} publication={} variants={} exact_hash=ok missing_hash=rejected empty=ok game_payload=none direct_mirrors=unsupported",
        found.len(),
        work.reference.id,
        work.variants.len()
    );
    Ok(())
}
