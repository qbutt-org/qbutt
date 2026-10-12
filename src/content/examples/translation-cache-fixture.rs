//! Generated chapter cache and v8 profile migration without a Codex call.

use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::translation::context::ContextAdditions;
use qbutt_content::translation::text::{cache_key, restore_cached};
use qbutt_content::{
    ChapterRef, Library, PublicationRef, Source, TextChapter, TextSpan, TranslationSelection,
    VariantRef, VariantSelector,
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
        connection.execute_batch("DROP TABLE translation_selection; PRAGMA user_version = 8;")?;
        drop(connection);

        let mut library = Library::open(&profile)?;
        assert!(library.translation_selection("ru")?.is_none());
        assert!(library.cached_translation::<TextChapter>(&key)?.is_some());

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
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    result?;
    println!("PASS translation_cache exact_hit model_miss effort_miss language_miss v8_migration");
    Ok(())
}
