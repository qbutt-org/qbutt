use std::time::Duration;

use serde_json::json;
use url::Url;

use super::browser::{BrowserBroker, BrowserChoice, BrowserError};
use super::fixture::search_page;
use super::{Category, Error, browser_fixture, x1337};

pub(super) async fn run(
    source: &x1337::X1337,
    origin: &Url,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("qbutt-1337x-broker-{}", std::process::id()));
    std::fs::create_dir(&root)?;
    let discovery = root.join("discovery.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let handle = broker.handle();
    let mut edge =
        browser_fixture::extension(&discovery, "edge", "1337x-generated-agent", false).await?;
    let mut status = handle.subscribe();
    tokio::time::timeout(Duration::from_secs(3), async {
        while status.borrow().connections.is_empty() {
            status.changed().await.map_err(|_| BrowserError::Stopped)?;
        }
        Ok::<_, BrowserError>(())
    })
    .await??;

    for fault in [
        "none",
        "anime_empty",
        "books_empty",
        "unknown_empty",
        "foreign",
        "mime",
        "status",
        "challenge",
    ] {
        let (category, category_name) = match fault {
            "anime_empty" | "unknown_empty" => (Category::Anime, "Anime"),
            "books_empty" => (Category::Books, "Other"),
            _ => (Category::Movies, "Movies"),
        };
        let operation = source.search("browser", category, Some(&handle), BrowserChoice::Edge);
        let responses = async {
            let request = browser_fixture::read(&mut edge).await?;
            assert_eq!(request["method"], "fetch");
            assert_eq!(request["url"], origin.join("srch?search=browser")?.as_str());
            let arrived = if fault == "foreign" {
                Url::parse("https://ads.example/search/browser/1/")?
            } else {
                origin.join("search/browser/1/")?
            };
            let body = if fault == "challenge" {
                "<div id='cf-challenge'>Complete check</div>".into()
            } else {
                search_page("browser", "Movies")
            };
            browser_fixture::write(&mut edge, &json!({"id":request["id"],"result":{
                "status":if fault == "status" { 502 } else { 200 },
                "headers":{"content-type":if fault == "mime" { "application/json" } else { "text/html; charset=windows-1251" }},
                "body":body,"url":arrived
            }})).await?;
            if matches!(
                fault,
                "none" | "anime_empty" | "books_empty" | "unknown_empty"
            ) {
                let request = browser_fixture::read(&mut edge).await?;
                assert_eq!(request["method"], "fetch");
                assert_eq!(
                    request["url"],
                    origin
                        .join(&format!("category-search/browser/{category_name}/1/"))?
                        .as_str()
                );
                let body = match fault {
                    "anime_empty" | "books_empty" => search_page("category empty", category_name),
                    "unknown_empty" => search_page("unknown message", category_name),
                    _ => search_page("browser", category_name),
                };
                browser_fixture::write(
                    &mut edge,
                    &json!({"id":request["id"],"result":{
                        "status":200,"headers":{"content-type":"text/html"},
                        "body":body,"url":request["url"]
                    }}),
                )
                .await?;
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (result, responded) = tokio::join!(operation, responses);
        responded?;
        match fault {
            "none" => assert_eq!(result?.len(), 2),
            "anime_empty" | "books_empty" => assert!(result?.is_empty()),
            "challenge" => assert!(matches!(
                result,
                Err(Error::SourceCheck(
                    super::webview::WebViewError::Unavailable
                ))
            )),
            "status" => assert!(matches!(result, Err(Error::HttpStatus(502)))),
            _ => assert!(matches!(result, Err(Error::InvalidResponse(_)))),
        }
    }

    let mut pending = Box::pin(source.search(
        "browser",
        Category::Movies,
        Some(&handle),
        BrowserChoice::Edge,
    ));
    let request = tokio::select! {
        request = browser_fixture::read(&mut edge) => request?,
        result = &mut pending => panic!("browser request ended before cancellation: {result:?}"),
    };
    drop(pending);
    let cancel = browser_fixture::read(&mut edge).await?;
    assert_eq!(cancel["method"], "cancel");
    assert_eq!(cancel["id"], request["id"]);

    let mut chrome =
        browser_fixture::extension(&discovery, "chrome", "1337x-generated-chrome", false).await?;
    tokio::time::timeout(Duration::from_secs(3), async {
        while status.borrow().connections.len() != 2 {
            status.changed().await.map_err(|_| BrowserError::Stopped)?;
        }
        Ok::<_, BrowserError>(())
    })
    .await??;
    assert!(matches!(
        source
            .search(
                "browser",
                Category::Movies,
                Some(&handle),
                BrowserChoice::Auto
            )
            .await,
        Err(Error::SourceCheck(
            super::webview::WebViewError::Unavailable
        ))
    ));
    broker.shutdown().await?;
    browser_fixture::closed(&mut edge).await?;
    browser_fixture::closed(&mut chrome).await?;
    drop(edge);
    drop(chrome);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
