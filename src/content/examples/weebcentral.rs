// SPDX-License-Identifier: GPL-3.0-or-later
use qbutt_content::{
    ChapterRef, Error, Library, PublicationRef, SharedWork, Source, WeebCentral, WorkGroup,
};
use sha2::{Digest, Sha256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = WeebCentral::new()?;
    let reference = PublicationRef {
        source: Source::WeebCentral,
        id: "01J76XY7E9FNDZ1DBBM6PBJPFK".into(),
    };
    let found = source
        .search("One Piece")
        .await
        .map_err(|error| format!("search: {error}"))?;
    assert!(found.iter().any(|work| work.reference == reference));
    assert!(
        source
            .search("qbutt-nonexistent-72618")
            .await
            .map_err(|error| format!("empty search: {error}"))?
            .is_empty()
    );
    let work = source
        .inspect(&reference)
        .await
        .map_err(|error| format!("inspect: {error}"))?;
    assert_eq!(work.title, "One Piece");
    assert_eq!(work.variants.len(), 1);
    assert_eq!(work.variants[0].languages, ["en"]);
    let chapters = source
        .chapters(&work.variants[0].reference)
        .await
        .map_err(|error| format!("chapters: {error}"))?;
    let chapter = chapters.get(1).ok_or("nonfirst chapter missing")?;
    assert_eq!(chapter.number.as_deref(), Some("2"));
    let pages = source
        .chapter_pages(&chapter.reference)
        .await
        .map_err(|error| format!("chapter pages: {error}"))?;
    assert!(pages.reference == chapter.reference);
    let bytes = source
        .image_bytes(&pages.images[0])
        .await
        .map_err(|error| format!("image: {error}"))?;
    assert!(!bytes.is_empty());
    let mut foreign = ChapterRef {
        variant: work.variants[0].reference.clone(),
        id: chapter.reference.id.clone(),
    };
    foreign.variant.publication.id = "01J76XY7E827QQQT0ERKCGH4CD".into();
    assert!(matches!(
        source.chapter_pages(&foreign).await,
        Err(Error::VariantUnavailable)
    ));

    let shared = SharedWork::new(
        vec![reference.clone()],
        Some(work.variants[0].reference.clone()),
    )?;
    assert_eq!(SharedWork::from_url(&shared.to_url()?)?, shared);

    let profile = std::env::temp_dir().join(format!("qbutt-weebcentral-{}", std::process::id()));
    std::fs::create_dir(&profile)?;
    let mut library = Library::open(&profile)?;
    library.set_favorite(&WorkGroup::single(work.clone()), true)?;
    drop(library);
    let library = Library::open(&profile)?;
    assert_eq!(
        library.get(&reference)?.unwrap().group.anchor_work(),
        Some(&work)
    );
    drop(library);
    std::fs::remove_dir_all(profile)?;
    println!(
        "PASS WeebCentral live search={} chapters={} nonfirst={} pages={} image_bytes={} sha256={:x} empty_search=ok foreign_publication=rejected public_share=ok library_reopen=ok",
        found.len(),
        chapters.len(),
        chapter.reference.id,
        pages.images.len(),
        bytes.len(),
        Sha256::digest(&bytes)
    );
    Ok(())
}
