//! Generated original text, atomic SQLite writes and separate-process restart.
//! Qt canvas confirmation and cancellation belong to native acceptance.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::{
    Category, ChapterRef, Library, LibraryError, LibraryView, PublicationRef, ReaderPosition,
    ReaderProgress, Source, TextChapter, Transport, Variant, VariantPreferences, VariantRef,
    VariantSelector, Work,
};
use rusqlite::Connection;
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
        verify_reopened(Path::new(&arguments.next().ok_or("missing profile")?))?;
        return Ok(());
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-text-progress-{nonce}"));
    std::fs::create_dir(&profile)?;
    let result = exercise(&profile);
    std::fs::remove_dir_all(&profile)?;
    result?;
    println!(
        "PASS original_text_progress restart=exact chapter=exact anchor=stable finite_bounds=ok sqlite_fault=atomic retry=immutable"
    );
    Ok(())
}

fn exercise(profile: &Path) -> Result<()> {
    let (work, chapter, progress) = fixture()?;
    qbutt_content::SourceOperation::Chapters(chapter.reference.variant.clone()).validate()?;
    qbutt_content::SourceOperation::Chapter(chapter.reference.clone()).validate()?;
    let original = serde_json::to_vec(&chapter)?;
    let preferences = VariantPreferences {
        selected: Some(work.variants[0].reference.clone()),
        ..Default::default()
    };
    let mut library = Library::open(profile)?;
    library.set_favorite(&qbutt_content::WorkGroup::single(work.clone()), true)?;
    library.save_preferences(
        &qbutt_content::WorkGroup::single(work.clone()),
        preferences.clone(),
    )?;
    assert!(library.entries(LibraryView::History)?.is_empty());
    for offset in [0.0, 1.0] {
        let mut boundary = progress.clone();
        boundary.position = ReaderPosition::Text {
            block_id: chapter.blocks[10].id.clone(),
            offset,
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
    assert_eq!(saved.reader_progress, Some(progress.clone()));
    assert_eq!(saved.last_opened.unwrap().variant, progress.chapter.variant);
    drop(library);
    assert!(
        Command::new(std::env::current_exe()?)
            .arg("--reopen")
            .arg(profile)
            .status()?
            .success()
    );

    let mut library = Library::open(profile)?;
    let before = library.get(&work.reference)?.unwrap();
    for offset in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        let mut invalid = progress.clone();
        invalid.position = ReaderPosition::Text {
            block_id: chapter.blocks[10].id.clone(),
            offset,
        };
        assert!(matches!(
            library.save_reader_progress(
                &qbutt_content::WorkGroup::single(work.clone()),
                preferences.clone(),
                invalid
            ),
            Err(LibraryError::InvalidEntry(_))
        ));
        assert_eq!(library.get(&work.reference)?.unwrap(), before);
    }
    for block_id in [String::new(), "x".repeat(129), "invalid\nanchor".into()] {
        let mut invalid = progress.clone();
        invalid.position = ReaderPosition::Text {
            block_id,
            offset: 0.0,
        };
        assert!(matches!(
            library.save_reader_progress(
                &qbutt_content::WorkGroup::single(work.clone()),
                preferences.clone(),
                invalid
            ),
            Err(LibraryError::InvalidEntry(_))
        ));
    }
    let mut invalid = progress.clone();
    invalid.position = ReaderPosition::Page {
        page: 0,
        scroll_offset: 0.0,
    };
    assert!(matches!(
        library.save_reader_progress(
            &qbutt_content::WorkGroup::single(work.clone()),
            preferences.clone(),
            invalid
        ),
        Err(LibraryError::InvalidEntry(_))
    ));
    let mut wrong_work = progress.clone();
    wrong_work.chapter.id = "1002/2002".into();
    assert!(matches!(
        library.save_reader_progress(
            &qbutt_content::WorkGroup::single(work.clone()),
            preferences.clone(),
            wrong_work
        ),
        Err(LibraryError::InvalidEntry(_))
    ));

    let raw = Connection::open(profile.join("content-library.sqlite"))?;
    raw.execute_batch(
        "CREATE TRIGGER reject_text_progress BEFORE UPDATE ON library
        BEGIN SELECT RAISE(ABORT, 'generated text write fault'); END;",
    )?;
    let mut intent = progress.clone();
    intent.position = ReaderPosition::Text {
        block_id: chapter.blocks[15].id.clone(),
        offset: 0.875,
    };
    assert!(matches!(
        library.save_reader_progress(
            &qbutt_content::WorkGroup::single(work.clone()),
            preferences.clone(),
            intent.clone()
        ),
        Err(LibraryError::Sqlite(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);
    raw.execute_batch("DROP TRIGGER reject_text_progress;")?;
    let retried = library.save_reader_progress(
        &qbutt_content::WorkGroup::single(work.clone()),
        preferences,
        intent.clone(),
    )?;
    assert_eq!(retried.reader_progress, Some(intent.clone()));
    assert_eq!(serde_json::to_vec(&chapter)?, original);
    let retained = library.remove_history(&work.reference)?.unwrap();
    assert!(retained.favorite && retained.last_opened.is_none());
    assert_eq!(retained.reader_progress, Some(intent));

    let mut corrupt = retained;
    corrupt.reader_progress.as_mut().unwrap().position = ReaderPosition::Text {
        block_id: chapter.blocks[0].id.clone(),
        offset: 2.0,
    };
    raw.execute(
        "UPDATE library SET entry = ?1",
        [serde_json::to_string(&corrupt)?],
    )?;
    assert!(matches!(
        library.get(&work.reference),
        Err(LibraryError::CorruptEntry(_))
    ));
    Ok(())
}

fn verify_reopened(profile: &Path) -> Result<()> {
    let (work, _, progress) = fixture()?;
    let library = Library::open(profile)?;
    let entry = library
        .get(&work.reference)?
        .ok_or("missing saved original text")?;
    assert_eq!(entry.group.anchor_work().unwrap(), &work);
    assert_eq!(entry.reader_progress, Some(progress.clone()));
    assert_eq!(entry.last_opened.unwrap().variant, progress.chapter.variant);
    assert!(entry.favorite);
    println!(
        "second_process: original text chapter, stable block and within-block offset restored"
    );
    Ok(())
}

fn fixture() -> Result<(Work, TextChapter, ReaderProgress)> {
    let publication = PublicationRef {
        source: Source::Kakuyomu,
        id: "1001".into(),
    };
    let variant = VariantRef {
        publication: publication.clone(),
        selector: VariantSelector::OriginalText,
    };
    let chapter = ChapterRef {
        variant: variant.clone(),
        id: "1001/2002".into(),
    };
    let body = (0..30)
        .map(|index| format!("<p>This is original fixture paragraph {index}.</p>"))
        .collect::<String>();
    let chapter = TextChapter::from_html(chapter, &body)?;
    let progress = ReaderProgress {
        chapter: chapter.reference.clone(),
        position: ReaderPosition::Text {
            block_id: chapter.blocks[10].id.clone(),
            offset: 0.625,
        },
    };
    let work = Work {
        reference: publication,
        title: "Generated original story".into(),
        category: Category::Books,
        url: Url::parse("https://kakuyomu.jp/works/1001")?,
        cover: None,
        description: None,
        year: None,
        aliases: vec![],
        identifiers: BTreeMap::new(),
        variants: vec![Variant {
            reference: variant,
            title: None,
            transport: Transport::Text,
            episode: None,
            quality: None,
            translation: None,
            languages: vec!["ja".into()],
            player: None,
            size: None,
            seeders: None,
            info_hash: None,
            reported_video_ads: false,
        }],
        needs_inspection: false,
    };
    Ok((work, chapter, progress))
}
