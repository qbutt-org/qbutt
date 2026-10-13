//! Generated public fiction pages exercise exact identities and reader formatting.

pub use qbutt_content::*;

use std::collections::HashMap;

use base64::Engine;
use sha2::{Digest, Sha256};

#[path = "../src/syosetu/response.rs"]
mod response;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let reference = PublicationRef {
        source: Source::Syosetu,
        id: "n1234ab".into(),
    };
    let search = response::search(
        r#"<div id="main_search"><div class="searchkekka_box"><div class="novel_h"><a class="tl" href="https://ncode.syosetu.com/n1234ab/">Generated fiction</a></div><div class="ex">An original description.</div></div></div>"#,
    )?;
    assert_eq!(search.len(), 1);
    assert_eq!(search[0].reference, reference);
    assert!(search[0].needs_inspection);
    assert!(response::search(r#"<div id="main_search"></div>"#)?.is_empty());

    let first = response::book_page(
        &book_html(
            "n1234ab",
            r#"<a class="c-pager__item--last" href="/n1234ab/?p=2">Last</a><div class="p-eplist__sublist"><a href="/n1234ab/1/">First chapter</a></div>"#,
        ),
        &reference,
        1,
        None,
    )?;
    assert_eq!(first.total_pages, 2);
    let second = response::book_page(
        &book_html(
            "n1234ab",
            r#"<div class="p-eplist__sublist"><a href="/n1234ab/2/">Second chapter</a></div>"#,
        ),
        &reference,
        2,
        Some(2),
    )?;
    let book = response::join_pages(vec![first, second])?;
    assert_eq!(book.chapters.len(), 2);
    assert_eq!(book.chapters[1].reference.id, "2");
    assert_eq!(book.chapters[1].number.as_deref(), Some("2"));
    assert!(!book.work.needs_inspection);
    assert_eq!(
        response::chapter_url(&book.chapters[1].reference).as_str(),
        "https://ncode.syosetu.com/n1234ab/2/"
    );

    let chapter = response::chapter(
        r#"<meta property="og:url" content="https://ncode.syosetu.com/n1234ab/2/"><div class="p-novel__text"><p>Second <ruby>物語<rt>ものがたり</rt></ruby> with <em>emphasis</em>.</p></div>"#,
        &book.chapters[1].reference,
    )?;
    let chapter = chapter.into_text(HashMap::new())?;
    assert_eq!(chapter.reference, book.chapters[1].reference);
    assert!(chapter.blocks[0].spans.iter().any(|span| {
        matches!(span, TextSpan::Ruby { text, reading } if text == "物語" && reading == "ものがたり")
    }));
    assert!(response::chapter(
        r#"<meta property="og:url" content="https://ncode.syosetu.com/n1234ab/1/"><div class="p-novel__text"><p>Wrong chapter</p></div>"#,
        &book.chapters[1].reference,
    )
    .is_err());

    let illustrated = response::chapter(
        r#"<meta property="og:url" content="https://ncode.syosetu.com/n1234ab/2/">
        <div class="p-novel__text p-novel__text--preface">
            <p>Author's preface.</p>
        </div>
        <div class="p-novel__text">
            <p>The story begins.</p>
            <p>
                <a href="https://1234.mitemin.net/i5678/">
                    <img src="//1234.mitemin.net/userpageimage/viewimagebig/icode/i5678/"
                        alt="Generated illustration">
                </a>
            </p>
            <p>The story continues.</p>
            <img src="https://1234.mitemin.net/userpageimage/viewimagebig/icode/i5678/"
                alt="Repeated illustration">
        </div>
        <div class="p-novel__text p-novel__text--afterword">
            <p>Author's afterword.</p>
        </div>
        <img src="https://unrelated.invalid/advertisement.png">"#,
        &book.chapters[1].reference,
    )?;
    assert_eq!(illustrated.images.len(), 1);
    let bytes = base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9QkQAAAABJRU5ErkJggg==",
    )?;
    let asset = TextImageAsset {
        id: format!("{:x}", Sha256::digest(&bytes)),
        media_type: "image/png".into(),
        bytes: bytes.into(),
    };
    let images = HashMap::from([(illustrated.images[0].clone(), asset)]);
    let illustrated = illustrated.into_text(images)?;
    assert_eq!(
        illustrated.blocks.first().unwrap().plain_text(),
        "Author's preface."
    );
    assert_eq!(
        illustrated.blocks.last().unwrap().plain_text(),
        "Author's afterword."
    );
    assert_eq!(
        illustrated
            .blocks
            .iter()
            .filter(|block| block.kind == TextBlockKind::Separator)
            .count(),
        2
    );
    assert_eq!(
        illustrated
            .blocks
            .iter()
            .filter(|block| block.kind == TextBlockKind::Image)
            .count(),
        2
    );
    assert_eq!(illustrated.assets.len(), 1);
    illustrated.validate()?;

    let missing_image = response::chapter(
        r#"<meta property="og:url" content="https://ncode.syosetu.com/n1234ab/2/">
        <div class="p-novel__text">
            <p>Story.</p>
            <img src="//1234.mitemin.net/userpageimage/viewimagebig/icode/i5678/">
        </div>"#,
        &book.chapters[1].reference,
    )?;
    assert!(missing_image.into_text(HashMap::new()).is_err());
    for source in [
        "https://example.invalid/userpageimage/viewimagebig/icode/i5678/",
        "https://1234.mitemin.net.evil.invalid/userpageimage/viewimagebig/icode/i5678/",
        "http://1234.mitemin.net/userpageimage/viewimagebig/icode/i5678/",
        "https://user@1234.mitemin.net/userpageimage/viewimagebig/icode/i5678/",
        "https://1234.mitemin.net/userpageimage/viewimagebig/icode/i5678/?private=value",
    ] {
        assert!(response::image_url(source).is_err());
    }
    assert!(
        response::chapter(
            r#"<meta property="og:url" content="https://ncode.syosetu.com/n1234ab/2/">
        <div class="p-novel__text"><p>Story.</p></div>
        <div class="p-novel__text"><p>Ambiguous story.</p></div>"#,
            &book.chapters[1].reference,
        )
        .is_err()
    );

    let excessive_images = (1..=65)
        .map(|id| format!("<img src='//1234.mitemin.net/userpageimage/viewimagebig/icode/i{id}/'>"))
        .collect::<String>();
    assert!(matches!(
        response::chapter(
            &format!(
                "<meta property='og:url' content='https://ncode.syosetu.com/n1234ab/2/'><div class='p-novel__text'>{excessive_images}</div>"
            ),
            &book.chapters[1].reference,
        ),
        Err(Error::ResponseTooLarge)
    ));

    let one_shot = PublicationRef {
        source: Source::Syosetu,
        id: "n5678cd".into(),
    };
    let short = response::book_page(
        &format!(
            "{}<div class=\"p-novel__text\"><p>Original short fiction.</p></div>",
            book_html("n5678cd", "")
        ),
        &one_shot,
        1,
        None,
    )?;
    assert_eq!(short.chapters[0].reference.id, "root");
    assert!(response::book_page(&book_html("n9999zz", ""), &one_shot, 1, None).is_err());

    println!(
        "PASS Syosetu generated search/pagination/exact ruby chapter/preface/afterword/inline image deduplication/one-shot and unsafe identity/image rejection"
    );
    Ok(())
}

fn book_html(id: &str, content: &str) -> String {
    format!(
        "<meta property=\"og:url\" content=\"https://ncode.syosetu.com/{id}/\"><h1 class=\"p-novel__title\">Generated fiction</h1>{content}"
    )
}
