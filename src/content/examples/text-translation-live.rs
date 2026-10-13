//! Generated legal chapter with a later context clue. Uses the installed Codex
//! account and never reads or changes a real reader profile.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use qbutt_content::translation::context::TranslationContext;
use qbutt_content::translation::text::{
    TextTranslationOptions, cache_key, restore_cached, translate_chapter,
};
use qbutt_content::{
    Chapter, ChapterRef, Library, PublicationRef, Source, TextBlockKind, TextChapter,
    TextImageAsset, VariantRef, VariantSelector,
};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let image_bytes = base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9QkQAAAABJRU5ErkJggg==",
    )?;
    let image = TextImageAsset {
        id: format!("{:x}", Sha256::digest(&image_bytes)),
        media_type: "image/png".into(),
        bytes: image_bytes.into(),
    };
    let original = TextChapter::from_html_with_images(
        reference(1),
        "<h2 id='entry'>レンの旅</h2><p>レンは森に戻った。</p><img src='scene.png'><p>『また会えたね』とレンは言った。</p>",
        &mut |_| Ok(Some(image.clone())),
        Some("entry"),
        None,
    )?;
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
        cancelled,
    )
    .await?;
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

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-text-translation-{nonce}"));
    std::fs::create_dir(&profile)?;
    let persisted = (|| -> Result<()> {
        let mut library = Library::open(&profile)?;
        let key = cache_key(&original, "ru", &result.model, &result.effort)?;
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
        let key = cache_key(&original, "ru", &result.model, &result.effort)?;
        let saved: TextChapter = library
            .cached_translation(&key)?
            .ok_or("translation cache is empty")?;
        assert_eq!(restore_cached(&original, saved)?, result.chapter);
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    persisted?;
    println!(
        "PASS original_text_translation model={} effort={} context_fetches={} blocks={} terms={} facts={} tokens={}",
        result.model,
        result.effort,
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
