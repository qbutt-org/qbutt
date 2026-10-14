//! Generated chapter and PNG caches with profile migrations and SQLite faults.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::translation::context::{
    ContextAdditions, ContextEvidence, EvidenceSegment, ImageObservation, TranslationTerm,
};
use qbutt_content::translation::manga::{FontFamily, Ink, PageRegion, Rect, TextRun};
use qbutt_content::translation::text::{cache_key, restore_cached};
use qbutt_content::{
    Category, ChapterRef, Library, LibraryError, LibraryView, PublicationRef, Source, TextChapter,
    TextSpan, TranslationCacheKey, TranslationSelection, Transport, Variant, VariantPreferences,
    VariantRef, VariantSelector, Work, WorkGroup,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let reference = ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::Kakuyomu,
                id: "1001".into(),
            },
            selector: VariantSelector::OriginalText,
        },
        id: "1001/1".into(),
    };
    let original = TextChapter::from_html(reference, "<p><u>レンは帰った。</u></p>")?;
    let mut translated = original.clone();
    translated.blocks[0].spans = vec![TextSpan::Text {
        text: "Рен вернулась.".into(),
        bold: false,
        italic: false,
        underline: true,
    }];

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-translation-cache-{nonce}"));
    let result = (|| -> Result<()> {
        let mut library = Library::open(&profile)?;
        let key = cache_key(&original, "ru", "codex", "gpt-6.1-sol", "low")?;
        library.save_translation(
            key.clone(),
            0,
            &translated,
            ContextAdditions::default(),
            &[],
            || true,
        )?;
        drop(library);
        exercise_cancelled_save(&profile, &original)?;

        // A v8 profile already has translated content but no selected model.
        let connection = rusqlite::Connection::open(profile.join("content-library.sqlite"))?;
        connection.execute_batch(
            "ALTER TABLE reader_settings DROP COLUMN translation_language;
             DROP TABLE translation_selection;
             DROP TABLE translated_images;
             DROP TABLE text_translation_service;
             PRAGMA user_version = 8;",
        )?;
        drop(connection);

        let mut library = Library::open(&profile)?;
        assert!(library.translation_selection("ru")?.is_none());
        assert!(library.cached_translation::<TextChapter>(&key)?.is_some());
        let mut settings = library.reader_settings()?;
        assert_eq!(settings.translation_language, "ru");
        settings.translation_language = "en".into();
        library.save_reader_settings(settings)?;

        // The first online exact hit records its resolved identity. Reopening
        // then uses that identity directly, without asking Codex for a catalog.
        let selection = TranslationSelection {
            target_language: "ru".into(),
            model: "gpt-6.1-sol".into(),
            effort: "low".into(),
            explicit: false,
        };
        library.save_translation_selection(&selection)?;
        drop(library);

        let mut library = Library::open(&profile)?;
        assert_eq!(library.reader_settings()?.translation_language, "en");
        let saved = library
            .translation_selection("ru")?
            .ok_or("selection missing")?;
        assert_eq!(saved, selection);
        let exact = cache_key(&original, "ru", "codex", &saved.model, &saved.effort)?;
        let chapter: TextChapter = library
            .cached_translation(&exact)?
            .ok_or("exact translation missing")?;
        assert_eq!(restore_cached(&original, chapter)?, translated);
        for (language, model, effort) in [
            ("ru", "another-model", "low"),
            ("ru", "gpt-6.1-sol", "medium"),
            ("en", "gpt-6.1-sol", "low"),
        ] {
            let different = cache_key(&original, language, "codex", model, effort)?;
            assert!(
                library
                    .cached_translation::<TextChapter>(&different)?
                    .is_none()
            );
        }
        assert!(library.translation_selection("en")?.is_none());

        let explicit = TranslationSelection {
            explicit: true,
            ..saved
        };
        library.save_translation_selection(&explicit)?;
        let different = cache_key(&original, "ru", "codex", "another-model", "low")?;
        assert!(
            library
                .save_translation(
                    different,
                    0,
                    &translated,
                    ContextAdditions::default(),
                    &[],
                    || true
                )
                .is_err()
        );
        assert_eq!(library.translation_selection("ru")?, Some(explicit));

        library.reset_translation_selection("ru")?;
        let automatic = library
            .translation_selection("ru")?
            .ok_or("automatic selection missing")?;
        assert!(!automatic.explicit);
        let offline_key = cache_key(
            &original,
            "ru",
            "codex",
            &automatic.model,
            &automatic.effort,
        )?;
        assert!(
            library
                .cached_translation::<TextChapter>(&offline_key)?
                .is_some()
        );

        // A term learned in another chapter must not discard an accepted
        // translation. Reopen using the current shared work context, as Qt does.
        let second = TextChapter::from_html(
            ChapterRef {
                id: "1001/2".into(),
                ..original.reference.clone()
            },
            "<p>星の鍵が扉を開く。</p>",
        )?;
        let mut second_translated = second.clone();
        second_translated.blocks[0].spans = vec![TextSpan::Text {
            text: "Звёздный ключ открывает дверь.".into(),
            bold: false,
            italic: false,
            underline: false,
        }];
        let evidence = EvidenceSegment {
            chapter: second.reference.clone(),
            id: second.blocks[0].id.clone(),
            text: second.blocks[0].plain_text(),
            image_observation: None,
        };
        let term = TranslationTerm {
            source: "星の鍵".into(),
            target: "Звёздный ключ".into(),
            forms: vec![],
            evidence: ContextEvidence {
                chapter: second.reference.clone(),
                segment_id: evidence.id.clone(),
                quote: evidence.text.clone(),
                image_observation: None,
            },
        };
        let second_key = cache_key(&second, "ru", "codex", "gpt-6.1-sol", "low")?;
        let expanded = library.save_translation(
            second_key,
            0,
            &second_translated,
            ContextAdditions {
                terms: vec![term],
                facts: vec![],
            },
            &[evidence],
            || true,
        )?;
        assert_eq!(expanded.revision, 1);
        let stale = library.save_translation(
            key.clone(),
            0,
            &translated,
            ContextAdditions::default(),
            &[],
            || true,
        );
        assert!(stale.is_err(), "stale context save must still be rejected");
        drop(library);

        let mut library = Library::open(&profile)?;
        let current = library.translation_context(&original.reference.variant.publication, "ru")?;
        let current_key = cache_key(&original, "ru", "codex", "gpt-6.1-sol", "low")?;
        assert_eq!(current.revision, 1);
        assert_eq!(current_key, key);
        let previous: Option<TextChapter> = library.cached_translation(&current_key)?;
        let previous =
            previous.ok_or("accepted chapter 1 vanished after chapter 2 context growth")?;
        assert_eq!(restore_cached(&original, previous)?, translated);

        let wrong_digest = TranslationCacheKey {
            original_digest: "b".repeat(64),
            ..key.clone()
        };
        let wrong_pipeline = TranslationCacheKey {
            pipeline: key.pipeline + 1,
            ..key.clone()
        };
        let wrong_page = TranslationCacheKey {
            page: Some(0),
            ..key.clone()
        };
        let wrong_provider = TranslationCacheKey {
            provider: "another-provider".into(),
            ..key.clone()
        };
        let mut wrong_chapter = key.clone();
        wrong_chapter.chapter.id = "1001/99".into();
        let mut wrong_publication = key.clone();
        wrong_publication.chapter.variant.publication.id = "1002".into();
        wrong_publication.chapter.id = "1002/1".into();
        for different in [
            wrong_digest,
            wrong_pipeline,
            wrong_page,
            wrong_provider,
            wrong_chapter,
            wrong_publication,
        ] {
            assert!(
                library
                    .cached_translation::<TextChapter>(&different)?
                    .is_none()
            );
        }

        let manga = ChapterRef {
            variant: VariantRef {
                publication: PublicationRef {
                    source: Source::MangaDex,
                    id: "11111111-1111-1111-1111-111111111111".into(),
                },
                selector: VariantSelector::MangaTranslation {
                    language: "ja".into(),
                    groups: vec![],
                },
            },
            id: "22222222-2222-2222-2222-222222222222".into(),
        };
        let image = ImageObservation {
            page: 0,
            original_digest: "a".repeat(64),
            bounds: [100_000, 200_000, 500_000, 600_000],
        };
        let evidence = EvidenceSegment {
            chapter: manga.clone(),
            id: "bubble-1".into(),
            text: "フリーレン、待って！".into(),
            image_observation: Some(image.clone()),
        };
        let reference = ContextEvidence {
            chapter: manga.clone(),
            segment_id: evidence.id.clone(),
            quote: "フリーレン".into(),
            image_observation: Some(image),
        };
        let term = TranslationTerm {
            source: "フリーレン".into(),
            target: "Фрирен".into(),
            forms: vec![],
            evidence: reference,
        };
        let manga_key = TranslationCacheKey {
            chapter: manga.clone(),
            page: Some(0),
            target_language: "ru".into(),
            original_digest: "a".repeat(64),
            provider: "codex".into(),
            model: "gpt-6.1-sol".into(),
            effort: "low".into(),
            pipeline: 4,
        };
        let regions = vec![PageRegion {
            id: evidence.id.clone(),
            source_text: evidence.text.clone(),
            translation: vec![
                TextRun {
                    text: "Фрирен, ".into(),
                    bold: false,
                    italic: false,
                },
                TextRun {
                    text: "подожди!".into(),
                    bold: true,
                    italic: true,
                },
            ],
            family: FontFamily::Comic,
            angle: -12.0,
            text_color: Some("#314455".into()),
            outline_color: Some("#ffffff".into()),
            bounds: Rect {
                x: 0.1,
                y: 0.2,
                width: 0.4,
                height: 0.4,
            },
            layout: Rect {
                x: 0.15,
                y: 0.25,
                width: 0.3,
                height: 0.3,
            },
            ink: Ink::Dark,
            ink_boxes: vec![Rect {
                x: 0.15,
                y: 0.25,
                width: 0.2,
                height: 0.2,
            }],
        }];
        let additions = ContextAdditions {
            terms: vec![term.clone()],
            facts: vec![],
        };
        let accepted = library.save_translation(
            manga_key.clone(),
            0,
            &regions,
            additions,
            &[evidence.clone()],
            || true,
        )?;
        assert_eq!(accepted.revision, 1);
        drop(library);

        let mut library = Library::open(&profile)?;
        let context = library.translation_context(&manga.variant.publication, "ru")?;
        assert_eq!(context.terms, vec![term.clone()]);
        let exact = manga_key.clone();
        let cached = library.cached_translation::<Vec<PageRegion>>(&exact)?;
        assert_eq!(
            serde_json::to_value(cached)?,
            serde_json::to_value(Some(regions))?
        );
        let mut forged = term;
        forged.source = "待って".into();
        forged.evidence.quote = "待って".into();
        forged
            .evidence
            .image_observation
            .as_mut()
            .unwrap()
            .original_digest = "b".repeat(64);
        let rejected = library.save_translation(
            exact.clone(),
            context.revision,
            &vec!["Подожди"],
            ContextAdditions {
                terms: vec![forged],
                facts: vec![],
            },
            &[evidence.clone()],
            || true,
        );
        assert!(rejected.is_err());
        let mut wrong_page = evidence.clone();
        wrong_page.image_observation.as_mut().unwrap().page = 1;
        assert!(
            library
                .save_translation(
                    exact,
                    context.revision,
                    &vec!["Подожди"],
                    ContextAdditions::default(),
                    &[wrong_page],
                    || true,
                )
                .is_err()
        );
        assert_eq!(
            library.translation_context(&manga.variant.publication, "ru")?,
            context
        );
        drop(library);
        exercise_image_cache(&profile, manga_key, evidence, &original, &translated)?;
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    result?;
    println!(
        "PASS translation_cache exact_hit model_miss effort_miss language_miss v8_migration target_persistence automatic_offline context_growth_offline stale_context_save digest_miss pipeline_miss page_miss provider_miss chapter_miss publication_miss manga_image_context"
    );
    Ok(())
}

fn exercise_cancelled_save(profile: &Path, original: &TextChapter) -> Result<()> {
    use std::cell::Cell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    let mut writer = Library::open(profile)?;
    let publication = &original.reference.variant.publication;
    let context = writer.translation_context(publication, "ru")?;
    let selection = writer.translation_selection("ru")?;
    let key = cache_key(original, "ru", "codex", "cancelled-model", "low")?;
    let evidence = EvidenceSegment {
        chapter: original.reference.clone(),
        id: original.blocks[0].id.clone(),
        text: original.blocks[0].plain_text(),
        image_observation: None,
    };
    let additions = ContextAdditions {
        terms: vec![TranslationTerm {
            source: "レン".into(),
            target: "Рен".into(),
            forms: vec![],
            evidence: ContextEvidence {
                chapter: original.reference.clone(),
                segment_id: evidence.id.clone(),
                quote: evidence.text.clone(),
                image_observation: None,
            },
        }],
        facts: vec![],
    };
    let checks = Cell::new(0);
    let result = writer.save_translation(
        key.clone(),
        context.revision,
        original,
        additions.clone(),
        &[evidence.clone()],
        || {
            checks.set(checks.get() + 1);
            checks.get() < 3
        },
    );
    assert!(matches!(result, Err(LibraryError::Cancelled)));
    assert_eq!(writer.translation_context(publication, "ru")?, context);
    assert_eq!(writer.translation_selection("ru")?, selection);
    assert!(writer.cached_translation::<TextChapter>(&key)?.is_none());

    let lock = rusqlite::Connection::open(profile.join("content-library.sqlite"))?;
    lock.execute_batch("BEGIN IMMEDIATE")?;
    let current = Arc::new(AtomicBool::new(true));
    let job_current = Arc::clone(&current);
    let job_key = key.clone();
    let job_original = original.clone();
    let (entered, ready) = mpsc::sync_channel(1);
    let (finished, outcome) = mpsc::sync_channel(1);
    let job = std::thread::spawn(move || {
        let announced = Cell::new(false);
        let result = writer.save_translation(
            job_key,
            context.revision,
            &job_original,
            additions,
            &[evidence],
            || {
                if !announced.replace(true) {
                    let _ = entered.send(());
                }
                job_current.load(Ordering::SeqCst)
            },
        );
        let _ = finished.send(result);
    });
    ready.recv_timeout(Duration::from_secs(2))?;
    assert!(matches!(
        outcome.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    current.store(false, Ordering::SeqCst);
    lock.execute_batch("ROLLBACK")?;
    assert!(matches!(
        outcome.recv_timeout(Duration::from_secs(2))?,
        Err(LibraryError::Cancelled)
    ));
    job.join().map_err(|_| "cancelled writer panicked")?;
    let library = Library::open(profile)?;
    assert_eq!(library.translation_context(publication, "ru")?, context);
    assert_eq!(library.translation_selection("ru")?, selection);
    assert!(library.cached_translation::<TextChapter>(&key)?.is_none());
    println!("PASS translation_cancel write_lock before_commit context_selection_payload_rollback");
    Ok(())
}

fn exercise_image_cache(
    profile: &Path,
    key: TranslationCacheKey,
    evidence: EvidenceSegment,
    original: &TextChapter,
    translated: &TextChapter,
) -> Result<()> {
    // Legal generated 1x1 RGBA PNGs, kept as bytes rather than JSON or base64.
    let white = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\
        \x00\x00\x00\x0dIDAT\x78\x9c\x63\xf8\xff\xff\xff\x7f\x00\x09\xfb\x03\xfd\x2a\x86\xe3\x8a\
        \x00\x00\x00\x00IEND\xae\x42\x60\x82";
    let black = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\
        \x00\x00\x00\x0dIDAT\x78\x9c\x63\x60\x60\x60\xf8\x0f\x00\x01\x04\x01\x00\x5f\xe5\xc3\x4b\
        \x00\x00\x00\x00IEND\xae\x42\x60\x82";
    let publication = &key.chapter.variant.publication;
    let mut library = Library::open(profile)?;
    let group = WorkGroup::single(Work {
        reference: original.reference.variant.publication.clone(),
        title: "Generated original chapter".into(),
        category: Category::Books,
        url: "https://kakuyomu.jp/works/1001".parse()?,
        cover: None,
        description: None,
        year: None,
        aliases: vec![],
        identifiers: Default::default(),
        variants: vec![Variant {
            reference: original.reference.variant.clone(),
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
    });
    let history = library.record_opened(
        &group,
        original.reference.variant.clone(),
        VariantPreferences::default(),
    )?;
    let json = library.cached_translation::<serde_json::Value>(&key)?;
    let context = library.translation_context(publication, "ru")?;
    let selection = library.translation_selection("ru")?;
    drop(library);

    // Schema 10 keeps its JSON cache, context, selected model and Original history.
    let connection = rusqlite::Connection::open(profile.join("content-library.sqlite"))?;
    connection.execute_batch(
        "DROP TABLE translated_images;
         DROP TABLE text_translation_service;
         PRAGMA user_version = 10;",
    )?;
    let mut library = Library::open(profile)?;
    assert_eq!(library.translation_context(publication, "ru")?, context);
    assert_eq!(library.translation_selection("ru")?, selection);
    assert!(library.cached_translation_image(&key)?.is_none());
    library.save_image_translation(
        key.clone(),
        context.revision,
        white,
        ContextAdditions::default(),
        std::slice::from_ref(&evidence),
    )?;
    drop(library);

    let mut library = Library::open(profile)?;
    assert_eq!(
        library.cached_translation_image(&key)?.as_deref(),
        Some(white.as_slice())
    );
    let encoded_key = serde_json::to_string(&key)?;
    let storage: (String, i64) = connection.query_row(
        "SELECT typeof(png), length(png) FROM translated_images WHERE key = ?1",
        [&encoded_key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(storage, ("blob".into(), white.len() as i64));
    for different in [
        TranslationCacheKey {
            model: "another-model".into(),
            ..key.clone()
        },
        TranslationCacheKey {
            effort: "medium".into(),
            ..key.clone()
        },
        TranslationCacheKey {
            target_language: "en".into(),
            ..key.clone()
        },
        TranslationCacheKey {
            page: Some(1),
            ..key.clone()
        },
        TranslationCacheKey {
            original_digest: "b".repeat(64),
            ..key.clone()
        },
        TranslationCacheKey {
            provider: "another-provider".into(),
            ..key.clone()
        },
        TranslationCacheKey {
            pipeline: key.pipeline + 1,
            ..key.clone()
        },
    ] {
        assert!(library.cached_translation_image(&different)?.is_none());
    }
    let additions = ContextAdditions {
        terms: vec![TranslationTerm {
            source: "待って".into(),
            target: "Подожди".into(),
            forms: vec![],
            evidence: ContextEvidence {
                chapter: evidence.chapter.clone(),
                segment_id: evidence.id.clone(),
                quote: "待って".into(),
                image_observation: evidence.image_observation.clone(),
            },
        }],
        facts: vec![],
    };
    let mut oversized = vec![0; 16 * 1024 * 1024 + 1];
    oversized[..white.len()].copy_from_slice(white);
    for invalid in [b"".as_slice(), b"not a PNG", oversized.as_slice()] {
        assert!(matches!(
            library.save_image_translation(
                key.clone(),
                context.revision,
                invalid,
                additions.clone(),
                std::slice::from_ref(&evidence),
            ),
            Err(LibraryError::InvalidEntry(_))
        ));
    }
    assert_eq!(library.translation_context(publication, "ru")?, context);
    assert_eq!(
        library.cached_translation_image(&key)?.as_deref(),
        Some(white.as_slice())
    );

    let previous_selection = TranslationSelection {
        target_language: "ru".into(),
        model: "keep-selection-model".into(),
        effort: "low".into(),
        explicit: false,
    };
    library.save_translation_selection(&previous_selection)?;
    connection.execute_batch(
        "CREATE TRIGGER reject_image BEFORE INSERT ON translated_images
         BEGIN SELECT RAISE(ABORT, 'generated image write fault'); END;",
    )?;
    assert!(matches!(
        library.save_image_translation(
            key.clone(),
            context.revision,
            black,
            additions.clone(),
            std::slice::from_ref(&evidence),
        ),
        Err(LibraryError::Sqlite(_))
    ));
    assert_eq!(library.translation_context(publication, "ru")?, context);
    assert_eq!(
        library.translation_selection("ru")?,
        Some(previous_selection.clone())
    );
    assert_eq!(
        library.cached_translation_image(&key)?.as_deref(),
        Some(white.as_slice())
    );
    connection.execute_batch("DROP TRIGGER reject_image;")?;

    let explicit = TranslationSelection {
        explicit: true,
        ..previous_selection
    };
    library.save_translation_selection(&explicit)?;
    assert!(matches!(
        library.save_image_translation(
            key.clone(),
            context.revision,
            black,
            additions.clone(),
            std::slice::from_ref(&evidence),
        ),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.translation_context(publication, "ru")?, context);
    assert_eq!(library.translation_selection("ru")?, Some(explicit));
    assert_eq!(
        library.cached_translation_image(&key)?.as_deref(),
        Some(white.as_slice())
    );
    library.reset_translation_selection("ru")?;

    let mut wrong_page = evidence.clone();
    wrong_page.image_observation.as_mut().unwrap().page = 1;
    assert!(
        library
            .save_image_translation(
                key.clone(),
                context.revision,
                black,
                additions.clone(),
                &[wrong_page],
            )
            .is_err()
    );
    let accepted = library.save_image_translation(
        key.clone(),
        context.revision,
        black,
        additions,
        &[evidence],
    )?;
    assert_eq!(accepted.revision, context.revision + 1);
    assert!(
        library
            .save_image_translation(
                key.clone(),
                context.revision,
                white,
                ContextAdditions::default(),
                &[],
            )
            .is_err()
    );
    assert_eq!(library.translation_context(publication, "ru")?, accepted);
    assert_eq!(
        library.cached_translation_image(&key)?.as_deref(),
        Some(black.as_slice())
    );

    for corrupt in [b"not a PNG".as_slice(), oversized.as_slice()] {
        connection.execute(
            "UPDATE translated_images SET png = ?1 WHERE key = ?2",
            rusqlite::params![corrupt, encoded_key],
        )?;
        assert!(matches!(
            library.cached_translation_image(&key),
            Err(LibraryError::CorruptEntry(_))
        ));
    }
    connection.execute(
        "UPDATE translated_images SET png = ?1 WHERE key = ?2",
        rusqlite::params![black.as_slice(), encoded_key],
    )?;
    drop(connection);
    drop(library);

    let library = Library::open(profile)?;
    assert_eq!(
        library.cached_translation_image(&key)?.as_deref(),
        Some(black.as_slice())
    );
    assert_eq!(library.cached_translation::<serde_json::Value>(&key)?, json);
    assert_eq!(library.entries(LibraryView::History)?, vec![history]);
    assert_eq!(library.translation_context(publication, "ru")?, accepted);
    assert_eq!(
        library.translation_selection("ru")?,
        Some(TranslationSelection {
            target_language: key.target_language.clone(),
            model: key.model.clone(),
            effort: key.effort.clone(),
            explicit: false,
        })
    );
    let text_key = cache_key(original, "ru", "codex", "gpt-6.1-sol", "low")?;
    let cached = library
        .cached_translation(&text_key)?
        .ok_or("text cache vanished")?;
    assert_eq!(restore_cached(original, cached)?, *translated);
    assert_eq!(library.reader_settings()?.translation_language, "en");
    println!(
        "PASS image_cache v10_migration blob_bytes exact_reopen key_miss invalid_png oversized_png sqlite_rollback selection_rollback stale_context corrupt_read original_history_json_independent"
    );
    Ok(())
}
