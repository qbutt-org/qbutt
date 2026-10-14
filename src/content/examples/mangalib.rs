use std::error::Error;

use qbutt_content::{LibSession, MangaLib, PublicationRef, Source, VariantSelector};
use sha2::{Digest, Sha256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let (publication, query, chapter_index) = match arguments.as_slice() {
        [] => ("5432--yotsubaamp", "Yotsuba", 0),
        [publication, query, index] | [publication, query, index, _] => (
            publication.as_str(),
            query.as_str(),
            index.parse::<usize>()?,
        ),
        _ => return Err("usage: mangalib [publication-id query zero-based-chapter-index [zero-based-page-index]]".into()),
    };
    let page_index = arguments
        .get(3)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    let source = MangaLib::new()?;
    let mut session = LibSession::anonymous(Source::MangaLib)?;
    let reference = PublicationRef {
        source: Source::MangaLib,
        id: publication.into(),
    };
    let search = source.search(query, &mut session).await?;
    let listed = search
        .iter()
        .find(|work| work.reference == reference)
        .ok_or("known publication missing from live search")?;
    assert!(listed.needs_inspection);
    assert_eq!(
        listed.url.as_str(),
        format!("https://mangalib.me/ru/manga/{publication}")
    );
    let work = source.inspect(&reference, &mut session).await?;
    assert!(!work.needs_inspection);
    let variant = work.variants.first().ok_or("translation branch missing")?;
    let chapters = source.chapters(&variant.reference, &mut session).await?;
    let chapter = chapters
        .get(chapter_index)
        .ok_or("selected chapter missing")?;
    let pages = source
        .chapter_pages(&chapter.reference, &mut session)
        .await?;
    assert_eq!(pages.reference, chapter.reference);
    assert!(!pages.images.is_empty());
    let image = pages
        .images
        .get(page_index)
        .ok_or("selected page missing")?;
    let bytes = source.image_bytes(image, &session).await?;
    let image_format = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpeg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else {
        return Err("unrecognized page image signature".into());
    };
    if arguments.is_empty() {
        assert_eq!(chapter.reference.id, "161943");
        assert_eq!(pages.images.len(), 53);
        assert_eq!(image_format, "jpeg");
        assert!(bytes.len() > 100_000);
    }
    let mut unavailable = variant.reference.clone();
    unavailable.selector = VariantSelector::LibTranslation {
        branch: Some(u64::MAX),
    };
    assert!(matches!(
        source.chapters(&unavailable, &mut session).await,
        Err(qbutt_content::Error::VariantUnavailable)
    ));
    let mut unavailable = chapter.reference.clone();
    unavailable.id = u64::MAX.to_string();
    assert!(matches!(
        source.chapter_pages(&unavailable, &mut session).await,
        Err(qbutt_content::Error::VariantUnavailable)
    ));
    assert!(source.search("\n", &mut session).await.is_err());
    println!(
        "{}",
        serde_json::json!({
            "status": "passed",
            "anonymous": true,
            "publication": reference.id,
            "variant": variant.reference,
            "chapter": chapter.reference,
            "chapterIndex": chapter_index,
            "pageIndex": page_index,
            "chapters": chapters.len(),
            "variants": work.variants.len(),
            "pages": pages.images.len(),
            "imageFormat": image_format,
            "imageBytes": bytes.len(),
            "imageSha256": format!("{:x}", Sha256::digest(&bytes)),
            "absentBranchAndUpload": "rejected",
            "nativeReader": "not checked by this provider example"
        })
    );
    Ok(())
}
