use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use tokio::io::AsyncReadExt;
use url::Url;

use super::browser::{BrowserBroker, BrowserChoice, BrowserError};
use super::{Category, Error, Fixture, PublicationRef, ResolvedContent, Source};
use super::{browser_fixture, first_torrent, http, igruha, webview};

type CheckResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The production helper protocol runs against this controlled executable peer.
/// It creates no browser and reads no real profile.
pub(super) fn helper() -> CheckResult {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let request: serde_json::Value = serde_json::from_str(&line)?;
    let profile = Path::new(request["profileDirectory"].as_str().unwrap());
    let root = profile.parent().unwrap().parent().unwrap();
    let mode = request["mode"].as_str().unwrap();
    assert!(matches!(mode, "silent" | "interactive"));
    assert_eq!(
        request["timeoutMs"],
        if mode == "silent" { 15_000 } else { 120_000 }
    );
    let url = Url::parse(request["url"].as_str().unwrap())?;
    let query = url
        .query_pairs()
        .find(|(name, _)| name == "story")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("checks.jsonl"))?;
    writeln!(log, "{}", json!({"mode":mode,"url":url,"profile":profile}))?;
    if query.starts_with("webview cancel ") {
        let port: u16 = query.split_whitespace().last().unwrap().parse()?;
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", port))?;
        socket.write_all(&[1])?;
        socket.read_exact(&mut [0])?;
        return Err("canceled helper survived".into());
    }

    let error = match query.as_str() {
        "webview manual" => Some("browser_check_required"),
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
    let root = std::env::temp_dir().join(format!("qbutt-igruha-check-{}", std::process::id()));
    let app = root.join("app");
    std::fs::create_dir_all(&app)?;
    // This hardlink is an immutable test executable, never writable staging data.
    std::fs::hard_link(std::env::current_exe()?, app.join("qbutt-webview.exe"))?;
    let new_source = || {
        let source = igruha::fixture(
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
        ("webview", 2),
        ("webview persistent", 2),
        ("webview manual", 1),
        ("webview fault", 1),
        ("webview foreign", 1),
    ] {
        let source = new_source();
        let before = fixture.hits.load(Ordering::SeqCst);
        let result = source
            .search(query, Category::Games, None, BrowserChoice::Auto)
            .await;
        assert_eq!(
            fixture.hits.load(Ordering::SeqCst) - before,
            hits,
            "{query}"
        );
        match query {
            "webview" => {
                assert_eq!(result?.len(), 1);
                let count = checks()?.len();
                assert_eq!(
                    source
                        .search(query, Category::Games, None, BrowserChoice::Auto)
                        .await?
                        .len(),
                    1
                );
                assert_eq!(checks()?.len(), count, "reuse clearance and matching UA");
            }
            "webview persistent" => assert!(matches!(result, Err(Error::SourceCheckRequired))),
            "webview manual" => {
                assert!(matches!(result, Err(Error::SourceCheckRequired)));
                source.complete_check().await?;
                assert_eq!(
                    source
                        .search(query, Category::Games, None, BrowserChoice::Auto)
                        .await?
                        .len(),
                    1
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

    for (id, hits) in [
        ("45-webview-article", 4),
        ("46-webview-landing", 5),
        ("47-webview-metadata", 6),
    ] {
        let source = new_source();
        let publication = PublicationRef {
            source: Source::Igruha,
            id: id.into(),
        };
        let before = fixture.hits.load(Ordering::SeqCst);
        let work = source
            .inspect(&publication, None, BrowserChoice::Auto)
            .await?;
        assert_eq!(fixture.hits.load(Ordering::SeqCst) - before, hits, "{id}");
        assert_eq!(work.variants.len(), 1);
        let count = checks()?.len();
        let ResolvedContent::TorrentFile {
            reference,
            metadata,
        } = source
            .resolve(&work.variants[0].reference, None, BrowserChoice::Auto)
            .await?
        else {
            panic!("expected exact checked torrent");
        };
        assert_eq!(reference, work.variants[0].reference);
        assert_eq!(metadata.bytes(), first_torrent());
        assert_eq!(checks()?.len(), count, "exact resolution reuses clearance");
        source.close();
    }

    let source = new_source();
    source
        .search("webview expiry", Category::Games, None, BrowserChoice::Auto)
        .await?;
    let count = checks()?.len();
    tokio::time::sleep(Duration::from_millis(600)).await;
    source
        .search("webview expiry", Category::Games, None, BrowserChoice::Auto)
        .await?;
    assert_eq!(checks()?.len(), count + 1, "refresh expired cookies");
    source.close();
    assert!(matches!(
        source
            .search("webview", Category::Games, None, BrowserChoice::Auto)
            .await,
        Err(Error::SourceCheck(webview::WebViewError::Stopped))
    ));

    let discovery = root.join("discovery.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let handle = broker.handle();
    let mut edge = browser_fixture::extension(&discovery, "edge", "fixture-browser", false).await?;
    let mut status = handle.subscribe();
    tokio::time::timeout(Duration::from_secs(3), async {
        while status.borrow().connections.is_empty() {
            status.changed().await.map_err(|_| BrowserError::Stopped)?;
        }
        Ok::<_, BrowserError>(())
    })
    .await??;
    let source = new_source();
    let count = checks()?.len();
    let operation = source.search(
        "webview browser",
        Category::Games,
        Some(&handle),
        BrowserChoice::Edge,
    );
    let response = async {
        let wire = browser_fixture::read(&mut edge).await?;
        assert_eq!(wire["method"], "cookies");
        browser_fixture::write(
            &mut edge,
            &json!({"id":wire["id"],"result":[{
                "name":"clearance","value":"fixture-pass","domain":"fixture.example",
                "path":"/","secure":true,"hostOnly":true
            }]}),
        )
        .await
    };
    let (result, response) = tokio::join!(operation, response);
    response?;
    assert_eq!(result?.len(), 1);
    assert_eq!(
        checks()?.len(),
        count,
        "qcatch session precedes owned check"
    );
    source.close();
    broker.shutdown().await?;
    browser_fixture::closed(&mut edge).await?;

    let source = new_source();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let query = format!("webview cancel {}", listener.local_addr()?.port());
    let mut pending = Box::pin(source.search(&query, Category::Games, None, BrowserChoice::Auto));
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
        "cancel must close the helper process"
    );
    source.close();

    let records = checks()?;
    assert_eq!(
        records
            .iter()
            .filter(|record| record["mode"] == "interactive")
            .count(),
        1
    );
    assert!(
        records
            .iter()
            .all(|record| record["profile"] == records[0]["profile"])
    );
    std::fs::remove_file(app.join("qbutt-webview.exe"))?;
    std::fs::remove_file(root.join("checks.jsonl"))?;
    std::fs::remove_dir(app)?;
    std::fs::remove_dir(root)?;
    println!(
        "PASS igruha_owned_check silent original_replay bounded_persistent exact_torrent qcatch_first expiry same_profile manual_only_on_request cancel_helper close_owner"
    );
    Ok(())
}
