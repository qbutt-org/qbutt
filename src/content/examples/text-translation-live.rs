//! Generated legal chapter with a later context clue. Uses the installed Codex
//! account by default. --openrouter explicitly provisions the environment key
//! into a fresh DPAPI profile, runs one job and removes that saved key.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use qbutt_content::translation::context::TranslationContext;
use qbutt_content::translation::text::{
    TextTranslationOptions, TextTranslationProvider, cache_key, restore_cached, translate_chapter,
};
use qbutt_content::{
    Chapter, ChapterRef, Library, LibraryError, OpenRouterSettings, PublicationRef, Source,
    TextBlockKind, TextChapter, TextImageAsset, TextSpan, TextTranslationService,
    TranslationCacheKey, VariantRef, VariantSelector,
};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [] => exercise(TextTranslationProvider::Codex, None).await,
        [operation, profile] if operation == "--reopen" => reopen(Path::new(profile)),
        [operation, model, effort, profile] if operation == "--openrouter" => {
            let profile = PathBuf::from(profile);
            if !profile.is_absolute() || profile.exists() {
                return Err("OpenRouter requires a new absolute evidence directory".into());
            }
            let options = OpenRouterSettings {
                model: model.clone(),
                effort: effort.clone(),
            };
            if !options.valid() {
                return Err("OpenRouter model or reasoning effort is invalid".into());
            }
            std::fs::create_dir(&profile)?;
            let mut library = Library::open(&profile)?;
            let key = std::env::var("OPENROUTER_API_KEY")
                .map_err(|_| "Explicit live fixture requires OPENROUTER_API_KEY")?;
            library.configure_text_translation(
                TextTranslationService::OpenRouter,
                &options,
                Some(&key),
            )?;
            drop(key);
            let provider = TextTranslationProvider::OpenRouter(library.openrouter_client()?);
            drop(library);
            let started = Instant::now();
            let result = exercise(provider, Some(&profile)).await;
            let failure_receipt = result.as_ref().err().map(|error| {
                serde_json::json!({
                    "provider": "openrouter", "model": options.model, "effort": options.effort,
                    "elapsed_ms": started.elapsed().as_millis(), "outcome": "failed",
                    "diagnostic": error.to_string(),
                })
            });
            let mut library = Library::open(&profile)?;
            library.configure_text_translation(
                TextTranslationService::OpenRouter,
                &options,
                Some(""),
            )?;
            drop(library);
            if let Some(receipt) = failure_receipt {
                std::fs::write(
                    profile.join("receipt.json"),
                    serde_json::to_vec_pretty(&receipt)?,
                )?;
            }
            result?;
            let mut process = Command::new(std::env::current_exe()?);
            process
                .arg("--reopen")
                .arg(&profile)
                .env_remove("OPENROUTER_API_KEY");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                process.creation_flags(0x0800_0000);
            }
            if !process.status()?.success() {
                return Err("Live OpenRouter offline process reopen failed".into());
            }
            Ok(())
        }
        _ => Err(
            "Use --openrouter MODEL EFFORT NEW_EVIDENCE_DIRECTORY, or no arguments for Codex"
                .into(),
        ),
    }
}

fn reopen(profile: &Path) -> Result<()> {
    let library = Library::open(profile)?;
    let original = authored_original()?;
    let stored_original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join("original.json"))?)?;
    assert_eq!(serde_json::to_value(&original)?, stored_original);
    let key: TranslationCacheKey =
        serde_json::from_slice(&std::fs::read(profile.join("cache-key.json"))?)?;
    let translated: TextChapter =
        serde_json::from_slice(&std::fs::read(profile.join("translated.json"))?)?;
    let context: TranslationContext =
        serde_json::from_slice(&std::fs::read(profile.join("context.json"))?)?;
    assert!(!library.text_translation_settings()?.key_configured);
    assert!(matches!(
        library.openrouter_client(),
        Err(LibraryError::Credentials(_))
    ));
    let cached: TextChapter = library
        .cached_translation(&key)?
        .ok_or("offline cache missing")?;
    assert_eq!(
        restore_cached(&original, cached)?,
        restore_cached(&original, translated)?
    );
    assert_eq!(
        library.translation_context(&key.chapter.variant.publication, &key.target_language)?,
        context
    );
    println!(
        "PASS openrouter_live offline_process_reopen original context no_saved_key no_inference"
    );
    Ok(())
}

