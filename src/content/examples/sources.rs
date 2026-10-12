//! Process restart and SQLite faults for source settings in an owned profile.

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::{
    Category, Library, LibraryError, LibraryView, PublicationRef, ReaderFit, ReaderMode,
    ReaderSettings, Source, SourceFailure, SourceFailureReason, SourceOperation, SourceSettings,
    VariantRef, VariantSelector,
};
use rusqlite::Connection;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
        let profile = args.next().ok_or("missing owned profile")?;
        let library = Library::open(profile)?;
        let settings = row(&library, Source::RuTor)?;
        assert!(!settings.enabled);
        assert_eq!(settings.failure.as_ref().unwrap().operation, operation());
        assert_eq!(
            row(&library, Source::MangaDex)?.failure.unwrap().operation,
            manga_inspection()
        );
        assert_eq!(library.reader_settings()?.fit, ReaderFit::Width);
        assert_eq!(library.reader_settings()?.mode, ReaderMode::Continuous);
        let added = row(&library, Source::Rutube)?;
        assert!(added.enabled && added.revision == 0 && added.failure.is_none());
        assert_eq!(library.source_settings()?.len(), Source::ALL.len());
        return Ok(());
    }

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile =
        std::env::temp_dir().join(format!("qbutt-sources-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&profile)?;
    let result = exercise(&profile);
    std::fs::remove_dir_all(&profile)?;
    result?;
    println!("PASS source_preferences=restart exact_failed_operation=restart");
    println!("unrelated_success=retained stale_success=retained same_operation_aba=retained");
    println!("exact_success=cleared invalid_write=unchanged sqlite_abort=unchanged");
    println!("new_source=initialized existing_source_settings=preserved");
    println!("corrupt_source=isolated reader_settings=preserved unrelated_payload=preserved");
    Ok(())
}

fn exercise(profile: &Path) -> Result<()> {
    let payload = profile.join("unrelated-payload.txt");
    std::fs::write(&payload, b"generated unrelated payload")?;
    let operation = operation();
    let failure = SourceFailure {
        operation: operation.clone(),
        reason: SourceFailureReason::Http(403),
    };
    let mut library = Library::open(profile)?;
    let settings = library.source_settings()?;
    assert_eq!(settings.len(), Source::ALL.len());
    for source in Source::ALL {
        assert_eq!(
            settings
                .iter()
                .filter(|settings| settings.source == source)
                .count(),
            1
        );
    }
    assert!(
        library
            .source_settings()?
            .iter()
            .all(|row| row.enabled && row.revision == 0 && row.failure.is_none())
    );
    library.save_reader_settings(ReaderSettings {
        fit: ReaderFit::Width,
        mode: ReaderMode::Continuous,
        ..ReaderSettings::default()
    })?;
    library.set_source_enabled(Source::RuTor, false)?;
    let failed = library.record_source_failure(failure.clone())?;
    library.record_source_failure(SourceFailure {
        operation: manga_inspection(),
        reason: SourceFailureReason::Timeout,
    })?;
    drop(library);
    let raw = Connection::open(profile.join("content-library.sqlite"))?;
    assert_eq!(
        raw.execute("DELETE FROM source_settings WHERE source = 'rutube'", [])?,
        1
    );
    drop(raw);
    let status = Command::new(std::env::current_exe()?)
        .arg("--reopen")
        .arg(profile)
        .status()?;
    assert!(
        status.success(),
        "source settings did not survive process restart"
    );

    let mut library = Library::open(profile)?;
    let unrelated = SourceOperation::Inspect {
        reference: PublicationRef {
            source: Source::RuTor,
            id: "769531".into(),
        },
        language: None,
    };
    assert_eq!(
        library.clear_source_failure(&unrelated, failed.revision)?,
        failed
    );
    let newer = library.record_source_failure(SourceFailure {
        operation: unrelated,
        reason: SourceFailureReason::Timeout,
    })?;
    assert_eq!(
        library.clear_source_failure(&operation, failed.revision)?,
        newer
    );
    let repeated = library.record_source_failure(failure.clone())?;
    assert_eq!(
        library.clear_source_failure(&operation, failed.revision)?,
        repeated
    );
    let cleared = library.clear_source_failure(&operation, repeated.revision)?;
    assert!(!cleared.enabled && cleared.failure.is_none());
    assert_eq!(cleared.revision, repeated.revision + 1);
    assert_eq!(
        library.clear_source_failure(&operation, cleared.revision)?,
        cleared
    );

    let failed = library.record_source_failure(failure.clone())?;
    let enabled = library.set_source_enabled(Source::RuTor, true)?;
    assert_eq!(
        library.clear_source_failure(&operation, failed.revision)?,
        enabled
    );
    assert_eq!(library.set_source_enabled(Source::RuTor, true)?, enabled);
    let invalid = SourceFailure {
        operation: SourceOperation::Search {
            source: Source::MangaDex,
            query: "Generated title".into(),
            category: Category::Manga,
            language: None,
        },
        reason: SourceFailureReason::Http(999),
    };
    assert!(matches!(
        library.record_source_failure(invalid),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(row(&library, Source::RuTor)?, enabled);

    let raw = Connection::open(profile.join("content-library.sqlite"))?;
    raw.execute_batch(
        "CREATE TRIGGER reject_source_write BEFORE UPDATE ON source_settings
         BEGIN SELECT RAISE(ABORT, 'generated source fault'); END;",
    )?;
    assert!(matches!(
        library.set_source_enabled(Source::RuTor, false),
        Err(LibraryError::Sqlite(_))
    ));
    assert!(matches!(
        library.record_source_failure(failure.clone()),
        Err(LibraryError::Sqlite(_))
    ));
    assert!(matches!(
        library.clear_source_failure(&operation, enabled.revision),
        Err(LibraryError::Sqlite(_))
    ));
    assert_eq!(row(&library, Source::RuTor)?, enabled);
    raw.execute_batch("DROP TRIGGER reject_source_write;")?;

    let serialized = raw.query_row(
        "SELECT failure FROM source_settings WHERE source = 'rutor'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    assert_eq!(serde_json::from_str::<SourceFailure>(&serialized)?, failure);
    assert!(!serialized.contains("http://") && !serialized.contains("https://"));
    raw.execute("INSERT INTO source_settings (source, enabled, revision) VALUES ('generated_unknown', 1, 0)", [])?;
    assert!(matches!(
        library.source_settings(),
        Err(LibraryError::InvalidSchema)
    ));
    assert_eq!(library.reader_settings()?.fit, ReaderFit::Width);
    assert!(library.entries(LibraryView::All)?.is_empty());
    raw.execute(
        "DELETE FROM source_settings WHERE source = 'generated_unknown'",
        [],
    )?;
    raw.execute("DELETE FROM source_settings WHERE source = 'mangalib'", [])?;
    assert!(matches!(
        library.source_settings(),
        Err(LibraryError::InvalidSchema)
    ));
    raw.execute(
        "INSERT INTO source_settings (source, enabled, revision) VALUES ('mangalib', 1, 0)",
        [],
    )?;
    raw.execute(
        "UPDATE source_settings SET failure = 'not JSON' WHERE source = 'rutor'",
        [],
    )?;
    drop(library);
    let library = Library::open(profile)?;
    assert!(matches!(
        library.source_settings(),
        Err(LibraryError::Metadata(_))
    ));
    assert_eq!(library.reader_settings()?.fit, ReaderFit::Width);
    assert_eq!(library.reader_settings()?.mode, ReaderMode::Continuous);
    assert!(library.entries(LibraryView::All)?.is_empty());
    assert_eq!(std::fs::read(payload)?, b"generated unrelated payload");
    drop(library);
    drop(raw);
    Ok(())
}

fn row(library: &Library, source: Source) -> Result<SourceSettings> {
    library
        .source_settings()?
        .into_iter()
        .find(|row| row.source == source)
        .ok_or_else(|| "missing source row".into())
}

fn operation() -> SourceOperation {
    SourceOperation::Resolve(VariantRef {
        publication: PublicationRef {
            source: Source::RuTor,
            id: "769530".into(),
        },
        selector: VariantSelector::TorrentHash {
            info_hash: "be8e5c194fc5e64c2bbea1d66ef174153d2c5ced".into(),
        },
    })
}

fn manga_inspection() -> SourceOperation {
    SourceOperation::Inspect {
        reference: PublicationRef {
            source: Source::MangaDex,
            id: "d8f1d7da-8bb1-4fef-8396-b6123be02157".into(),
        },
        language: Some("ru".into()),
    }
}
