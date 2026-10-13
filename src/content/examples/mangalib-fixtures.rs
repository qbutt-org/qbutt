//! Generated legal payload faults compiled against the production DTO and HTTP
//! guard files, without provider URL overrides or a product test mode.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
#[cfg(windows)]
pub use qbutt_content::webview;

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;
#[allow(dead_code)]
#[path = "../src/http.rs"]
mod http;
#[path = "../src/mangalib/response.rs"]
mod response;

use serde_json::{Value, json};
use url::Url;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let selected = ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::MangaLib,
                id: "5432--generated".into(),
            },
            selector: VariantSelector::LibTranslation { branch: None },
        },
        id: "7".into(),
    };
    selected.validate()?;
    let listing: response::ChapterRecord = serde_json::from_value(json!({
        "index": 1, "volume": "1", "number": "1", "name": "Generated chapter",
        "branches": [{"id": 7, "branch_id": null, "teams": [{"name": "Generated translation"}]}]
    }))?;
    listing.validate()?;
    let valid = json!({
        "id": 7, "manga_id": 5432, "branch_id": null, "volume": "1", "number": "1",
        "pages": [
            {"id": 71, "chapter_id": 7, "slug": 1, "url": "//manga/generated/chapters/7/1.jpg", "external": 0, "chunks": 0},
            {"id": 72, "chapter_id": 7, "slug": 2, "url": "//manga/generated/chapters/7/2.jpg", "external": 0, "chunks": 0}
        ]
    });
    let server = Url::parse("https://img3.cdnlibs.org")?;
    let pages = decode(valid.clone(), &selected, &listing, &server)?;
    assert_eq!(pages.images.len(), 2);
    assert_eq!(
        pages.images[1].url().as_str(),
        "https://img3.cdnlibs.org/manga/generated/chapters/7/2.jpg"
    );

    for (field, value) in [
        ("id", json!(8)),
        ("manga_id", json!(5433)),
        ("branch_id", json!(9)),
        ("volume", json!("2")),
        ("number", json!("2")),
    ] {
        let mut changed = valid.clone();
        changed[field] = value;
        assert!(matches!(
            decode(changed, &selected, &listing, &server),
            Err(Error::VariantUnavailable)
        ));
    }
    for (field, value) in [
        ("id", json!(71)),
        ("chapter_id", json!(8)),
        ("slug", json!(1)),
        ("external", json!(1)),
        ("chunks", json!(1)),
    ] {
        let mut changed = valid.clone();
        changed["pages"][1][field] = value;
        assert!(matches!(
            decode(changed, &selected, &listing, &server),
            Err(Error::InvalidResponse(_))
        ));
    }
    let mut duplicate = valid.clone();
    duplicate["pages"][1]["url"] = duplicate["pages"][0]["url"].clone();
    assert!(matches!(
        decode(duplicate, &selected, &listing, &server),
        Err(Error::InvalidResponse(_))
    ));
    for address in [
        "https://example.org/manga/generated/chapters/7/2.jpg",
        "https://img3.cdnlibs.org/manga/generated/chapters/8/2.jpg",
        "//manga/generated/chapters/7/../../other/2.jpg",
        "https://img3.cdnlibs.org:444/manga/generated/chapters/7/2.jpg",
        "http://img3.cdnlibs.org/manga/generated/chapters/7/2.jpg",
    ] {
        let mut changed = valid.clone();
        changed["pages"][1]["url"] = json!(address);
        assert!(matches!(
            decode(changed, &selected, &listing, &server),
            Err(Error::UnsafeAddress)
        ));
    }
    let constants: response::Constants = serde_json::from_value(json!({
        "imageServers": [{"id": "download", "site_ids": [1], "url": "https://example.org"}]
    }))?;
    assert!(matches!(
        constants.download_server(),
        Err(Error::UnsafeAddress)
    ));

    let metadata: response::MangaRecord = serde_json::from_value(json!({
        "id": 5432, "slug_url": "5432--generated", "name": "Original", "rus_name": "Generated title", "eng_name": "",
        "model": "manga", "site": 1, "summary": "<p>Generated <strong>description</strong>.</p>"
    }))?;
    assert_eq!(
        metadata.into_work(false)?.description.as_deref(),
        Some("Generated description.")
    );
    let client = http::SourceHttp::new(
        Url::parse("https://api.cdnlibs.org")?,
        reqwest::header::HeaderMap::new(),
    )?;
    assert!(matches!(
        client
            .image(
                Url::parse("https://example.org/image.jpg")?,
                response::IMAGE_HOSTS
            )
            .await,
        Err(Error::UnsafeAddress)
    ));
    println!(
        "PASS MangaLib generated exact_null_branch/upload/manga/volume/number/page_order/duplicate_ids/duplicate_URLs guards; foreign_CDN/path_traversal/port/http/server rejected before acquisition"
    );
    Ok(())
}

fn decode(
    value: Value,
    reference: &ChapterRef,
    chapter: &response::ChapterRecord,
    server: &Url,
) -> Result<ChapterPages> {
    let response: response::Response<response::ChapterResponse> =
        serde_json::from_value(json!({ "data": value })).map_err(Error::Json)?;
    response.data.validate(reference, chapter)?;
    response.data.into_pages(reference.clone(), server)
}
