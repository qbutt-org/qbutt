//! Standalone generated source-response acceptance. The same DTO/guard/model
//! files are compiled here so fault injection needs no product constructor,
//! URL override, alternate transport or unit-test harness.

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
#[path = "../src/mangadex/response.rs"]
mod response;

use serde_json::{Value, json};
use url::Url;

const MANGA: &str = "b0b721ff-c388-4486-aa0f-c2b0bb321512";
const CHAPTER: &str = "a1105d32-6fd4-4b42-bf29-3b69e6a6ffab";
const GROUP: &str = "bfd9705d-415a-4bee-b910-a60b683ca6bc";

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let selected = ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::MangaDex,
                id: MANGA.into(),
            },
            selector: VariantSelector::MangaTranslation {
                language: "ru".into(),
                groups: vec![GROUP.into()],
            },
        },
        id: CHAPTER.into(),
    };
    selected.validate()?;
    let manga: response::MangaRecord = serde_json::from_value(manga_value())?;
    let work = manga.into_work("ru", true)?;
    assert_eq!(work.title, "Созданная манга");
    assert_eq!(work.aliases, vec!["Generated manga".to_string()]);
    assert!(
        work.cover
            .as_ref()
            .unwrap()
            .path()
            .ends_with("cover.png.256.jpg")
    );
    assert!(work.needs_inspection && work.variants.is_empty());
    let chapter: response::ChapterRecord = serde_json::from_value(chapter_value())?;
    chapter.matches(&selected)?;
    assert_eq!(
        chapter.branch()?.translation.as_deref(),
        Some("Generated group")
    );
    let chapter = chapter.into_chapter(selected.variant.clone())?;
    assert!(chapter.reference == selected);
    assert_eq!(chapter.number.as_deref(), Some("1.5"));
    assert_eq!(chapter.pages, 2);

    let pages = manifest(manifest_value(), &selected, 2)?;
    assert_eq!(pages.images.len(), 2);
    assert_eq!(
        pages.images[0].url().as_str(),
        "https://example.org/session-token/data/0123456789abcdef0123456789abcdef/1-page.png"
    );
    assert_eq!(
        pages.images[1].url().path(),
        "/session-token/data/0123456789abcdef0123456789abcdef/2-page.jpg"
    );

    for id in [
        MANGA.to_uppercase(),
        format!("{MANGA}/extra"),
        "00000000-0000-0000-0000-000000000000".into(),
        "1".into(),
    ] {
        let invalid = PublicationRef {
            source: Source::MangaDex,
            id,
        };
        assert!(matches!(invalid.validate(), Err(Error::InvalidReference)));
    }
    let mut wrong_source = selected.variant.clone();
    wrong_source.publication = PublicationRef {
        source: Source::AniLiberty,
        id: "1".into(),
    };
    assert!(matches!(
        wrong_source.validate(),
        Err(Error::InvalidReference)
    ));
    for (language, groups) in [
        ("RU", vec![GROUP.into()]),
        ("ru", vec![GROUP.into(), GROUP.into()]),
        ("ru", vec![GROUP.into(), CHAPTER.into()]),
        ("ru", vec!["not-a-uuid".into()]),
    ] {
        let invalid = VariantRef {
            publication: selected.variant.publication.clone(),
            selector: VariantSelector::MangaTranslation {
                language: language.into(),
                groups,
            },
        };
        assert!(matches!(invalid.validate(), Err(Error::InvalidReference)));
    }
    for field in [
        "language",
        "groups",
        "manga",
        "unavailable",
        "external",
        "identity",
    ] {
        let mut value = chapter_value();
        match field {
            "language" => value["attributes"]["translatedLanguage"] = json!("en"),
            "groups" => value["relationships"][0]["id"] = json!(CHAPTER),
            "manga" => value["relationships"][1]["id"] = json!(CHAPTER),
            "unavailable" => value["attributes"]["isUnavailable"] = json!(true),
            "external" => value["attributes"]["externalUrl"] = json!("https://example.org"),
            "identity" => value["id"] = json!(MANGA),
            _ => unreachable!(),
        }
        let record: response::ChapterRecord = serde_json::from_value(value)?;
        assert!(
            record.matches(&selected).is_err(),
            "accepted foreign/unavailable chapter: {field}"
        );
    }
    for base in [
        "http://example.org",
        "https://127.0.0.1",
        "https://10.0.0.1",
        "https://[::1]",
        "https://[::ffff:127.0.0.1]",
        "https://user:pass@example.org",
        "https://example.org/?token=credential",
        "https://example.org/#fragment",
    ] {
        let mut value = manifest_value();
        value["baseUrl"] = json!(base);
        assert!(
            manifest(value, &selected, 2).is_err(),
            "accepted unsafe manifest origin"
        );
    }
    for filename in [
        "../page.png",
        "%2fpage.png",
        "/page.png",
        "page.png?token=secret",
        "..",
        "",
        "page\\file.png",
    ] {
        let mut value = manifest_value();
        value["chapter"]["data"][0] = json!(filename);
        assert!(
            manifest(value, &selected, 2).is_err(),
            "accepted unsafe page filename"
        );
    }
    let mut duplicated = manifest_value();
    duplicated["chapter"]["data"][1] = duplicated["chapter"]["data"][0].clone();
    assert!(manifest(duplicated, &selected, 2).is_err());
    let mut invalid_hash = manifest_value();
    invalid_hash["chapter"]["hash"] = json!("../escape");
    assert!(manifest(invalid_hash, &selected, 2).is_err());
    assert!(manifest(manifest_value(), &selected, 1).is_err());
    let collection: response::Collection<Value> = serde_json::from_value(
        json!({ "result":"ok", "data":[1,2], "limit":500, "offset":0, "total":1 }),
    )?;
    assert!(collection.validate(0, 500).is_err());
    let failed: response::EntityResponse<Value> =
        serde_json::from_value(json!({ "result":"error", "data":{} }))?;
    assert!(failed.entity().is_err());

    let mut oversized = manga_value();
    oversized["attributes"]["title"]["en"] = json!("a".repeat(2049));
    assert!(
        serde_json::from_value::<response::MangaRecord>(oversized)?
            .into_work("ru", true)
            .is_err()
    );
    let mut malformed = chapter_value();
    malformed["attributes"]["pages"] = json!(2001);
    assert!(
        serde_json::from_value::<response::ChapterRecord>(malformed)?
            .matches(&selected)
            .is_err()
    );

    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let client = http::SourceHttp::new(
        Url::parse("https://api.mangadex.org")?,
        reqwest::header::HeaderMap::new(),
    )?;
    let local = Url::parse(&format!("https://127.0.0.1:{port}/image.png"))?;
    assert!(matches!(
        client.image(local, &[]).await,
        Err(Error::UnsafeAddress)
    ));
    let local_domain = Url::parse(&format!("https://localhost:{port}/image.png"))?;
    match client.image(local_domain, &[]).await {
        Err(Error::Network(error)) => assert!(error.url().is_none()),
        _ => return Err("public resolver did not reject a local DNS result".into()),
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(matches!(
        client
            .json::<Value>(Url::parse("https://example.org/private")?)
            .await,
        Err(Error::UnsafeAddress)
    ));

    let mut ordered = [chapter.clone(), chapter.clone(), chapter.clone()];
    ordered[0].number = Some("10".into());
    ordered[1].number = Some("2".into());
    ordered[2].number = Some("1.5".into());
    ordered.sort_by(response::compare_chapters);
    assert_eq!(
        ordered
            .iter()
            .map(|chapter| chapter.number.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["1.5", "2", "10"]
    );
    ordered[0].number = Some("Special".into());
    ordered[1].number = Some("2".into());
    ordered[2].number = Some("10".into());
    ordered.sort_by(response::compare_chapters);
    assert_eq!(
        ordered
            .iter()
            .map(|chapter| chapter.number.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["2", "10", "Special"]
    );
    println!(
        "PASS mangadex-fixtures localized_metadata=ok canonical_source_refs=ok exact_chapter_binding=ok bounded_metadata=ok unsafe_manifest=rejected unsafe_page_path=rejected local_ip_and_dns=rejected no_local_connection=ok fixed_api_origin=ok token_path=preserved numeric_chapter_order=ok"
    );
    Ok(())
}

fn manifest(value: Value, selected: &ChapterRef, pages: u32) -> Result<ChapterPages> {
    let response: response::AtHome = serde_json::from_value(value).map_err(Error::Json)?;
    response.into_pages(selected.clone(), pages)
}

fn manga_value() -> Value {
    json!({
        "id": MANGA,
        "type": "manga",
        "attributes": {
            "title": { "en": "Generated manga" },
            "altTitles": [{ "ru": "Созданная манга" }],
            "description": { "en": "Generated legal fixture" },
            "year": 2026,
            "availableTranslatedLanguages": ["ru", "en"]
        },
        "relationships": [{
            "id": CHAPTER,
            "type": "cover_art",
            "attributes": { "fileName": "cover.png" }
        }]
    })
}

fn chapter_value() -> Value {
    json!({
        "id": CHAPTER,
        "type": "chapter",
        "attributes": {
            "volume": "1",
            "chapter": "1.5",
            "title": "Generated chapter",
            "translatedLanguage": "ru",
            "pages": 2,
            "externalUrl": null,
            "isUnavailable": false
        },
        "relationships": [
            { "id": GROUP, "type": "scanlation_group", "attributes": { "name": "Generated group" } },
            { "id": MANGA, "type": "manga" }
        ]
    })
}

fn manifest_value() -> Value {
    json!({
        "result": "ok",
        "baseUrl": "https://example.org:443/session-token",
        "chapter": {
            "hash": "0123456789abcdef0123456789abcdef",
            "data": ["1-page.png", "2-page.jpg"]
        }
    })
}
