//! Generated legal-format integration for the native local book parser.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use qbutt_content::{LocalBook, TextBlockKind, TextSpan};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args().nth(1).expect("fixture directory argument");
    let directory = Path::new(&directory);

    let epub = LocalBook::open(directory.join("native.epub"))?;
    assert_eq!(epub.title, "Native EPUB fixture");
    assert_eq!(epub.chapters.len(), 2);
    assert_eq!(epub.chapters[0].title, "Opening scene");
    assert_eq!(epub.chapters[1].title, "Closing scene");
    let first = epub.read_chapter(&epub.chapter_list()[0].reference)?;
    assert!(
        first
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Heading)
    );
    assert!(
        first
            .blocks
            .iter()
            .any(|block| matches!(block.kind, TextBlockKind::ListItem { ordinal: Some(1) }))
    );
    assert!(
        first
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Image)
    );
    assert!(
        first
            .blocks
            .iter()
            .flat_map(|block| &block.spans)
            .any(|span| matches!(span, TextSpan::Text { bold: true, .. }))
    );
    assert!(
        first
            .blocks
            .iter()
            .flat_map(|block| &block.spans)
            .any(|span| matches!(span, TextSpan::Ruby { .. }))
    );
    assert_eq!(first.assets.len(), 1);
    assert_eq!(first.assets[0].media_type, "image/png");
    let second = epub.read_chapter(&epub.chapter_list()[1].reference)?;
    assert!(
        second
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Quote)
    );

    let fragments = LocalBook::open(directory.join("fragments.epub"))?;
    assert_eq!(fragments.chapters.len(), 2);
    assert_eq!(fragments.chapters[1].title, "Closing fragment");
    let closing = fragments.read_chapter(&fragments.chapter_list()[1].reference)?;
    assert_eq!(closing.assets.len(), 1);
    assert_eq!(
        closing.entry_anchor.as_deref(),
        closing
            .blocks
            .iter()
            .find(|block| block.plain_text() == "Closing fragment")
            .map(|block| block.id.as_str())
    );

    let fb2 = LocalBook::open(directory.join("native.fb2"))?;
    assert_eq!(fb2.title, "Native FB2 fixture");
    assert_eq!(fb2.chapters.len(), 3);
    assert_eq!(fb2.chapters[1].title, "Nested section");
    let first = fb2.read_chapter(&fb2.chapter_list()[0].reference)?;
    assert!(
        first
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Heading)
    );
    assert_eq!(first.assets.len(), 1);
    assert!(
        !first
            .blocks
            .iter()
            .any(|block| block.plain_text() == "Nested text.")
    );
    let nested = fb2.read_chapter(&fb2.chapter_list()[1].reference)?;
    assert!(
        nested
            .blocks
            .iter()
            .any(|block| block.plain_text() == "Nested text.")
    );

    let txt = LocalBook::open(directory.join("native.txt"))?;
    assert_eq!(txt.chapters.len(), 1);
    let text = txt.read_chapter(&txt.chapter_list()[0].reference)?;
    assert_eq!(text.blocks.len(), 2);
    assert_eq!(text.blocks[0].plain_text(), "Opening line.\nAnother line.");

    assert!(LocalBook::open(directory.join("unsafe.epub")).is_err());
    let changed = directory.join("changed.txt");
    std::fs::copy(directory.join("native.txt"), &changed)?;
    let book = LocalBook::open(&changed)?;
    OpenOptions::new()
        .append(true)
        .open(&changed)?
        .write_all(b"changed")?;
    assert!(book.verify().is_err());
    assert!(book.read_chapter(&book.chapter_list()[0].reference).is_err());
    assert!(LocalBook::open(directory.join("bad-encoding.fb2")).is_err());

    println!("PASS local EPUB, FB2, TXT, changed-file and malformed-file checks");
    Ok(())
}
