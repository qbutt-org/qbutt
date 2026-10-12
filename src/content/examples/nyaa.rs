//! Read-only public catalog acceptance. No torrent is added or downloaded.

use qbutt_content::{Category, Error, Nyaa, ResolvedContent, VariantSelector};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let source = Nyaa::new()?;
    let query = std::env::args().nth(1).unwrap_or_else(|| "Frieren".into());
    let works = source.search(&query, Category::Anime).await?;
    let found = works.first().ok_or("no public catalog results")?;
    let work = source.inspect(&found.reference).await?;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.category, Category::Anime);
    assert_eq!(work.variants[0].reference, found.variants[0].reference);
    assert!(!work.needs_inspection);
    match source.resolve(&found.variants[0].reference).await? {
        ResolvedContent::Torrent {
            reference, magnet, ..
        } => {
            assert_eq!(reference, found.variants[0].reference);
            assert_eq!(magnet.scheme(), "magnet");
            assert!(magnet.query_pairs().any(|(name, value)| {
                name == "xt"
                    && value == format!("urn:btih:{}", work.variants[0].info_hash.as_ref().unwrap())
            }));
        }
        _ => return Err("wrong transport".into()),
    }
    let mut absent = found.variants[0].reference.clone();
    absent.selector = VariantSelector::TorrentHash {
        info_hash: "0".repeat(40),
    };
    assert!(matches!(
        source.resolve(&absent).await,
        Err(Error::VariantUnavailable)
    ));
    assert!(
        source
            .search("qbuttgeneratedunlikelyemptyresult20261012", Category::Anime)
            .await?
            .is_empty()
    );
    println!(
        "PASS nyaa_live search={} publication={} exact_hash=ok metadata=source_reported changed_hash=rejected empty=ok torrent_payload=not_downloaded",
        works.len(),
        work.reference.id
    );
    Ok(())
}
