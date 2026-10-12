use std::error::Error;

use qbutt_content::{LibSession, MangaLib, PublicationRef, Source, VariantSelector};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let source = MangaLib::new()?;
    let mut session = LibSession::anonymous(Source::MangaLib)?;
    let reference = PublicationRef {
        source: Source::MangaLib,
        id: "5432--yotsubaamp".into(),
    };
    let search = source.search("Yotsuba", &mut session).await?;
    let listed = search
        .iter()
        .find(|work| work.reference == reference)
        .ok_or("known publication missing from live search")?;
    assert!(listed.needs_inspection);
    assert_eq!(
        listed.url.as_str(),
        "https://mangalib.me/ru/manga/5432--yotsubaamp"
    );
    let work = source.inspect(&reference, &mut session).await?;
    assert!(!work.needs_inspection);
    let variant = work.variants.first().ok_or("translation branch missing")?;
    let chapters = source.chapters(&variant.reference, &mut session).await?;
    assert_eq!(chapters.first().unwrap().reference.id, "161943");
    let pages = source
        .chapter_pages(&chapters[0].reference, &mut session)
        .await?;
    assert_eq!(pages.reference, chapters[0].reference);
    assert_eq!(pages.images.len(), 53);
    let bytes = source.image_bytes(&pages.images[0], &mut session).await?;
    assert!(bytes.starts_with(&[0xff, 0xd8, 0xff]));
    assert!(bytes.len() > 100_000);
    let mut unavailable = variant.reference.clone();
    unavailable.selector = VariantSelector::LibTranslation {
        branch: Some(u64::MAX),
    };
    assert!(matches!(
        source.chapters(&unavailable, &mut session).await,
        Err(qbutt_content::Error::VariantUnavailable)
    ));
    let mut unavailable = chapters[0].reference.clone();
    unavailable.id = u64::MAX.to_string();
    assert!(matches!(
        source.chapter_pages(&unavailable, &mut session).await,
        Err(qbutt_content::Error::VariantUnavailable)
    ));
    assert!(source.search("\n", &mut session).await.is_err());
    println!(
        "PASS MangaLib native search/inspect/exact branch/chapter/JPEG; chapters={} variants={} pages={} image_bytes={} absent_branch_and_upload=rejected",
        chapters.len(),
        work.variants.len(),
        pages.images.len(),
        bytes.len()
    );
    Ok(())
}
