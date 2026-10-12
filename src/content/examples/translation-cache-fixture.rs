//! Generated chapter cache and v8 profile migration without a Codex call.

use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::translation::context::{
    ContextAdditions, ContextEvidence, EvidenceSegment, ImageObservation, TranslationTerm,
};
use qbutt_content::translation::text::{cache_key, restore_cached};
use qbutt_content::{
    ChapterRef, Library, PublicationRef, Source, TextChapter, TextSpan, TranslationCacheKey,
    TranslationSelection, VariantRef, VariantSelector,
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
    let original = TextChapter::from_html(reference, "<p>レンは帰った。</p>")?;
    let mut translated = original.clone();
    translated.blocks[0].spans = vec![TextSpan::Text {
        text: "Рен вернулась.".into(),
        bold: false,
        italic: false,
    }];

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-translation-cache-{nonce}"));
    let result = (|| -> Result<()> {
        let mut library = Library::open(&profile)?;
        let key = cache_key(&original, "ru", "gpt-6.1-sol", "low", 0)?;
        library.save_translation(key.clone(), &translated, ContextAdditions::default(), &[])?;
        drop(library);

        // A v8 profile already has translated content but no selected model.
        let connection = rusqlite::Connection::open(profile.join("content-library.sqlite"))?;
        connection.execute_batch(
            "ALTER TABLE reader_settings DROP COLUMN translation_language;
             DROP TABLE translation_selection;
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
        let exact = cache_key(&original, "ru", &saved.model, &saved.effort, 0)?;
        let chapter: TextChapter = library
            .cached_translation(&exact)?
            .ok_or("exact translation missing")?;
        assert_eq!(restore_cached(&original, chapter)?, translated);
        for (language, model, effort) in [
            ("ru", "another-model", "low"),
            ("ru", "gpt-6.1-sol", "medium"),
            ("en", "gpt-6.1-sol", "low"),
        ] {
            let different = cache_key(&original, language, model, effort, 0)?;
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
        let different = cache_key(&original, "ru", "another-model", "low", 0)?;
        assert!(
            library
                .save_translation(different, &translated, ContextAdditions::default(), &[])
                .is_err()
        );
        assert_eq!(library.translation_selection("ru")?, Some(explicit));

        library.reset_translation_selection("ru")?;
        let automatic = library
            .translation_selection("ru")?
            .ok_or("automatic selection missing")?;
        assert!(!automatic.explicit);
        let offline_key = cache_key(&original, "ru", &automatic.model, &automatic.effort, 0)?;
        assert!(
            library
                .cached_translation::<TextChapter>(&offline_key)?
                .is_some()
        );

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
            context_revision: 0,
            pipeline: 2,
        };
        let additions = ContextAdditions {
            terms: vec![term.clone()],
            facts: vec![],
        };
        let accepted = library.save_translation(
            manga_key.clone(),
            &vec!["Фрирен"],
            additions,
            &[evidence.clone()],
        )?;
        assert_eq!(accepted.revision, 1);
        drop(library);

        let mut library = Library::open(&profile)?;
        let context = library.translation_context(&manga.variant.publication, "ru")?;
        assert_eq!(context.terms, vec![term.clone()]);
        let exact = TranslationCacheKey {
            context_revision: context.revision,
            ..manga_key.clone()
        };
        assert_eq!(
            library.cached_translation::<Vec<String>>(&exact)?,
            Some(vec!["Фрирен".into()])
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
            &vec!["Подожди"],
            ContextAdditions {
                terms: vec![forged],
                facts: vec![],
            },
            &[evidence.clone()],
        );
        assert!(rejected.is_err());
        let mut wrong_page = evidence;
        wrong_page.image_observation.as_mut().unwrap().page = 1;
        assert!(
            library
                .save_translation(
                    exact,
                    &vec!["Подожди"],
                    ContextAdditions::default(),
                    &[wrong_page]
                )
                .is_err()
        );
        assert_eq!(
            library.translation_context(&manga.variant.publication, "ru")?,
            context
        );
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    result?;
    println!(
        "PASS translation_cache exact_hit model_miss effort_miss language_miss v8_migration target_persistence automatic_offline manga_image_context"
    );
    Ok(())
}
