//! Current-user DPAPI and isolated Library settings across a real process restart.
//! The generated key is public fixture data; no environment or installed key is read.

use std::path::Path;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use qbutt_content::translation::context::ContextAdditions;
use qbutt_content::translation::text::{cache_key, restore_cached};
use qbutt_content::{
    ChapterRef, Library, LibraryError, OpenRouterSettings, PublicationRef, Source, TextChapter,
    TextSpan, TextTranslationService, TranslationSelection, VariantRef, VariantSelector,
};

const KEY: &str = "generated-fixture-key";
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let operation = args.next();
    if operation.as_deref() == Some(std::ffi::OsStr::new("--seed-reader")) {
        let profile = std::path::PathBuf::from(args.next().ok_or("fresh profile missing")?);
        if profile.exists() && profile.read_dir()?.next().is_some() {
            return Err("reader seed requires an empty isolated profile".into());
        }
        std::fs::create_dir_all(&profile)?;
        let path = profile.join("authored.html");
        std::fs::write(
            &path,
            "<h1>Ren's return</h1><p>Ren returned to the forest.</p><p>She carried the blue key.</p>",
        )?;
        let book = qbutt_content::LocalBook::open(&path)?;
        let original = book.read_chapter(&book.chapter_list()[0].reference)?;
        let mut translated = original.clone();
        for block in &mut translated.blocks {
            block.spans = vec![TextSpan::Text {
                text: format!("Cached: {}", block.plain_text()),
                bold: false,
                italic: false,
                underline: false,
            }];
        }
        let mut library = Library::open(&profile)?;
        let options = OpenRouterSettings {
            model: "example/text-model".into(),
            effort: "low".into(),
        };
        library.configure_text_translation(TextTranslationService::OpenRouter, &options, None)?;
        let key = cache_key(
            &original,
            "ru",
            "openrouter",
            &options.model,
            &options.effort,
        )?;
        library.save_translation(
            key,
            0,
            &translated,
            ContextAdditions::default(),
            &[],
            || true,
        )?;
        println!("PASS openrouter_reader_seed authored_book exact_cache no_key no_inference");
        return Ok(());
    }
    if matches!(
        operation.as_deref().and_then(|value| value.to_str()),
        Some("--reopen" | "--without-key")
    ) {
        let profile = args.next().ok_or("profile missing")?;
        let library = Library::open(profile)?;
        let saved = library.text_translation_settings()?;
        assert_eq!(saved.service, TextTranslationService::OpenRouter);
        assert_eq!(saved.openrouter.model, "example/text-model");
        if operation.as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
            assert!(saved.key_configured);
            let _client = library.openrouter_client()?;
            println!("PASS openrouter_settings process_reopen native_unlock");
        } else {
            assert!(!saved.key_configured);
            assert!(matches!(
                library.openrouter_client(),
                Err(LibraryError::Credentials(_))
            ));
            let original = original()?;
            let key = cache_key(
                &original,
                "ru",
                "openrouter",
                &saved.openrouter.model,
                &saved.openrouter.effort,
            )?;
            let cached: TextChapter = library
                .cached_translation(&key)?
                .ok_or("offline cache missing")?;
            assert_eq!(
                restore_cached(&original, cached)?.blocks[0].plain_text(),
                "Ren returned."
            );
            println!("PASS openrouter_settings env_ignored_without_saved_key exact_offline_cache");
        }
        return Ok(());
    }
    let started = Instant::now();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-openrouter-settings-{nonce}"));
    let result = exercise(&profile);
    std::fs::remove_dir_all(&profile)?;
    result?;
    println!(
        "PASS openrouter_settings dpapi rollback key_removal no_env elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(())
}

