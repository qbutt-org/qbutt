//! Generated original chapters through the production parser and bounded cache.
//! No source page, live profile or translation service is used.

use qbutt_content::{
    ChapterRef, Error, PublicationRef, Source, TextBlockKind, TextChapter, TextChapterCache,
    TextSpan, VariantRef, VariantSelector,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let body = r#"
        <h2>A chapter written for this fixture</h2>
        <p>Original &amp; <u><strong>bold</strong> <em>italic</em></u><br>Next line.</p>
        <p><ruby>物語<rp>(</rp><rt>ものがたり</rt><rp>)</rp></ruby> stays original.</p>
        <p>&lt;img src='https://invalid.test/pixel'&gt;</p>
        <script>forbidden script</script><iframe>forbidden frame</iframe>
        <p hidden>hidden text</p><p style='display: none !important'>hidden style</p>
        <img src='https://invalid.test/pixel' onerror='forbidden handler'>
        <hr><p>Last original paragraph.</p>
    "#;
    let chapter = TextChapter::from_html(reference(1), body)?;
    chapter.validate()?;
    assert_eq!(chapter.blocks.len(), 6);
    assert_eq!(chapter.blocks[0].kind, TextBlockKind::Heading);
    assert_eq!(
        chapter.blocks[1].plain_text(),
        "Original & bold italic\nNext line."
    );
    assert!(
        chapter.blocks[1]
            .spans
            .iter()
            .any(|span| matches!(span, TextSpan::Text { bold: true, .. }))
    );
    assert!(
        chapter.blocks[1]
            .spans
            .iter()
            .any(|span| matches!(span, TextSpan::Text { italic: true, .. }))
    );
    assert_eq!(
        chapter.blocks[2].plain_text(),
        "物語(ものがたり) stays original."
    );
    assert_eq!(
        chapter.blocks[3].plain_text(),
        "<img src='https://invalid.test/pixel'>"
    );
    assert_eq!(chapter.blocks[4].kind, TextBlockKind::Separator);
    let original = serde_json::to_vec(&chapter)?;
    assert!(!String::from_utf8_lossy(&original).contains("forbidden"));
    assert!(!String::from_utf8_lossy(&original).contains("hidden text"));
    assert_eq!(serde_json::from_slice::<TextChapter>(&original)?, chapter);
    assert!(chapter.blocks[1].spans.iter().any(|span| matches!(
        span,
        TextSpan::Text {
            bold: true,
            underline: true,
            ..
        }
    )));
    assert!(chapter.blocks[1].spans.iter().any(|span| matches!(
        span,
        TextSpan::Text {
            italic: true,
            underline: true,
            ..
        }
    )));
    let unstyled_underline =
        TextChapter::from_html(reference(1), &body.replace("<u>", "").replace("</u>", ""))?;
    assert_eq!(chapter.blocks[1].id, unstyled_underline.blocks[1].id);

    let inserted = TextChapter::from_html(
        reference(1),
        &format!("<p>Inserted before the story.</p>{body}"),
    )?;
    assert_eq!(inserted.blocks[3].id, chapter.blocks[2].id);
    let duplicates = TextChapter::from_html(reference(2), "<p>Same.</p><p>Same.</p>")?;
    assert_ne!(duplicates.blocks[0].id, duplicates.blocks[1].id);
    let repeated = TextChapter::from_html(reference(2), "<p>Same.</p><p>Same.</p>")?;
    assert_eq!(duplicates, repeated);

    let mut cache = TextChapterCache::default();
    cache.insert(chapter.clone())?;
    for id in 2..=3 {
        cache.insert(TextChapter::from_html(
            reference(id),
            "<p>Generated chapter.</p>",
        )?)?;
    }
    assert_eq!(cache.get(&reference(1)).unwrap().as_ref(), &chapter);
    cache.insert(TextChapter::from_html(
        reference(4),
        "<p>Fourth generated chapter.</p>",
    )?)?;
    assert!(cache.get(&reference(2)).is_none());
    assert!(cache.get(&reference(1)).is_some());
    cache.clear();
    assert!(cache.get(&reference(1)).is_none());
    assert_eq!(serde_json::to_vec(&chapter)?, original);

    assert!(matches!(
        TextChapter::from_html(reference(1), &"x".repeat(4 * 1024 * 1024 + 1)),
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        TextChapter::from_html(
            reference(1),
            &format!("<p>{}</p>", "x".repeat(2 * 1024 * 1024 + 1))
        ),
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        TextChapter::from_html(reference(1), &"<p>x</p>".repeat(20_001)),
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        TextChapter::from_html(
            reference(1),
            &format!("{}text{}", "<div>".repeat(130), "</div>".repeat(130))
        ),
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        TextChapter::from_html(reference(1), "<script>no story</script>"),
        Err(Error::InvalidResponse(_))
    ));
    let mut duplicate_anchor = chapter.clone();
    duplicate_anchor.blocks[1].id = duplicate_anchor.blocks[0].id.clone();
    assert!(cache.insert(duplicate_anchor).is_err());
    println!(
        "PASS original_text html=discarded ruby=retained emphasis=retained anchors=stable cache=3 bounds=rejected"
    );
    Ok(())
}

fn reference(id: u64) -> ChapterRef {
    // A generated typed identity; this fixture never requests the provider.
    ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::Kakuyomu,
                id: "1001".into(),
            },
            selector: VariantSelector::OriginalText,
        },
        id: format!("1001/{id}"),
    }
}
