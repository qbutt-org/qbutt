//! Read-only acceptance against the public catalog. No torrent payload is fetched.

use qbutt_content::{
    Category, Error, LimeTorrents, PublicationRef, ResolvedContent, Source, VariantSelector,
};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let source = LimeTorrents::new()?;
    let results = source.search("Big Buck Bunny", Category::Movies).await?;
    let found = results
        .iter()
        .find(|work| work.reference.id.ends_with("-torrent-9616433.html"))
        .ok_or("known public-domain release missing from search")?;
    assert!(found.needs_inspection && found.variants.is_empty());

    let release = source.inspect(&found.reference).await?;
    assert_eq!(release.reference, found.reference);
    assert_eq!(release.category, Category::Movies);
    assert!(!release.needs_inspection);
    assert_eq!(release.variants.len(), 1);
    let variant = &release.variants[0];

    let ResolvedContent::Torrent {
        reference, magnet, ..
    } = source.resolve(&variant.reference).await?
    else {
        return Err("unexpected transport".into());
    };
    assert_eq!(reference, variant.reference);
    assert!(magnet.query_pairs().any(|(name, value)| {
        name == "xt"
            && value
                .eq_ignore_ascii_case(&format!("urn:btih:{}", variant.info_hash.as_ref().unwrap()))
    }));

    let mut changed = variant.reference.clone();
    changed.selector = VariantSelector::TorrentHash {
        info_hash: "0".repeat(40),
    };
    assert!(matches!(
        source.resolve(&changed).await,
        Err(Error::VariantUnavailable)
    ));

    let missing = PublicationRef {
        source: Source::LimeTorrents,
        id: "qbutt-nonexistent-torrent-9999999999.html".into(),
    };
    assert!(matches!(
        source.inspect(&missing).await,
        Err(Error::VariantUnavailable)
    ));
    assert!(
        source
            .search("qbutt-nonexistent-fixture-907376", Category::Movies)
            .await?
            .is_empty()
    );

    println!(
        "PASS limetorrents_live search={} publication={} hash={} exact_resolve=ok changed_hash=rejected missing=404 empty=ok payload=none",
        results.len(),
        release.reference.id,
        variant.info_hash.as_deref().unwrap()
    );
    Ok(())
}
