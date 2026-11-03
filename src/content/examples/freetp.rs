//! Read-only live FreeTP acceptance. It fetches small `.torrent` metadata only.

use qbutt_content::{Category, Error, FreeTP, ResolvedContent, VariantSelector};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let source = FreeTP::new()?;
    for query in ["Acon Acoustica 8", "Re:Zero", "occt"] {
        println!("CHECK freetp_live query={query:?}");
        assert!(source.search(query, Category::Games).await?.is_empty());
        println!("PASS freetp_live query={query:?} results=0 game_payload=none");
    }
    let mut args = std::env::args().skip(1);
    let query = args.next().unwrap_or_else(|| "КЛЕТЬ".into());
    let publication_id = args.next();
    println!("CHECK freetp_live query={query:?}");
    let found = source.search(&query, Category::Games).await?;
    let publication = match publication_id.as_deref() {
        Some(id) => found.iter().find(|work| work.reference.id == id),
        None => found.first(),
    }
    .ok_or("no matching FreeTP game result")?;
    assert!(publication.needs_inspection && publication.variants.is_empty());

    let work = source.inspect(&publication.reference).await?;
    assert_eq!(work.reference, publication.reference);
    assert_eq!(work.category, Category::Games);
    assert!(!work.needs_inspection && !work.variants.is_empty());
    for variant in &work.variants {
        let ResolvedContent::TorrentFile {
            reference,
            metadata,
        } = source.resolve(&variant.reference).await?
        else {
            return Err("FreeTP returned a non-torrent variant".into());
        };
        assert_eq!(reference, variant.reference);
        assert!(!metadata.bytes().is_empty());
        println!(
            "PASS freetp_variant publication={} info_hash={} metadata_bytes={}",
            reference.publication.id,
            variant.info_hash.as_deref().unwrap(),
            metadata.bytes().len()
        );
    }

    let mut missing = work.variants[0].reference.clone();
    missing.selector = VariantSelector::TorrentHash {
        info_hash: "0".repeat(40),
    };
    assert!(matches!(
        source.resolve(&missing).await,
        Err(Error::VariantUnavailable)
    ));
    println!(
        "PASS freetp_live search={} publication={} variants={} exact_hash=ok missing_hash=rejected empty=ok game_payload=none",
        found.len(),
        work.reference.id,
        work.variants.len()
    );
    Ok(())
}
