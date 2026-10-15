use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use tokio::io::AsyncReadExt;
use url::Url;

use super::browser::{BrowserBroker, BrowserChoice, BrowserError};
use super::fixture::{Fixture, HASH};
use super::{
    Category, Error, PublicationRef, Source, SourceOperation, VariantRef, VariantSelector,
    browser_fixture, http, webview, x1337,
};

type CheckResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Controlled executable peer of the production WebView protocol. It never
/// creates a browser or reads a real profile.
pub(super) fn helper() -> CheckResult {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let request: serde_json::Value = serde_json::from_str(&line)?;
    let profile = Path::new(request["profileDirectory"].as_str().unwrap());
    let root = profile.parent().unwrap().parent().unwrap();
    assert_eq!(
        profile.parent().unwrap().file_name().unwrap(),
        "web-session"
    );
    let mode = request["mode"].as_str().unwrap();
    assert!(matches!(mode, "silent" | "interactive"));
    assert_eq!(
        request["timeoutMs"],
        if mode == "silent" { 15_000 } else { 120_000 }
    );
    let url = Url::parse(request["url"].as_str().unwrap())?;
    let query = url
        .query_pairs()
        .find(|(name, _)| name == "search")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("checks.jsonl"))?;
    writeln!(
        log,
        "{}",
        json!({"mode":mode,"url":url,"profile":profile,"configured":request["configuredSession"]})
    )?;
    if query.starts_with("webview cancel ") {
        let port: u16 = query.split_whitespace().last().unwrap().parse()?;
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", port))?;
        socket.write_all(&[1])?;
        socket.read_exact(&mut [0])?;
        return Err("canceled helper survived".into());
    }
    let manual_publication =
        url.path().starts_with("/torrent/58/") || url.path().starts_with("/torrent/59/");
    let error = match query.as_str() {
        "webview manual" if mode == "silent" => Some("browser_check_required"),
        _ if manual_publication && mode == "silent" => Some("browser_check_required"),
        "webview fault" => Some("webview_failed"),
        _ => None,
    };
    let response = if let Some(code) = error {
        json!({"id":request["id"],"error":{"code":code}})
    } else {
        let returned = if query == "webview foreign" {
            Url::parse("https://ads.example/")?
        } else {
            url
        };
        let expires = (query == "webview expiry").then(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64()
                + 0.5
        });
        json!({"id":request["id"],"session":{
            "url":returned,"userAgent":"fixture-webview","cookies":[{
                "name":"clearance","value":"fixture-pass","domain":"fixture.example",
                "path":"/","secure":true,"httpOnly":true,"hostOnly":true,"expires":expires
            }]
        }})
    };
    println!("{response}");
    Ok(())
}