fn exercise(profile: &Path) -> Result<()> {
    let mut library = Library::open(profile)?;
    assert_eq!(
        library.text_translation_settings()?.service,
        TextTranslationService::Codex
    );
    assert!(!library.text_translation_settings()?.key_configured);
    assert!(library.openrouter_client().is_err());
    let options = OpenRouterSettings {
        model: "example/text-model".into(),
        effort: "low".into(),
    };
    let saved = library.configure_text_translation(
        TextTranslationService::OpenRouter,
        &options,
        Some(KEY),
    )?;
    assert!(saved.key_configured);
    assert!(!format!("{saved:?}").contains(KEY));
    // Construction is deliberately outside a Tokio runtime, like the Library worker.
    let _client = library.openrouter_client()?;
    exercise_cache(&mut library, &options)?;
    drop(library);
    let database = profile.join("content-library.sqlite");
    let connection = rusqlite::Connection::open(&database)?;
    let protected: Vec<u8> = connection.query_row(
        "SELECT api_key_dpapi FROM text_translation_service",
        [],
        |row| row.get(0),
    )?;
    assert!(!protected.is_empty());
    assert!(
        !protected
            .windows(KEY.len())
            .any(|part| part == KEY.as_bytes())
    );
    drop(connection);

    reopen(profile, "--reopen")?;
    let connection = rusqlite::Connection::open(&database)?;
    connection.execute_batch(
        "CREATE TRIGGER fail_service_write BEFORE UPDATE ON text_translation_service
         BEGIN
             SELECT RAISE(ABORT, 'fixture');
         END;",
    )?;
    drop(connection);
    let mut library = Library::open(profile)?;
    let changed = OpenRouterSettings {
        effort: "high".into(),
        ..options.clone()
    };
    assert!(
        library
            .configure_text_translation(
                TextTranslationService::OpenRouter,
                &changed,
                Some("different-fixture-key")
            )
            .is_err()
    );
    assert_eq!(library.text_translation_settings()?.openrouter, options);
    drop(library);
    let connection = rusqlite::Connection::open(&database)?;
    let unchanged: Vec<u8> = connection.query_row(
        "SELECT api_key_dpapi FROM text_translation_service",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(unchanged, protected);
    connection.execute_batch("DROP TRIGGER fail_service_write;")?;
    drop(connection);

    let mut library = Library::open(profile)?;
    let removed = library.configure_text_translation(
        TextTranslationService::OpenRouter,
        &options,
        Some(""),
    )?;
    assert_eq!(removed.openrouter, options);
    assert!(!removed.key_configured);
    assert!(matches!(
        library.openrouter_client(),
        Err(LibraryError::Credentials(_))
    ));
    drop(library);
    reopen(profile, "--without-key")?;
    let mut library = Library::open(profile)?;
    library.configure_text_translation(TextTranslationService::OpenRouter, &options, Some(KEY))?;
    drop(library);
    let connection = rusqlite::Connection::open(&database)?;
    connection.execute(
        "UPDATE text_translation_service SET api_key_dpapi = x'0102'",
        [],
    )?;
    drop(connection);
    let library = Library::open(profile)?;
    assert_eq!(library.text_translation_settings()?.openrouter, options);
    let error = library
        .openrouter_client()
        .err()
        .ok_or("corrupt blob unlocked")?;
    assert!(matches!(error, LibraryError::Credentials(_)));
    assert!(!error.to_string().contains(KEY));
    drop(library);

    // A genuine previous schema gains only this singleton; existing reader choices survive.
    let connection = rusqlite::Connection::open(&database)?;
    connection.execute_batch(
        "DROP TABLE text_translation_service;
         UPDATE reader_settings SET font_size = 24;
         PRAGMA user_version = 11;",
    )?;
    drop(connection);
    let library = Library::open(profile)?;
    assert_eq!(
        library.text_translation_settings()?.service,
        TextTranslationService::Codex
    );
    assert_eq!(library.reader_settings()?.font_size, 24);
    Ok(())
}

fn reopen(profile: &Path, operation: &str) -> Result<()> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg(operation)
        .arg(profile)
        .env("OPENROUTER_API_KEY", "generated-env-key-must-not-be-used");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    assert!(command.status()?.success());
    Ok(())
}

fn original() -> Result<TextChapter> {
    Ok(TextChapter::from_html(
        ChapterRef {
            variant: VariantRef {
                publication: PublicationRef {
                    source: Source::Kakuyomu,
                    id: "1001".into(),
                },
                selector: VariantSelector::OriginalText,
            },
            id: "1001/1".into(),
        },
        "<p>レンは帰った。</p>",
    )?)
}

fn exercise_cache(library: &mut Library, options: &OpenRouterSettings) -> Result<()> {
    let original = original()?;
    let mut translated = original.clone();
    translated.blocks[0].spans = vec![TextSpan::Text {
        text: "Ren returned.".into(),
        bold: false,
        italic: false,
        underline: false,
    }];
    let codex = TranslationSelection {
        target_language: "ru".into(),
        model: "gpt-6.1-sol".into(),
        effort: "low".into(),
        explicit: true,
    };
    library.save_translation_selection(&codex)?;
    let key = cache_key(
        &original,
        "ru",
        "openrouter",
        &options.model,
        &options.effort,
    )?;
    library.save_translation(
        key.clone(),
        0,
        &translated,
        ContextAdditions::default(),
        &[],
        || true,
    )?;
    assert_eq!(library.translation_selection("ru")?, Some(codex));
    for (provider, model, effort, language) in [
        ("codex", options.model.as_str(), "low", "ru"),
        ("openrouter", "example/another-model", "low", "ru"),
        ("openrouter", options.model.as_str(), "high", "ru"),
        ("openrouter", options.model.as_str(), "low", "en"),
    ] {
        let different = cache_key(&original, language, provider, model, effort)?;
        assert!(
            library
                .cached_translation::<TextChapter>(&different)?
                .is_none()
        );
    }
    let changed = OpenRouterSettings {
        effort: "high".into(),
        ..options.clone()
    };
    library.configure_text_translation(TextTranslationService::OpenRouter, &changed, None)?;
    assert!(
        library
            .save_translation(
                key.clone(),
                0,
                &translated,
                ContextAdditions::default(),
                &[],
                || true
            )
            .is_err()
    );
    library.configure_text_translation(TextTranslationService::OpenRouter, options, None)?;
    assert!(
        library
            .save_translation(
                key.clone(),
                1,
                &translated,
                ContextAdditions::default(),
                &[],
                || true
            )
            .is_err()
    );
    let cached: TextChapter = library
        .cached_translation(&key)?
        .ok_or("accepted cache lost")?;
    assert_eq!(restore_cached(&original, cached)?, translated);
    println!(
        "PASS openrouter_cache exact_identity stale_settings stale_context codex_selection_preserved"
    );
    Ok(())
}
