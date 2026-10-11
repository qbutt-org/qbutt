//! Bounded, read-only official API acceptance. Downloads one original page into
//! memory, without opening a player, publishing images or touching a live profile.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::{
    Category, ChapterRef, Error, Library, LibraryError, MangaDex, PublicationRef, Source,
    Transport, VariantPreferences, VariantRef, VariantSelector,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = std::env::args().nth(1).unwrap_or_else(|| "Frieren".into());
    let language = std::env::args().nth(2).unwrap_or_else(|| "ru".into());
    let catalog = MangaDex::new()?;
    let invalid = PublicationRef {
        source: Source::MangaDex,
        id: "B0B721FF-C388-4486-AA0F-C2B0BB321512".into(),
    };
    assert!(matches!(
        catalog.inspect(&invalid, &language).await,
        Err(Error::InvalidReference)
    ));
    let foreign = VariantRef {
        publication: PublicationRef {
            source: Source::AniLiberty,
            id: "1".into(),
        },
        selector: VariantSelector::Torrent { id: 1 },
    };
    assert!(matches!(
        catalog.chapters(&foreign).await,
        Err(Error::InvalidReference)
    ));
    let works = catalog.search(&query, &language).await?;
    let selected = works.first().ok_or("no official API search results")?;
    assert_eq!(selected.category, Category::Manga);
    assert!(selected.needs_inspection);
    assert!(selected.variants.is_empty());
    let work = catalog.inspect(&selected.reference, &language).await?;
    assert_eq!(work.reference, selected.reference);
    assert!(!work.needs_inspection);
    assert!(
        work.variants
            .iter()
            .all(|variant| variant.transport == Transport::Manga)
    );
    assert_eq!(
        work.variants.len(),
        work.variants
            .iter()
            .map(|variant| &variant.reference)
            .collect::<HashSet<_>>()
            .len()
    );
    let variant = work
        .variants
        .first()
        .ok_or("manga has no readable translation branches")?;
    let chapters = catalog.chapters(&variant.reference).await?;
    assert!(
        chapters
            .iter()
            .all(|chapter| chapter.reference.variant == variant.reference)
    );
    assert_eq!(
        chapters.len(),
        chapters
            .iter()
            .map(|chapter| &chapter.reference.id)
            .collect::<HashSet<_>>()
            .len()
    );
    let chapter = chapters
        .first()
        .ok_or("translation has no readable chapters")?;
    let pages = catalog.chapter_pages(&chapter.reference).await?;
    assert!(pages.reference == chapter.reference);
    assert_eq!(pages.images.len(), chapter.pages as usize);
    let bytes = catalog.image_bytes(&pages.images[0]).await?;
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || bytes.starts_with(b"\xff\xd8\xff")
            || bytes.starts_with(b"RIFF")
            || bytes.starts_with(b"GIF8")
    );

    let mut missing = variant.reference.clone();
    missing.selector = VariantSelector::MangaTranslation {
        language: "zz".into(),
        groups: Vec::new(),
    };
    assert!(matches!(
        catalog.chapters(&missing).await,
        Err(Error::VariantUnavailable)
    ));
    let wrong_branch = ChapterRef {
        variant: missing,
        id: chapter.reference.id.clone(),
    };
    assert!(matches!(
        catalog.chapter_pages(&wrong_branch).await,
        Err(Error::VariantUnavailable)
    ));

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile =
        std::env::temp_dir().join(format!("qbutt-mangadex-{}-{nonce}", std::process::id()));
    let mut library = Library::open(&profile)?;
    library.set_favorite(&work, true)?;
    let preferences = VariantPreferences {
        selected: Some(variant.reference.clone()),
        ..Default::default()
    };
    library.save_preferences(&work, preferences.clone())?;
    let mut wrong_transport = work.clone();
    wrong_transport.variants[0].transport = Transport::Torrent;
    assert!(matches!(
        library.set_favorite(&wrong_transport, false),
        Err(LibraryError::InvalidEntry(_))
    ));
    let mut wrong_source = work.clone();
    wrong_source.variants[0].reference.selector = VariantSelector::Torrent { id: 1 };
    assert!(matches!(
        library.set_favorite(&wrong_source, false),
        Err(LibraryError::InvalidEntry(_))
    ));
    let mut wrong_category = work.clone();
    wrong_category.category = Category::Anime;
    assert!(matches!(
        library.set_favorite(&wrong_category, false),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert!(library.get(&work.reference)?.unwrap().favorite);
    drop(library);
    let library = Library::open(&profile)?;
    let saved = library
        .get(&work.reference)?
        .ok_or("saved manga was lost")?;
    assert_eq!(saved.work, work);
    assert_eq!(saved.preferences, preferences);
    assert!(saved.last_opened.is_none());
    drop(library);
    std::fs::remove_dir_all(&profile)?;

    println!(
        "PASS mangadex search={} branches={} chapters={} pages={} verified_image_bytes={} exact_refs=ok missing_branch=rejected foreign_chapter_branch=rejected library_reopen=ok source_bound_library=ok history_absent=ok",
        works.len(),
        work.variants.len(),
        chapters.len(),
        pages.images.len(),
        bytes.len()
    );
    Ok(())
}