pub(super) async fn run(fixture: &Fixture) -> CheckResult {
    let root = std::env::temp_dir().join(format!("qbutt-1337x-check-{}", std::process::id()));
    let app = root.join("app");
    std::fs::create_dir_all(&app)?;
    // The hardlink is only an immutable fixture executable, never writable data.
    std::fs::hard_link(std::env::current_exe()?, app.join("qbutt-webview.exe"))?;
    let new_source = || {
        let source = x1337::fixture(
            http::fixture(fixture.client.clone(), fixture.origin.clone()),
            fixture.origin.clone(),
        );
        source.initialize_webview(root.clone(), app.clone());
        source
    };
    let checks = || -> CheckResult<Vec<serde_json::Value>> {
        Ok(std::fs::read_to_string(root.join("checks.jsonl"))?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?)
    };
    for (query, hits) in [
        ("webview", 4),
        ("webview category", 6),
        ("webview persistent", 2),
        ("webview manual", 1),
        ("webview fault", 1),
        ("webview foreign", 1),
    ] {
        let source = new_source();
        let input = if query == "webview manual" {
            "webview-&-manual"
        } else {
            query
        };
        let before = fixture.hits.load(Ordering::SeqCst);
        let result = source
            .search(input, Category::Movies, None, BrowserChoice::Auto)
            .await;
        assert_eq!(
            fixture.hits.load(Ordering::SeqCst) - before,
            hits,
            "{query}"
        );
        match query {
            "webview" | "webview category" => {
                assert_eq!(result?.len(), 2);
                let count = checks()?.len();
                assert_eq!(
                    source
                        .search(query, Category::Movies, None, BrowserChoice::Auto)
                        .await?
                        .len(),
                    2
                );
                assert_eq!(checks()?.len(), count, "reuse clearance and matching UA");
            }
            "webview persistent" => assert!(matches!(result, Err(Error::SourceCheckRequired))),
            "webview manual" => {
                assert!(matches!(result, Err(Error::SourceCheckRequired)));
                source
                    .complete_check(&SourceOperation::Search {
                        source: Source::X1337,
                        query: input.into(),
                        category: Category::Movies,
                        language: None,
                    })
                    .await?;
                let records = checks()?;
                let silent = &records[records.len() - 2];
                let manual = &records[records.len() - 1];
                assert_eq!(silent["mode"], "silent");
                assert_eq!(manual["mode"], "interactive");
                assert_eq!(silent["url"], manual["url"]);
                assert_eq!(
                    manual["url"],
                    fixture.origin.join("srch?search=webview+manual")?.as_str()
                );
                assert_eq!(
                    source
                        .search(input, Category::Movies, None, BrowserChoice::Auto)
                        .await?
                        .len(),
                    2
                );
            }
            "webview fault" => assert!(matches!(
                result,
                Err(Error::SourceCheck(webview::WebViewError::Failed))
            )),
            "webview foreign" => assert!(matches!(result, Err(Error::UnsafeAddress))),
            _ => unreachable!(),
        }
        source.close();
    }
    let source = new_source();
    let publication = PublicationRef {
        source: Source::X1337,
        id: "57/Generated-Legal".into(),
    };
    let work = source
        .inspect(&publication, None, BrowserChoice::Auto)
        .await?;
    let count = checks()?.len();
    source
        .resolve(&work.variants[0].reference, None, BrowserChoice::Auto)
        .await?;
    assert_eq!(checks()?.len(), count, "exact release reuses clearance");
    source.close();

    for (id, resolve) in [("58/Manual-Inspect", false), ("59/Manual-Resolve", true)] {
        let source = new_source();
        let publication = PublicationRef {
            source: Source::X1337,
            id: id.into(),
        };
        let selected = VariantRef {
            publication: publication.clone(),
            selector: VariantSelector::TorrentHash {
                info_hash: HASH.into(),
            },
        };
        let operation = if resolve {
            SourceOperation::Resolve(selected.clone())
        } else {
            SourceOperation::Inspect {
                reference: publication.clone(),
                language: None,
            }
        };
        let result = if resolve {
            source
                .resolve(&selected, None, BrowserChoice::Auto)
                .await
                .map(|_| ())
        } else {
            source
                .inspect(&publication, None, BrowserChoice::Auto)
                .await
                .map(|_| ())
        };
        assert!(matches!(result, Err(Error::SourceCheckRequired)));
        source.complete_check(&operation).await?;
        let records = checks()?;
        let silent = &records[records.len() - 2];
        let manual = &records[records.len() - 1];
        assert_eq!(silent["mode"], "silent");
        assert_eq!(manual["mode"], "interactive");
        assert_eq!(silent["url"], manual["url"]);
        assert_eq!(
            manual["url"],
            fixture.origin.join(&format!("torrent/{id}/"))?.as_str()
        );
        if resolve {
            let super::ResolvedContent::Torrent { reference, .. } =
                source.resolve(&selected, None, BrowserChoice::Auto).await?
            else {
                panic!("unexpected acquisition");
            };
            assert_eq!(reference, selected);
        } else {
            assert_eq!(
                source
                    .inspect(&publication, None, BrowserChoice::Auto)
                    .await?
                    .reference,
                publication
            );
        }
        assert_eq!(
            checks()?.len(),
            records.len(),
            "manual clearance continues the same operation"
        );
        source.close();
    }

    let source = new_source();
    source
        .search(
            "webview expiry",
            Category::Movies,
            None,
            BrowserChoice::Auto,
        )
        .await?;
    let count = checks()?.len();
    tokio::time::sleep(Duration::from_millis(600)).await;
    source
        .search(
            "webview expiry",
            Category::Movies,
            None,
            BrowserChoice::Auto,
        )
        .await?;
    assert_eq!(
        checks()?.len(),
        count + 1,
        "expired clearance must be checked again"
    );
    source.close();
    assert!(matches!(
        source
            .search("webview", Category::Movies, None, BrowserChoice::Auto)
            .await,
        Err(Error::SourceCheck(webview::WebViewError::Stopped))
    ));
    assert_eq!(checks()?.len(), count + 1, "close retires the helper owner");

    let discovery = root.join("discovery.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let handle = broker.handle();
    let mut edge = browser_fixture::extension(&discovery, "edge", "fixture-edge", false).await?;
    let mut chrome =
        browser_fixture::extension(&discovery, "chrome", "fixture-chrome", false).await?;
    let mut status = handle.subscribe();
    tokio::time::timeout(Duration::from_secs(3), async {
        while status.borrow().connections.len() != 2 {
            status.changed().await.map_err(|_| BrowserError::Stopped)?;
        }
        Ok::<_, BrowserError>(())
    })
    .await??;
    let source = new_source();
    assert_eq!(
        source
            .search(
                "webview",
                Category::Movies,
                Some(&handle),
                BrowserChoice::Auto
            )
            .await?
            .len(),
        2,
        "ambiguous connected browsers must not block the owned source check"
    );
    source.close();
    broker.shutdown().await?;
    browser_fixture::closed(&mut edge).await?;
    browser_fixture::closed(&mut chrome).await?;

    let source = new_source();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let query = format!("webview cancel {}", listener.local_addr()?.port());
    let mut pending = Box::pin(source.search(&query, Category::Movies, None, BrowserChoice::Auto));
    let mut child = tokio::select! {
        socket = tokio::time::timeout(Duration::from_secs(5), listener.accept()) => socket??.0,
        result = &mut pending => panic!("helper ended before cancellation: {result:?}"),
    };
    assert_eq!(child.read_u8().await?, 1);
    drop(pending);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), child.read_u8())
            .await?
            .is_err(),
        "helper process must close its socket"
    );
    source.close();
    let records = checks()?;
    assert_eq!(
        records
            .iter()
            .filter(|record| record["mode"] == "interactive")
            .count(),
        3,
        "only explicit complete_check may show a window"
    );
    assert!(
        records
            .iter()
            .all(|record| record["profile"] == records[0]["profile"]
                && record["configured"] == records[0]["configured"])
    );
    std::fs::remove_file(app.join("qbutt-webview.exe"))?;
    std::fs::remove_file(root.join("checks.jsonl"))?;
    std::fs::remove_dir(app)?;
    std::fs::remove_dir(root)?;
    println!(
        "PASS 1337x_owned_check silent=ok original_search_replayed=ok category_replayed=ok cookie_ua_cache=ok expiry=refreshed ambiguous_browser=owned_check persistent=manual_required explicit_manual=exact_search_inspect_resolve same_profile=ok helper_fault=typed foreign_origin=rejected cancel=helper_closed close=retired"
    );
    Ok(())
}
