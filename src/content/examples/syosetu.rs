//! Live public search, paginated table of contents, nonfirst text and one-shot work.
//! No chapter text or source cookies are printed.

use std::error::Error;

use qbutt_content::{
    ChapterRef, PublicationRef, Source, Syosetu, TextBlockKind, TextSpan, VariantRef,
    VariantSelector,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let source = Syosetu::new()?;
    let reference = PublicationRef {
        source: Source::Syosetu,
        id: "n2267be".into(),
    };
    let found = source.search("Ｒｅ：ゼロから始める異世界生活").await?;
    assert!(found.iter().any(|work| work.reference == reference));

    let work = source.inspect(&reference).await?;
    let variant = &work.variants[0].reference;
    assert_eq!(variant.selector, VariantSelector::OriginalText);
    assert_eq!(work.url.as_str(), "https://ncode.syosetu.com/n2267be/");

    let chapters = source.chapters(variant).await?;
    assert!(
        chapters.len() > 100,
        "paginated table of contents was incomplete"
    );
    assert_eq!(chapters[1].reference.id, "2");
    let text = source.chapter_text(&chapters[1].reference).await?;
    assert_eq!(text.reference, chapters[1].reference);
    assert!(text.text_bytes() > 5_000);
    assert!(
        text.blocks
            .iter()
            .flat_map(|block| &block.spans)
            .any(|span| { matches!(span, TextSpan::Ruby { .. }) })
    );

    let short = PublicationRef {
        source: Source::Syosetu,
        id: "n5917je".into(),
    };
    let short_work = source.inspect(&short).await?;
    let short_chapters = source.chapters(&short_work.variants[0].reference).await?;
    assert_eq!(short_chapters.len(), 1);
    assert_eq!(short_chapters[0].reference.id, "root");
    let short_text = source.chapter_text(&short_chapters[0].reference).await?;
    assert!(short_text.text_bytes() > 1_000);

    let notes = source.chapter_text(&chapter_ref("n4830bu", "1")).await?;
    assert!(
        notes
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Separator)
    );
    assert!(notes.text_bytes() > 1_000);

    let illustrated = source.chapter_text(&chapter_ref("n7653fr", "39")).await?;
    assert_eq!(illustrated.assets.len(), 1);
    assert!(illustrated.assets[0].bytes.len() > 1_000);
    assert!(
        illustrated
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Image)
    );
    assert!(
        illustrated
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Separator)
    );

    println!(
        "PASS Syosetu live search/inspect/paginated TOC/nonfirst ruby text/short work/author notes/inline image; chapters={} text_bytes={} short_bytes={} notes_bytes={} image_bytes={}",
        chapters.len(),
        text.text_bytes(),
        short_text.text_bytes(),
        notes.text_bytes(),
        illustrated.assets[0].bytes.len()
    );
    Ok(())
}

fn chapter_ref(publication: &str, chapter: &str) -> ChapterRef {
    ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::Syosetu,
                id: publication.into(),
            },
            selector: VariantSelector::OriginalText,
        },
        id: chapter.into(),
    }
}
