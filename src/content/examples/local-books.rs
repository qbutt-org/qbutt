//! Generated legal-format integration for the native local book parser.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use qbutt_content::{Library, LocalBook, TextBlockKind, TextSpan, Transport};

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
    let opening = fragments.read_chapter(&fragments.chapter_list()[0].reference)?;
    assert!(
        opening
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Image)
    );
    assert!(
        !opening
            .blocks
            .iter()
            .any(|block| block.plain_text() == "Closing fragment")
    );
    let closing = fragments.read_chapter(&fragments.chapter_list()[1].reference)?;
    assert!(closing.assets.is_empty());
    assert!(
        !closing
            .blocks
            .iter()
            .any(|block| block.plain_text() == "Opening fragment")
    );
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

    let html = LocalBook::open(directory.join("html/local.html"))?;
    assert_eq!(html.work().variants[0].transport, Transport::Text);
    let chapter = html.read_chapter(&html.chapter_list()[0].reference)?;
    assert!(
        chapter
            .blocks
            .iter()
            .any(|block| block.kind == TextBlockKind::Heading)
    );
    assert!(
        chapter
            .blocks
            .iter()
            .any(|block| matches!(block.kind, TextBlockKind::ListItem { .. }))
    );
    assert!(
        chapter
            .blocks
            .iter()
            .flat_map(|block| &block.spans)
            .any(|span| matches!(span, TextSpan::Text { bold: true, .. }))
    );
    assert_eq!(chapter.assets.len(), 1);
    assert_eq!(chapter.assets[0].media_type, "image/png");
    assert!(
        !chapter
            .blocks
            .iter()
            .any(|block| block.plain_text().contains("doNotRender"))
    );

    for extension in ["cbz", "zip"] {
        let comic = LocalBook::open(directory.join(format!("native.{extension}")))?;
        assert_eq!(comic.work().variants[0].transport, Transport::Manga);
        assert_eq!(comic.chapter_list().len(), 1);
        assert_eq!(comic.chapter_list()[0].pages, 3);
        let pages = comic.chapter_pages(&comic.chapter_list()[0].reference)?;
        for (index, number) in [1, 2, 10].into_iter().enumerate() {
            assert_eq!(
                comic.image_bytes(&pages.images[index])?,
                std::fs::read(directory.join(format!("comic-page{number}.png")))?
            );
        }
    }
    let comic = LocalBook::open(directory.join("native.cbz"))?;
    let profile = directory.join("reader-profile");
    Library::open(&profile)?.save_local_book(&comic)?;
    let library = Library::open(&profile)?;
    assert_eq!(
        library.local_book_path(&comic.reference())?,
        Some(comic.path().to_path_buf())
    );
    for extension in ["png", "jpg", "webp", "gif", "bmp"] {
        let image = LocalBook::open(directory.join(format!("single.{extension}")))?;
        let chapter = &image.chapter_list()[0];
        assert_eq!(image.work().variants[0].transport, Transport::Manga);
        assert_eq!(chapter.pages, 1);
        let pages = image.chapter_pages(&chapter.reference)?;
        assert_eq!(
            image.image_bytes(&pages.images[0])?,
            std::fs::read(directory.join(format!("single.{extension}")))?
        );
    }
    for name in [
        "traversal.cbz",
        "duplicate.cbz",
        "encrypted.cbz",
        "oversized.cbz",
    ] {
        assert!(
            LocalBook::open(directory.join(name)).is_err(),
            "{name} accepted"
        );
    }

    let corrupt = LocalBook::open(directory.join("corrupt-page.cbz"))?;
    let pages = corrupt.chapter_pages(&corrupt.chapter_list()[0].reference)?;
    assert!(corrupt.image_bytes(&pages.images[0]).is_err());

    assert!(LocalBook::open(directory.join("unsafe.epub")).is_err());
    let changed = directory.join("changed.txt");
    std::fs::copy(directory.join("native.txt"), &changed)?;
    let book = LocalBook::open(&changed)?;
    OpenOptions::new()
        .append(true)
        .open(&changed)?
        .write_all(b"changed")?;
    assert!(book.verify().is_err());
    assert!(
        book.read_chapter(&book.chapter_list()[0].reference)
            .is_err()
    );
    assert!(LocalBook::open(directory.join("bad-encoding.fb2")).is_err());

    let changed_comic = directory.join("changed.cbz");
    std::fs::copy(directory.join("native.cbz"), &changed_comic)?;
    let comic = LocalBook::open(&changed_comic)?;
    let pages = comic.chapter_pages(&comic.chapter_list()[0].reference)?;
    OpenOptions::new()
        .append(true)
        .open(&changed_comic)?
        .write_all(b"changed")?;
    assert!(comic.image_bytes(&pages.images[0]).is_err());

    println!("PASS local books, comics, images, bounded HTML resources and archive faults");
    Ok(())
}
