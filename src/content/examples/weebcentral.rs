// SPDX-License-Identifier: GPL-3.0-or-later
//! Usage: weebcentral [page-index] [publication-id] [chapter-id]
use qbutt_content::{Error, Library, PublicationRef, SharedWork, Source, WeebCentral, WorkGroup};
use sha2::{Digest, Sha256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let page_index = args
        .next()
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    let source = WeebCentral::new()?;
    let reference = PublicationRef {
        source: Source::WeebCentral,
        id: args
            .next()
            .unwrap_or_else(|| "01J76XY7E9FNDZ1DBBM6PBJPFK".into()),
    };
    let selected_chapter = args.next();
    if args.next().is_some() {
        return Err("usage: weebcentral [page-index] [publication-id] [chapter-id]".into());
    }
    let work = source
        .inspect(&reference)
        .await
        .map_err(|error| format!("inspect: {error}"))?;
    let found = source
        .search(&work.title)
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
    assert_eq!(work.variants.len(), 1);
    assert_eq!(work.variants[0].languages, ["en"]);
    let chapters = source
        .chapters(&work.variants[0].reference)
        .await
        .map_err(|error| format!("chapters: {error}"))?;
    let (chapter_index, chapter) = if let Some(id) = selected_chapter {
        chapters
            .iter()
            .enumerate()
            .find(|(_, chapter)| chapter.reference.id == id)
            .ok_or("selected chapter missing")?
    } else {
        (1, chapters.get(1).ok_or("nonfirst chapter missing")?)
    };
    let pages = source
        .chapter_pages(&chapter.reference)
        .await
        .map_err(|error| format!("chapter pages: {error}"))?;
    assert!(pages.reference == chapter.reference);
    let image = pages
        .images
        .get(page_index)
        .ok_or("selected page missing")?;
    let bytes = source
        .image_bytes(image)
        .await
        .map_err(|error| format!("image: {error}"))?;
    assert!(!bytes.is_empty());
    let mut foreign = chapter.reference.clone();
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
        "PASS WeebCentral live search={} chapters={} chapter={} chapter_index={} pages={} page_index={} image_bytes={} sha256={:x} empty_search=ok foreign_publication=rejected public_share=ok library_reopen=ok",
        found.len(),
        chapters.len(),
        chapter.reference.id,
        chapter_index,
        pages.images.len(),
        page_index,
        bytes.len(),
        Sha256::digest(&bytes)
    );
    Ok(())
}
