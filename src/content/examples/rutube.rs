//! Bounded public metadata and manifest check using a Blender open movie.
//! No media segments, live application profile or player are opened.

use qbutt_content::{Category, ResolvedContent, Rutube};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let source = Rutube::new()?;
    let works = source.search("Big Buck Bunny", Category::Movies).await?;
    let found = works.first().ok_or("no free movie publication found")?;
    let work = source.inspect(&found.reference).await?;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.variants[0].reference, found.variants[0].reference);
    assert_eq!(work.category, Category::Movies);
    assert!(found.variants[0].quality.is_none());
    assert!(work.url.query().is_none());
    assert!(!work.needs_inspection);
    let ResolvedContent::DirectVideo {
        reference,
        quality,
        url,
        ..
    } = source.resolve(&found.variants[0].reference).await?
    else {
        return Err("wrong playback descriptor".into());
    };
    assert_eq!(reference, found.variants[0].reference);
    assert_eq!(
        work.variants[0].quality,
        Some(format!("{}p", quality.height()))
    );
    assert_eq!(url.scheme(), "https");
    println!(
        "PASS rutube_live search={} publication={} exact_id=ok free_access=ok manifest_quality={}p media_payload=not_downloaded native_playback=unverified",
        works.len(),
        work.reference.id,
        quality.height()
    );
    Ok(())
}
