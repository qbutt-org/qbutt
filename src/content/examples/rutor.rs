//! Bounded read-only live catalog acceptance. This resolves one exact magnet;
//! it never adds a torrent or downloads its payload.

use qbutt_content::{Category, Error, ResolvedContent, RuTor, VariantSelector};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let query = std::env::args().nth(1).unwrap_or_else(|| "blender".into());
    let source = RuTor::new()?;
    let works = source.search(&query, Category::Software).await?;
    let selected = works.first().ok_or("no live source results")?;
    assert_eq!(selected.category, Category::Software);
    let work = source.inspect(&selected.reference).await?;
    assert_eq!(work.reference, selected.reference);
    assert_eq!(work.category, Category::Software);
    assert!(!work.needs_inspection);
    let variant = &selected.variants[0].reference;
    match source.resolve(variant).await? {
        ResolvedContent::Torrent {
            reference, magnet, ..
        } => {
            assert_eq!(reference, *variant);
            assert_eq!(magnet.scheme(), "magnet");
        }
        _ => return Err("wrong torrent transport".into()),
    }
    let mut missing = variant.clone();
    missing.selector = VariantSelector::TorrentHash {
        info_hash: "0".repeat(40),
    };
    assert!(matches!(
        source.resolve(&missing).await,
        Err(Error::VariantUnavailable)
    ));
    let empty = source
        .search("qbuttgeneratedunlikelyemptysource", Category::Software)
        .await?;
    assert!(empty.is_empty());
    println!(
        "PASS rutor-live search={} publication={} category=software exact_magnet=ok changed_hash=rejected valid_empty=ok torrent_payload=not_tested",
        works.len(),
        work.reference.id
    );
    Ok(())
}
