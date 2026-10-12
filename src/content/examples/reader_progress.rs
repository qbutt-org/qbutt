//! Generated legal metadata, real SQLite transactions and separate-process
//! reopen. Rendering confirmation belongs to the Qt reader acceptance path.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::{
    Category, ChapterRef, Library, LibraryError, LibraryView, PublicationRef, ReaderPosition,
    ReaderProgress, Source, Transport, Variant, VariantFilters, VariantPreferences, VariantRef,
    VariantSelector, Work,
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
    let root = std::env::temp_dir().join(format!("qbutt-reader-progress-{nonce}"));
    std::fs::create_dir(&root)?;
    let result = exercise(&root);
    std::fs::remove_dir_all(&root)?;
    result?;
    println!("PASS reader_progress reopen=ok full_work=ok exact_branch=ok preferences=independent");
    println!(
        "finite_bounds=ok invalid_refs=rejected sqlite_abort=atomic corrupt_progress=rejected"
    );
    Ok(())
}

fn exercise(profile: &Path) -> Result<()> {
    let work = fixture();
    let progress = position(&work);
    let preferences = choices(&work, 0);
    let mut library = Library::open(profile)?;
    library.set_favorite(&qbutt_content::WorkGroup::single(work.clone()), true)?;
    library.save_preferences(
        &qbutt_content::WorkGroup::single(work.clone()),
        preferences.clone(),
    )?;
    assert!(library.entries(LibraryView::History)?.is_empty());
    for offset in [0.0, 1.0] {
        let mut boundary = progress.clone();
        boundary.position = ReaderPosition::Page {
            page: 3,
            scroll_offset: offset,
        };
        library.save_reader_progress(
            &qbutt_content::WorkGroup::single(work.clone()),
            preferences.clone(),
            boundary,
        )?;
    }
    let saved = library.save_reader_progress(
        &qbutt_content::WorkGroup::single(work.clone()),
        preferences.clone(),
        progress.clone(),
    )?;
    assert_eq!(saved.group.anchor_work().unwrap(), &work);
    assert_eq!(saved.preferences, preferences);
    assert_eq!(saved.reader_progress, Some(progress.clone()));
    assert_eq!(
        saved.last_opened.as_ref().unwrap().variant,
        progress.chapter.variant
    );
    assert!(saved.last_opened.as_ref().unwrap().at_unix_millis > 0);

    // A new dropdown selection retains the last actually rendered branch.
    library.save_preferences(
        &qbutt_content::WorkGroup::single(work.clone()),
        choices(&work, 1),
    )?;
    drop(library);
    let status = Command::new(std::env::current_exe()?)
        .arg("--reopen")
        .arg(profile)
        .status()?;
    assert!(
        status.success(),
        "reader progress did not survive process restart"
    );

    let mut library = Library::open(profile)?;
    let before = library.get(&work.reference)?.unwrap();
    for offset in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01, 1.01] {
        let mut invalid = progress.clone();
        invalid.position = ReaderPosition::Page {
            page: 3,
            scroll_offset: offset,
        };
        assert!(matches!(
            library.save_reader_progress(
                &qbutt_content::WorkGroup::single(work.clone()),
                choices(&work, 0),
                invalid
            ),
            Err(LibraryError::InvalidEntry(_))
        ));
        assert_eq!(library.get(&work.reference)?.unwrap(), before);
    }
    let mut invalid = progress.clone();
    invalid.position = ReaderPosition::Page {
        page: u32::MAX,
        scroll_offset: 0.0,
    };
    assert!(matches!(
        library.save_reader_progress(
            &qbutt_content::WorkGroup::single(work.clone()),
            choices(&work, 0),
            invalid
        ),
        Err(LibraryError::InvalidEntry(_))
    ));
    for id in ["invalid-chapter", "00000000-0000-0000-0000-000000000000"] {
        let mut invalid = progress.clone();
        invalid.chapter.id = id.into();
        assert!(matches!(
            library.save_reader_progress(
                &qbutt_content::WorkGroup::single(work.clone()),
                choices(&work, 0),
                invalid
            ),
            Err(LibraryError::InvalidEntry(_))
        ));
    }
    let mut invalid = progress.clone();
    invalid.chapter.variant.publication.id = "33333333-3333-3333-3333-333333333333".into();
    assert!(matches!(
        library.save_reader_progress(
            &qbutt_content::WorkGroup::single(work.clone()),
            choices(&work, 0),
            invalid
        ),
        Err(LibraryError::InvalidEntry(_))
    ));
    let mut missing_variant = progress.clone();
    missing_variant.chapter.variant.selector = VariantSelector::MangaTranslation {
        language: "ja".into(),
        groups: Vec::new(),
    };
    assert!(matches!(
        library.save_reader_progress(
            &qbutt_content::WorkGroup::single(work.clone()),
            choices(&work, 0),
            missing_variant
        ),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);

    let raw = Connection::open(profile.join("content-library.sqlite"))?;
    raw.execute_batch(
        "CREATE TRIGGER reject_reader_progress BEFORE UPDATE ON library
         BEGIN SELECT RAISE(ABORT, 'generated reader write fault'); END;",
    )?;
    let mut next = progress.clone();
    next.position = ReaderPosition::Page {
        page: 4,
        scroll_offset: 0.625,
    };
    assert!(matches!(
        library.save_reader_progress(
            &qbutt_content::WorkGroup::single(work.clone()),
            choices(&work, 0),
            next
        ),
        Err(LibraryError::Sqlite(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);
    raw.execute_batch("DROP TRIGGER reject_reader_progress;")?;

    let retained = library.remove_history(&work.reference)?.unwrap();
    assert!(retained.last_opened.is_none());
    assert_eq!(retained.reader_progress, Some(progress.clone()));
    assert_eq!(retained.preferences, choices(&work, 1));
    assert!(retained.favorite);
    drop(library);

    let mut corrupt = retained;
    corrupt.reader_progress.as_mut().unwrap().position = ReaderPosition::Page {
        page: 3,
        scroll_offset: 2.0,
    };
    let metadata = serde_json::to_string(&corrupt)?;
    raw.execute("UPDATE library SET entry = ?1", [&metadata])?;
    let library = Library::open(profile)?;
    assert!(matches!(
        library.get(&work.reference),
        Err(LibraryError::CorruptEntry(_))
    ));
    assert_eq!(
        raw.query_row("SELECT entry FROM library", [], |row| row
            .get::<_, String>(0))?,
        metadata
    );
    Ok(())
}

fn verify_reopened(profile: &Path) -> Result<()> {
    let work = fixture();
    let library = Library::open(profile)?;
    let entry = library
        .get(&work.reference)?
        .ok_or("missing reopened reader work")?;
    assert_eq!(entry.group.anchor_work().unwrap(), &work);
    assert_eq!(entry.preferences, choices(&work, 1));
    assert_eq!(entry.reader_progress, Some(position(&work)));
    assert_eq!(
        entry.last_opened.as_ref().unwrap().variant,
        work.variants[0].reference
    );
    assert!(entry.favorite);
    assert_eq!(library.entries(LibraryView::History)?, vec![entry]);
    println!("second_process: exact chapter/page/scroll, full work and current choices restored");
    Ok(())
}

fn position(work: &Work) -> ReaderProgress {
    ReaderProgress {
        chapter: ChapterRef {
            variant: work.variants[0].reference.clone(),
            id: "22222222-2222-2222-2222-222222222222".into(),
        },
        position: ReaderPosition::Page {
            page: 3,
            scroll_offset: 0.625,
        },
    }
}

fn choices(work: &Work, variant: usize) -> VariantPreferences {
    VariantPreferences {
        selected: Some(work.variants[variant].reference.clone()),
        filters: VariantFilters {
            language: Some("ru".into()),
            source: Some(Source::MangaDex),
            transport: Some(Transport::Manga),
            ..Default::default()
        },
    }
}

fn fixture() -> Work {
    let reference = PublicationRef {
        source: Source::MangaDex,
        id: "11111111-1111-1111-1111-111111111111".into(),
    };
    let variants = ["ru", "en"]
        .into_iter()
        .map(|language| Variant {
            reference: VariantRef {
                publication: reference.clone(),
                selector: VariantSelector::MangaTranslation {
                    language: language.into(),
                    groups: Vec::new(),
                },
            },
            title: None,
            transport: Transport::Manga,
            episode: None,
            quality: None,
            translation: Some("Generated legal translation".into()),
            languages: vec![language.into()],
            player: None,
            size: None,
            seeders: None,
            info_hash: None,
            reported_video_ads: false,
        })
        .collect();
    Work {
        reference,
        title: "Generated reader fixture".into(),
        category: Category::Manga,
        url: Url::parse("https://mangadex.org/title/11111111-1111-1111-1111-111111111111").unwrap(),
        cover: None,
        description: Some("Generated legal metadata without downloaded content".into()),
        year: Some(2026),
        aliases: vec!["Generated alias".into()],
        identifiers: BTreeMap::from([("generated".into(), "reader".into())]),
        variants,
        needs_inspection: false,
    }
}
