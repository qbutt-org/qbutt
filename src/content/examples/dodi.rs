//! Bounded metadata-only verification. No game payload is downloaded or run.

use qbutt_content::{Category, Dodi, ResolvedContent};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let resolve = std::env::args().any(|argument| argument == "--resolve");
    let query = "Devil May Cry";
    let provider = Dodi::new()?;
    let works = provider.search(query).await?;
    let selected = works
        .iter()
        .find(|work| work.reference.id == "2643--devil-may-cry-5")
        .ok_or("the expected DODI publication was not found")?;
    assert!(selected.needs_inspection && selected.variants.is_empty());
    let work = provider.inspect(&selected.reference).await?;
    assert_eq!(work.reference, selected.reference);
    assert_eq!(work.category, Category::Games);
    assert!(!work.needs_inspection);
    println!(
        "PASS dodi-live metadata_search={} exact_publication={} title={}",
        works.len(),
        work.reference.id,
        work.title
    );
    assert!(
        provider
            .search("qbuttgeneratedmissing947613")
            .await?
            .is_empty()
    );
    if resolve {
        let selected = work
            .variants
            .first()
            .ok_or("no supported DODI torrent mirror")?;
        let ResolvedContent::TorrentFile {
            reference,
            metadata,
        } = provider.resolve(&selected.reference).await?
        else {
            return Err("wrong torrent acquisition transport".into());
        };
        assert_eq!(reference, selected.reference);
        if let Some(path) = std::env::var_os("QBUTT_DODI_METADATA_OUTPUT") {
            std::fs::write(path, metadata.bytes())?;
        }
        println!(
            "PASS real_metadata_bytes={} exact_reference=ok native_libtorrent_acceptance=unverified",
            metadata.bytes().len()
        );
    }
    Ok(())
}
