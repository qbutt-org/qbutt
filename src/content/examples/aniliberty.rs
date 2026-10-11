//! Bounded read-only integration check. Fetches catalog metadata and resolves
//! exact descriptors; it does not download media or add a torrent.

use std::collections::HashSet;

use qbutt_content::{AniLiberty, Error, ResolvedContent, VariantRef, VariantSelector};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = std::env::args().nth(1).unwrap_or_else(|| "Frieren".into());
    let catalog = AniLiberty::new()?;
    let works = catalog.search(&query).await?;
    let selected = works.first().ok_or("no catalog results")?;
    let work = catalog.inspect(&selected.reference).await?;
    assert_eq!(work.reference, selected.reference);
    assert!(!work.needs_inspection);
    assert!(!work.variants.is_empty());
    assert_eq!(
        work.variants
            .iter()
            .map(|variant| &variant.reference)
            .collect::<HashSet<_>>()
            .len(),
        work.variants.len()
    );

    let torrent = work
        .variants
        .iter()
        .find(|variant| matches!(variant.reference.selector, VariantSelector::Torrent { .. }))
        .ok_or("no torrent variant in this integration fixture")?;
    match catalog.resolve(&torrent.reference).await? {
        ResolvedContent::Torrent {
            reference, magnet, ..
        } => {
            assert_eq!(reference, torrent.reference);
            assert_eq!(magnet.scheme(), "magnet");
        }
        _ => return Err("torrent selector resolved as a different transport".into()),
    }

    let video = work
        .variants
        .iter()
        .find(|variant| matches!(variant.reference.selector, VariantSelector::Episode { .. }))
        .ok_or("no video variant in this integration fixture")?;
    match catalog.resolve(&video.reference).await? {
        ResolvedContent::DirectVideo { reference, url, .. } => {
            assert_eq!(reference, video.reference);
            assert!(matches!(url.scheme(), "http" | "https"));
        }
        _ => return Err("episode selector resolved as a different transport".into()),
    }

    let missing = VariantRef {
        publication: work.reference.clone(),
        selector: VariantSelector::Torrent { id: u64::MAX },
    };
    assert!(matches!(
        catalog.resolve(&missing).await,
        Err(Error::VariantUnavailable)
    ));

    // Caller cancellation smoke: dropping the future stops awaiting this result.
    // This does not assert that the OS's blocking DNS lookup has already drained.
    let mut cancelled = Box::pin(catalog.search(&query));
    let cancelled_before_completion = tokio::select! {
        biased;
        _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => true,
        _ = &mut cancelled => false,
    };
    drop(cancelled);
    assert!(
        cancelled_before_completion,
        "request finished before cancellation could be exercised"
    );

    println!(
        "search={} inspected={} variants={} exact_torrent=ok exact_video_descriptor=ok missing_variant=rejected",
        works.len(),
        work.reference.id,
        work.variants.len()
    );
    Ok(())
}
