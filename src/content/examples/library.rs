//! Isolated process integration scenario with generated public metadata only.
//! No network, media download, live profile or unit-test harness is involved.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::{
    Category, Library, LibraryError, LibraryView, PublicationRef, Source, Transport, Variant,
    VariantFilters, VariantPreferences, VariantRef, VariantSelector, VideoQuality, Work,
};
use rusqlite::Connection;
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
        let profile = args.next().ok_or("missing isolated profile")?;
        verify_reopened(Path::new(&profile))?;
        return Ok(());
    }

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!("qbutt-library-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root)?;
    let result = exercise(&root);
    std::fs::remove_dir_all(&root)?;
    result?;
    println!(
        "reopen=ok full_variants=ok exact_selection=ok filters=ok favorite_history=independent rejected_update=unchanged sqlite_abort=unchanged corrupt_metadata=rejected unsupported_schema=rejected unrelated_data=preserved"
    );
    Ok(())
}

fn exercise(root: &Path) -> Result<()> {
    let profile = root.join("profile");
    std::fs::create_dir(&profile)?;
    let unrelated = profile.join("downloaded-payload.txt");
    std::fs::write(&unrelated, b"generated unrelated data")?;
    let legacy = profile.join("library.sqlite");
    std::fs::write(&legacy, b"generated legacy placeholder")?;

    let work = fixture();
    let mut library = Library::open(&profile)?;
    assert!(library.entries(LibraryView::All)?.is_empty());
    library.set_favorite(&work, true)?;
    library.save_preferences(&work, preferences(&work, 0))?;
    assert!(library.entries(LibraryView::History)?.is_empty());
    library.record_opened(
        &work,
        work.variants[0].reference.clone(),
        preferences(&work, 0),
    )?;
    library.save_preferences(&work, preferences(&work, 1))?;
    drop(library);

    let status = Command::new(std::env::current_exe()?)
        .arg("--reopen")
        .arg(&profile)
        .status()?;
    assert!(
        status.success(),
        "the second process failed to reopen the library"
    );

    let mut library = Library::open(&profile)?;
    let mut partial = work.clone();
    partial.title = "Refreshed generated title".into();
    partial.variants.truncate(1);
    library.set_favorite(&partial, false)?;
    let entry = library.get(&work.reference)?.ok_or("missing saved work")?;
    assert!(!entry.favorite);
    assert_eq!(entry.work.title, partial.title);
    assert_eq!(entry.work.variants, work.variants);
    assert_eq!(entry.preferences, preferences(&work, 1));
    assert_eq!(
        entry.last_opened.as_ref().unwrap().variant,
        work.variants[0].reference
    );
    assert_eq!(library.entries(LibraryView::History)?.len(), 1);
    assert!(library.entries(LibraryView::Favorites)?.is_empty());

    library.set_favorite(&work, true)?;
    let entry = library
        .remove_history(&work.reference)?
        .ok_or("missing saved work")?;
    assert!(entry.favorite);
    assert!(entry.last_opened.is_none());
    assert_eq!(entry.preferences, preferences(&work, 1));
    assert_eq!(entry.work.variants, work.variants);
    assert!(library.entries(LibraryView::History)?.is_empty());
    assert_eq!(library.entries(LibraryView::Favorites)?.len(), 1);

    let before = library.get(&work.reference)?.unwrap();
    let mut invalid = preferences(&work, 0);
    invalid.selected.as_mut().unwrap().publication.id = "another-publication".into();
    assert!(matches!(
        library.save_preferences(&partial, invalid),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);

    let missing = VariantRef {
        publication: work.reference.clone(),
        selector: VariantSelector::Torrent { id: u64::MAX },
    };
    assert!(matches!(
        library.record_opened(&partial, missing, preferences(&work, 0)),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);

    let mut private = work.clone();
    private.cover = Some(Url::parse(
        "https://anilibria.top/poster.jpg?token=generated-secret",
    )?);
    assert!(matches!(
        library.set_favorite(&private, false),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);

    let database = profile.join("content-library.sqlite");
    let raw = Connection::open(&database)?;
    raw.execute_batch(
        "CREATE TRIGGER reject_library_update BEFORE UPDATE ON library
         BEGIN SELECT RAISE(ABORT, 'generated integration fault'); END;",
    )?;
    assert!(matches!(
        library.set_favorite(&partial, false),
        Err(LibraryError::Sqlite(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);
    raw.execute_batch("DROP TRIGGER reject_library_update;")?;
    drop(library);

    raw.pragma_update(None, "user_version", 2)?;
    assert!(matches!(
        Library::open(&profile),
        Err(LibraryError::UnsupportedSchema(2))
    ));
    assert_eq!(
        raw.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
        2
    );
    raw.pragma_update(None, "user_version", 1)?;

    raw.execute("UPDATE library SET entry = ?1", ["not JSON"])?;
    let library = Library::open(&profile)?;
    assert!(matches!(
        library.get(&work.reference),
        Err(LibraryError::Metadata(_))
    ));
    assert!(matches!(
        library.entries(LibraryView::Favorites),
        Err(LibraryError::Metadata(_))
    ));
    let raw_metadata: String = raw.query_row("SELECT entry FROM library", [], |row| row.get(0))?;
    assert_eq!(raw_metadata, "not JSON");
    drop(library);

    let mut corrupt = before.clone();
    corrupt
        .preferences
        .selected
        .as_mut()
        .unwrap()
        .publication
        .id = "another-publication".into();
    let metadata = serde_json::to_string(&corrupt)?;
    raw.execute("UPDATE library SET entry = ?1", [&metadata])?;
    let mut library = Library::open(&profile)?;
    assert!(matches!(
        library.get(&work.reference),
        Err(LibraryError::CorruptEntry(_))
    ));
    assert!(matches!(
        library.set_favorite(&work, false),
        Err(LibraryError::CorruptEntry(_))
    ));
    assert_eq!(
        raw.query_row("SELECT entry FROM library", [], |row| row
            .get::<_, String>(0))?,
        metadata
    );
    drop(library);
    drop(raw);

    let foreign_profile = root.join("foreign-profile");
    std::fs::create_dir(&foreign_profile)?;
    let foreign = Connection::open(foreign_profile.join("content-library.sqlite"))?;
    foreign.execute_batch(
        "CREATE TABLE unrelated (value TEXT); INSERT INTO unrelated VALUES ('generated data');",
    )?;
    assert!(matches!(
        Library::open(&foreign_profile),
        Err(LibraryError::ForeignDatabase)
    ));
    assert_eq!(
        foreign.query_row("SELECT value FROM unrelated", [], |row| row
            .get::<_, String>(0))?,
        "generated data"
    );
    drop(foreign);

    assert_eq!(std::fs::read(unrelated)?, b"generated unrelated data");
    assert_eq!(std::fs::read(legacy)?, b"generated legacy placeholder");
    Ok(())
}

fn verify_reopened(profile: &Path) -> Result<()> {
    let work = fixture();
    let library = Library::open(profile)?;
    let entry = library
        .get(&work.reference)?
        .ok_or("missing reopened work")?;
    assert_eq!(entry.work, work);
    assert_eq!(entry.preferences, preferences(&work, 1));
    assert!(entry.favorite);
    assert_eq!(
        entry.last_opened.as_ref().unwrap().variant,
        work.variants[0].reference
    );
    assert!(entry.last_opened.as_ref().unwrap().at_unix_millis > 0);
    assert_eq!(
        library.entries(LibraryView::Favorites)?,
        vec![entry.clone()]
    );
    assert_eq!(library.entries(LibraryView::History)?, vec![entry]);
    println!(
        "second_process: saved Work, all variants, choices, favorite and exact history restored"
    );
    Ok(())
}

fn preferences(work: &Work, index: usize) -> VariantPreferences {
    VariantPreferences {
        selected: Some(work.variants[index].reference.clone()),
        filters: VariantFilters {
            episode: Some("1".into()),
            translation: Some("Generated voice".into()),
            source: Some(Source::AniLiberty),
            language: Some("ru".into()),
            player: Some("Generated player".into()),
            transport: Some(work.variants[index].transport),
        },
    }
}

fn fixture() -> Work {
    let reference = PublicationRef {
        source: Source::AniLiberty,
        id: "1".into(),
    };
    let selectors = [
        VariantSelector::Episode {
            id: "generated-episode".into(),
            quality: VideoQuality::P1080,
        },
        VariantSelector::Torrent { id: 42 },
        VariantSelector::Rutube {
            episode_id: "generated-episode".into(),
        },
    ];
    let variants = selectors
        .into_iter()
        .map(|selector| {
            let transport = if matches!(selector, VariantSelector::Torrent { .. }) {
                Transport::Torrent
            } else {
                Transport::Video
            };
            Variant {
                reference: VariantRef {
                    publication: reference.clone(),
                    selector,
                },
                title: None,
                transport,
                episode: Some("1".into()),
                quality: Some("1080p".into()),
                translation: Some("Generated voice".into()),
                languages: vec!["ru".into(), "en".into()],
                player: Some("Generated player".into()),
                size: Some(1024),
                seeders: Some(3),
                info_hash: None,
                reported_video_ads: false,
            }
        })
        .collect();
    Work {
        reference,
        title: "Generated catalog fixture".into(),
        category: Category::Anime,
        url: Url::parse("https://anilibria.top/anime/releases/release/generated-fixture").unwrap(),
        cover: Some(Url::parse("https://anilibria.top/generated-poster.jpg").unwrap()),
        description: Some("Legal generated metadata without acquisition URLs".into()),
        year: Some(2026),
        aliases: vec!["Generated alias".into()],
        identifiers: BTreeMap::from([("generated".into(), "1".into())]),
        variants,
        needs_inspection: false,
    }
}
