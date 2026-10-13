//! Generated public fiction pages exercise exact identities and reader formatting.

pub use qbutt_content::*;

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
    assert_eq!(chapter.reference, book.chapters[1].reference);
    assert!(chapter.blocks[0].spans.iter().any(|span| {
        matches!(span, TextSpan::Ruby { text, reading } if text == "物語" && reading == "ものがたり")
    }));
    assert!(response::chapter(
        r#"<meta property="og:url" content="https://ncode.syosetu.com/n1234ab/1/"><div class="p-novel__text"><p>Wrong chapter</p></div>"#,
        &book.chapters[1].reference,
    )
    .is_err());

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
        "PASS Syosetu generated search/pagination/exact nonfirst ruby chapter/one-shot and identity rejection"
    );
    Ok(())
}

fn book_html(id: &str, content: &str) -> String {
    format!(
        "<meta property=\"og:url\" content=\"https://ncode.syosetu.com/{id}/\"><h1 class=\"p-novel__title\">Generated fiction</h1>{content}"
    )
}
