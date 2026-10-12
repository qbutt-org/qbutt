//! Generated legal metadata and text exercise the production parser and guards.
//! No copyrighted chapter, live profile or test-only provider endpoint is used.

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[allow(dead_code)]
#[path = "../src/http.rs"]
mod http;
#[allow(dead_code)]
#[path = "../src/model.rs"]
mod model;
#[path = "../src/ranobelib/response.rs"]
mod response;
#[allow(dead_code)]
#[path = "../src/text.rs"]
mod text;

pub use model::*;
pub use qbutt_content::{Error, Result};
pub use text::{TextBlock, TextBlockKind, TextChapter, TextSpan};

use serde_json::{Value, json};
use url::Url;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let selected = ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::RanobeLib,
                id: "42--generated".into(),
            },
            selector: VariantSelector::LibTranslation { branch: None },
        },
        id: "7".into(),
    };
    let record = listing();
    let valid = json!({
        "id": 7, "manga_id": 42, "model": "chapter", "branch_id": null,
        "volume": "1", "number": "2", "expired_type": 0, "bundle_id": null,
        "content": {"type": "doc", "content": [
            {"type": "heading", "content": [{"type": "text", "text": "Generated heading"}]},
            {"type": "paragraph"},
            {"type": "paragraph", "content": [
                {"type": "text", "text": "Original ", "marks": [{"type": "italic"}]},
                {"type": "text", "text": "emphasis", "marks": [{"type": "bold"}]},
                {"type": "hardBreak"},
                {"type": "text", "text": "<script>literal source text</script>"}
            ]},
            {"type": "image", "attrs": {"images": [{"image": "untrusted asset"}]}},
            {"type": "horizontalRule"},
            {"type": "paragraph", "content": [{"type": "text", "text": "Generated note"}]}
        ]}
    });
    let chapter = decode(valid.clone(), &selected, &record)?;
    assert_eq!(chapter.reference, selected);
    assert_eq!(chapter.blocks.len(), 4);
    assert_eq!(chapter.blocks[0].kind, TextBlockKind::Heading);
    assert_eq!(chapter.blocks[2].kind, TextBlockKind::Separator);
    assert!(matches!(
        &chapter.blocks[1].spans[0],
        TextSpan::Text {
            italic: true,
            bold: false,
            ..
        }
    ));
    assert!(matches!(
        &chapter.blocks[1].spans[1],
        TextSpan::Text {
            italic: false,
            bold: true,
            ..
        }
    ));
    assert_eq!(
        chapter.blocks[1].plain_text(),
        "Original emphasis\n<script>literal source text</script>"
    );
    assert_eq!(chapter, decode(valid.clone(), &selected, &record)?);
    let mut inserted = valid.clone();
    inserted["content"]["content"]
        .as_array_mut()
        .unwrap()
        .insert(
            0,
            json!({"type": "paragraph", "content": [{"type": "text", "text": "New introduction"}]}),
        );
    assert_eq!(
        chapter.blocks[1].id,
        decode(inserted, &selected, &record)?.blocks[2].id
    );

    for (field, value) in [
        ("id", json!(8)),
        ("manga_id", json!(43)),
        ("branch_id", json!(9)),
        ("volume", json!("2")),
        ("number", json!("3")),
        ("model", json!("manga")),
        ("expired_type", json!(1)),
        ("bundle_id", json!(1)),
        ("content", Value::Null),
    ] {
        let mut changed = valid.clone();
        changed[field] = value;
        assert!(matches!(
            decode(changed, &selected, &record),
            Err(Error::VariantUnavailable)
        ));
    }
    for body in [
        json!({"type": "doc", "content": []}),
        json!({"type": "doc", "content": [{"type": "paragraph"}]}),
        json!({"type": "doc", "content": [{"type": "image"}]}),
        json!("<script>hidden</script><img src='https://127.0.0.1/private'>"),
    ] {
        let mut changed = valid.clone();
        changed["content"] = body;
        assert!(decode(changed, &selected, &record).is_err());
    }
    for body in [
        json!({"type": "not-a-document", "content": []}),
        json!({"type": "doc", "content": [{"type": "unknown", "text": "Do not drop text"}]}),
        json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "Unsupported mark", "marks": [{"type": "unknown"}]}
        ]}]}),
    ] {
        let mut changed = valid.clone();
        changed["content"] = body;
        assert!(decode(changed, &selected, &record).is_err());
    }
    let mut html = valid.clone();
    html["content"] = json!(concat!(
        "<h3>Generated HTML heading</h3><p>Safe <b>bold</b> and <i>italic</i>.</p>",
        "<script>must disappear</script><p hidden>must disappear</p>",
        "<img src='https://127.0.0.1/private'><hr><p>Generated note</p>"
    ));
    let parsed = decode(html, &selected, &record)?;
    assert_eq!(parsed.blocks.len(), 4);
    assert_eq!(parsed.blocks[0].kind, TextBlockKind::Heading);
    assert!(
        !parsed
            .blocks
            .iter()
            .any(|block| block.plain_text().contains("must disappear"))
    );
    for body in [
        json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "x".repeat(2 * 1024 * 1024 + 1)}
        ]}]}),
        json!({"type": "doc", "content": vec![json!({"type": "horizontalRule"}); 20_001]}),
    ] {
        let mut changed = valid.clone();
        changed["content"] = body;
        assert!(matches!(
            decode(changed, &selected, &record),
            Err(Error::ResponseTooLarge)
        ));
    }

    let listing = response::validate_listing(vec![serde_json::from_value(record.clone())?])?;
    assert_eq!(listing[0].public_branches().count(), 1);
    for (field, value) in [
        ("expired_type", json!(1)),
        ("branch_id", json!(0)),
        ("id", json!(0)),
    ] {
        let mut changed = record.clone();
        changed["branches"][0][field] = value;
        let result = response::validate_listing(vec![serde_json::from_value(changed)?]);
        if field == "expired_type" {
            assert_eq!(result?.first().unwrap().public_branches().count(), 0);
        } else {
            assert!(result.is_err());
        }
    }
    let mut paid = record.clone();
    paid["bundle_id"] = json!(1);
    assert_eq!(
        response::validate_listing(vec![serde_json::from_value(paid)?])?[0]
            .public_branches()
            .count(),
        0
    );
    assert!(
        response::validate_listing(vec![
            serde_json::from_value(record.clone())?,
            serde_json::from_value(record.clone())?
        ])
        .is_err()
    );
    let mut duplicate = record.clone();
    duplicate["index"] = json!(2);
    assert!(
        response::validate_listing(vec![
            serde_json::from_value(record.clone())?,
            serde_json::from_value(duplicate)?
        ])
        .is_err()
    );
    assert!(
        response::validate_listing(
            (0..10_001)
                .map(|_| serde_json::from_value(record.clone()).unwrap())
                .collect()
        )
        .is_err()
    );

    let metadata = json!({
        "id": 42, "slug_url": "42--generated", "name": "Original title",
        "rus_name": "Generated title", "eng_name": "", "model": "manga", "site": 3,
        "summary": {"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "Generated description"}
        ]}]}
    });
    let work: response::BookRecord = serde_json::from_value(metadata.clone())?;
    let work = work.into_work(false)?;
    assert_eq!(work.description.as_deref(), Some("Generated description"));
    assert_eq!(work.category, Category::Books);
    assert_eq!(
        work.url.as_str(),
        "https://ranobelib.me/ru/book/42--generated"
    );
    for address in [
        "http://cover.cdnlibs.org/a.jpg",
        "https://127.0.0.1/a.jpg",
        "https://example.org/a.jpg",
        "https://cover.cdnlibs.org:444/a.jpg",
        "https://cover.cdnlibs.org/a.jpg?secret=value",
    ] {
        let mut changed = metadata.clone();
        changed["cover"] = json!({"default": address});
        let book: response::BookRecord = serde_json::from_value(changed)?;
        assert!(matches!(book.into_work(false), Err(Error::UnsafeAddress)));
    }
    let mut foreign = metadata;
    foreign["site"] = json!(1);
    let book: response::BookRecord = serde_json::from_value(foreign)?;
    assert!(book.into_work(false).is_err());
    let client = http::SourceHttp::new(
        Url::parse("https://api.cdnlibs.org")?,
        reqwest::header::HeaderMap::new(),
    )?;
    for address in [
        "https://example.org/api/manga",
        "https://127.0.0.1/api/manga",
        "http://api.cdnlibs.org/api/manga",
    ] {
        assert!(matches!(
            client.json::<Value>(Url::parse(address)?).await,
            Err(Error::UnsafeAddress)
        ));
    }
    let mut saved = serde_json::to_value(&work)?;
    saved["variants"] = json!([{
        "reference": selected.variant, "transport": "text", "languages": [],
        "reported_video_ads": false
    }]);
    let saved: qbutt_content::Work = serde_json::from_value(saved)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let profile =
        std::env::temp_dir().join(format!("qbutt-ranobelib-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&profile)?;
    let persisted = (|| -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut library = qbutt_content::Library::open(&profile)?;
        library.set_favorite(&saved, true)?;
        assert!(library.source_settings()?.iter().any(|setting| {
            setting.source == qbutt_content::Source::RanobeLib && setting.enabled
        }));
        drop(library);
        let library = qbutt_content::Library::open(&profile)?;
        let entry = library
            .get(&saved.reference)?
            .ok_or("saved text work missing")?;
        assert_eq!(entry.work, saved);
        assert_eq!(
            entry.work.variants[0].transport,
            qbutt_content::Transport::Text
        );
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    persisted?;
    println!(
        "PASS RanobeLib generated exact_publication/branch/upload/volume/number ownership; JSON/HTML text and emphasis; stable anchors; empty/unknown/paid/missing/oversize rejected; unsafe origins rejected before I/O"
    );
    println!(
        "PASS RanobeLib source registration and exact text variant survive isolated Library reopen"
    );
    Ok(())
}

fn listing() -> Value {
    json!({
        "index": 1, "volume": "1", "number": "2", "name": "Generated chapter", "bundle_id": null,
        "branches": [{"id": 7, "branch_id": null, "expired_type": 0, "teams": []}]
    })
}

fn decode(value: Value, selected: &ChapterRef, listing: &Value) -> Result<TextChapter> {
    let response: response::Response<response::ChapterResponse> =
        serde_json::from_value(json!({"data": value})).map_err(Error::Json)?;
    let records = response::validate_listing(vec![
        serde_json::from_value(listing.clone()).map_err(Error::Json)?,
    ])?;
    response.data.into_text(selected, &records[0])
}
