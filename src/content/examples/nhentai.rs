// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit metadata/HEAD probe. Never downloads a public gallery's image bodies.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qbutt_content::{Nhentai, PublicationRef, Source};
use serde_json::json;
use sha2::{Digest, Sha256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let id = args
        .next()
        .ok_or("usage: nhentai <public-gallery-id> <receipt.json>")?;
    let receipt = args
        .next()
        .ok_or("missing receipt path outside tracked source")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let reference = PublicationRef {
        source: Source::Nhentai,
        id,
    };
    reference.validate()?;
    let provider = Nhentai::new()?;
    assert!(
        provider
            .search("qbuttgeneratedunlikelyemptyresult20261014")
            .await?
            .is_empty()
    );
    let found = provider.search("non-h -loli -shota").await?;
    assert!(!found.is_empty());
    let work = provider.inspect(&reference).await?;
    assert_eq!(work.reference, reference);
    let chapters = provider.chapters(&work.variants[0].reference).await?;
    assert_eq!(chapters.len(), 1);
    let pages = provider.chapter_pages(&chapters[0].reference).await?;
    assert_eq!(pages.images.len(), chapters[0].pages as usize);
    let second = pages
        .images
        .get(1)
        .ok_or("control gallery has no second page")?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(25))
        .user_agent("qbutt-content metadata probe")
        .build()?;
    let mut resources = Vec::new();
    for (kind, url) in [
        ("cover", work.cover.as_ref().ok_or("missing cover")?),
        ("page_2", second.url()),
    ] {
        let response = client
            .head(url.clone())
            .header(reqwest::header::REFERER, "https://nhentai.net/")
            .send()
            .await?;
        assert!(response.status().is_success(), "{kind} HEAD failed");
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .ok_or("missing image MIME")?;
        assert!(mime.starts_with("image/"));
        resources.push(json!({"kind": kind, "url": url, "method": "HEAD",
            "status": response.status().as_u16(), "content_type": mime}));
    }
    let document = json!({
        "checked_at_unix": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "publication": reference,
        "title_sha256": format!("{:x}", Sha256::digest(work.title.as_bytes())),
        "search_rows": found.len(), "empty_search": true,
        "variant": work.variants[0].reference, "chapter": chapters[0].reference,
        "pages": pages.images.len(), "resources": resources,
        "public_image_bodies_requested": 0,
        "live_reader_accepted": false, "gui_accepted": false
    });
    std::fs::write(receipt, serde_json::to_vec_pretty(&document)?)?;
    println!(
        "PASS nhentai live search/empty/inspect/exact gallery manifest/cover+page2 HEAD; image bodies=0; live reading and GUI unverified"
    );
    Ok(())
}