fn authored_original() -> Result<TextChapter> {
    let image_bytes = base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9QkQAAAABJRU5ErkJggg==",
    )?;
    let image = TextImageAsset {
        id: format!("{:x}", Sha256::digest(&image_bytes)),
        media_type: "image/png".into(),
        bytes: image_bytes.into(),
    };
    TextChapter::from_html_with_images(
        reference(1),
        "<h2 id='entry'>レンの旅</h2><p><strong>レン</strong>は<em>森</em>に<u>戻った</u>。</p><img src='scene.png'><p>『また会えたね』とレンは言った。</p>",
        &mut |_| Ok(Some(image.clone())),
        Some("entry"),
        None,
    ).map_err(Into::into)
}

async fn exercise(provider: TextTranslationProvider, saved_profile: Option<&Path>) -> Result<()> {
    let original = authored_original()?;
    assert_eq!(
        original.entry_anchor.as_deref(),
        Some(original.blocks[0].id.as_str())
    );
    let later = TextChapter::from_html(
        reference(7),
        "<p>レンは妹に言った。『私はあなたの姉です』</p>",
    )?;
    let chapters = [chapter(1, "Ren's journey"), chapter(7, "A later encounter")];
    let fetched = Arc::new(AtomicUsize::new(0));
    let source = {
        let fetched = Arc::clone(&fetched);
        move |reference: ChapterRef| {
            let fetched = Arc::clone(&fetched);
            let later = later.clone();
            async move {
                fetched.fetch_add(1, Ordering::SeqCst);
                if reference == later.reference {
                    Ok(later)
                } else {
                    Err(qbutt_content::Error::VariantUnavailable)
                }
            }
        }
    };
    let (_sender, cancelled) = tokio::sync::watch::channel(false);
    let started = Instant::now();
    let result = translate_chapter(
        &original,
        &chapters,
        &source,
        &TranslationContext::default(),
        &TextTranslationOptions {
            target_language: "ru".into(),
            model: None,
            effort: None,
        },
        &provider,
        cancelled,
    )
    .await?;
    let translation_ms = started.elapsed().as_millis();
    assert!(
        fetched.load(Ordering::SeqCst) > 0,
        "later chapter was not inspected"
    );
    assert!(
        result.additions.facts.iter().any(|fact| {
            fact.attribute == "gender"
                && fact.value == "female"
                && fact.evidence.chapter == reference(7)
        }),
        "later chapter's gender evidence was not retained"
    );
    assert_eq!(result.chapter.reference, original.reference);
    assert_eq!(result.chapter.blocks.len(), original.blocks.len());
    assert!(
        result
            .chapter
            .blocks
            .iter()
            .zip(&original.blocks)
            .all(
                |(translated, source)| translated.id == source.id && translated.kind == source.kind
            )
    );
    assert_eq!(original.blocks[1].plain_text(), "レンは森に戻った。");
    let image_block = original
        .blocks
        .iter()
        .position(|block| block.kind == TextBlockKind::Image)
        .unwrap();
    assert_eq!(
        result.chapter.blocks[image_block],
        original.blocks[image_block]
    );
    assert_eq!(result.chapter.assets, original.assets);
    let cached = serde_json::from_slice(&serde_json::to_vec(&result.chapter)?)?;
    let restored = restore_cached(&original, cached)?;
    assert_eq!(restored, result.chapter);

    for emphasis in [0, 1, 2] {
        assert!(
            result
                .chapter
                .blocks
                .iter()
                .flat_map(|block| &block.spans)
                .any(|span| {
                    matches!(span, TextSpan::Text { bold, italic, underline, .. }
                if [*bold, *italic, *underline][emphasis])
                }),
            "semantic emphasis was lost"
        );
    }
    let name = result
        .additions
        .terms
        .iter()
        .find(|term| term.source == "レン")
        .ok_or("confirmed translated name missing")?;
    assert!(
        result
            .chapter
            .blocks
            .iter()
            .any(|block| block.plain_text().contains(&name.target))
    );
    let provider_name = match &provider {
        TextTranslationProvider::Codex => "codex",
        TextTranslationProvider::OpenRouter(_) => "openrouter",
    };
    let profile = if let Some(profile) = saved_profile {
        profile.to_path_buf()
    } else {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let profile = std::env::temp_dir().join(format!("qbutt-text-translation-{nonce}"));
        std::fs::create_dir(&profile)?;
        profile
    };
    let persisted = (|| -> Result<()> {
        let mut library = Library::open(&profile)?;
        let key = cache_key(
            &original,
            "ru",
            provider_name,
            &result.model,
            &result.effort,
        )?;
        let context = library.save_translation(
            key,
            0,
            &result.chapter,
            result.additions.clone(),
            &result.evidence,
            || true,
        )?;
        drop(library);
        let library = Library::open(&profile)?;
        let loaded = library.translation_context(&original.reference.variant.publication, "ru")?;
        assert_eq!(loaded, context);
        let key = cache_key(
            &original,
            "ru",
            provider_name,
            &result.model,
            &result.effort,
        )?;
        let saved: TextChapter = library
            .cached_translation(&key)?
            .ok_or("translation cache is empty")?;
        assert_eq!(restore_cached(&original, saved)?, result.chapter);
        if saved_profile.is_some() {
            for (file, value) in [
                ("original.json", serde_json::to_value(&original)?),
                ("cache-key.json", serde_json::to_value(&key)?),
                ("translated.json", serde_json::to_value(&result.chapter)?),
                ("context.json", serde_json::to_value(&context)?),
                (
                    "receipt.json",
                    serde_json::json!({
                        "provider": provider_name, "model": result.model, "effort": result.effort,
                        "translation_ms": translation_ms, "context_fetches": fetched.load(Ordering::SeqCst),
                        "blocks": result.chapter.blocks.len(), "terms": result.additions.terms.len(),
                        "facts": result.additions.facts.len(), "usage": result.usage,
                    }),
                ),
            ] {
                std::fs::write(profile.join(file), serde_json::to_vec_pretty(&value)?)?;
            }
        }
        Ok(())
    })();
    if saved_profile.is_none() {
        std::fs::remove_dir_all(&profile)?;
    }
    persisted?;
    println!(
        "PASS original_text_translation provider={} model={} effort={} translation_ms={} context_fetches={} blocks={} terms={} facts={} tokens={}",
        provider_name,
        result.model,
        result.effort,
        translation_ms,
        fetched.load(Ordering::SeqCst),
        result.chapter.blocks.len(),
        result.additions.terms.len(),
        result.additions.facts.len(),
        result.usage.total_tokens,
    );
    for block in result.chapter.blocks {
        println!("{}: {}", block.id, block.plain_text());
    }
    Ok(())
}

fn chapter(number: u64, title: &str) -> Chapter {
    Chapter {
        reference: reference(number),
        title: Some(title.into()),
        volume: None,
        number: Some(number.to_string()),
        pages: 0,
    }
}

fn reference(number: u64) -> ChapterRef {
    ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::Kakuyomu,
                id: "1001".into(),
            },
            selector: VariantSelector::OriginalText,
        },
        id: format!("1001/{number}"),
    }
}
