//! Explicit live acceptance using qcatch's already authorized browser session.
//! Output contains public publication metadata, never acquisition descriptors.

#![cfg(windows)]

use std::path::PathBuf;
use std::time::Duration;

use qbutt_content::browser::{BrowserBroker, BrowserChoice};
use qbutt_content::{Category, MediaGateway, ResolvedContent, VkVideo};
use serde_json::json;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> Result {
    let query = std::env::args()
        .nth(1)
        .ok_or("Supply the public search query")?;
    let discovery = PathBuf::from(
        std::env::var_os("QBUTT_BROWSER_BRIDGE_DISCOVERY")
            .ok_or("Supply the isolated native broker discovery path")?,
    );
    let broker = BrowserBroker::start(discovery).await?;
    let browser = broker.handle();
    let result: Result = async {
        tokio::time::timeout(Duration::from_secs(45), async {
            while browser.status().connections.is_empty() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await?;
        let provider = VkVideo;
        let works = provider
            .search(&query, Category::Video, &browser, BrowserChoice::Edge)
            .await?;
        println!(
            "{}",
            serde_json::to_string(&json!({ "query": query, "works": works }))?
        );
        if let Ok(id) = std::env::var("QBUTT_VKVIDEO_PUBLICATION") {
            let selected = works
                .iter()
                .find(|work| work.reference.id == id)
                .ok_or("The approved publication is absent from this search")?;
            let ResolvedContent::BrowserVideo {
                reference, media, ..
            } = provider
                .resolve(
                    &selected.variants[0].reference,
                    &browser,
                    BrowserChoice::Edge,
                )
                .await?
            else {
                return Err("The selected publication did not resolve as browser video".into());
            };
            let (gateway, prepared) = MediaGateway::prepare(media).await?;
            let client = reqwest::Client::builder().no_proxy().build()?;
            for range in ["bytes=0-1023", "bytes=0-"] {
                let response = client
                    .get(prepared.url.clone())
                    .header("Range", range)
                    .send()
                    .await?;
                if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                    && !(range == "bytes=0-" && response.status() == reqwest::StatusCode::OK)
                {
                    return Err(format!(
                        "Guarded local range {range} returned {}",
                        response.status()
                    )
                    .into());
                }
            }
            drop(gateway);
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "reference": reference,
                    "guardedMp4Prefix": true,
                }))?
            );
        }
        Ok(())
    }
    .await;
    broker.shutdown().await?;
    result
}
