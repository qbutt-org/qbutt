use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;

use serde_json::json;
use url::Url;

use super::fixture::{Fault, Fixture};
use super::{Category, Error, PublicationRef, Source, SourceOperation, http, royalroad, webview};

type CheckResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Controlled executable peer of the production WebView protocol. It creates no
/// browser and never reads a real profile.
pub(super) fn helper() -> CheckResult {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let request: serde_json::Value = serde_json::from_str(&line)?;
    let profile = Path::new(request["profileDirectory"].as_str().unwrap());
    assert_eq!(
        profile.parent().unwrap().file_name().unwrap(),
        "web-session"
    );
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
        .find(|(name, _)| name == "title")
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
    let error = match query.as_str() {
        "manual-check" if mode == "silent" => Some("browser_check_required"),
        "check-timeout" => Some("webview_timeout"),
        "check-fault" => Some("webview_failed"),
        _ => None,
    };
    let response = if let Some(code) = error {
        json!({"id":request["id"],"error":{"code":code}})
    } else {
        let returned = if query == "foreign-session" {
            Url::parse("https://ads.example/")?
        } else {
            url
        };
        let expires = (query == "clearance-expiry").then(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
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
    let root = std::env::temp_dir().join(format!("qbutt-royalroad-check-{}", std::process::id()));
    let app = root.join("app");
    std::fs::create_dir_all(&app)?;
    // Only the immutable fixture executable is linked, never writable data.
    std::fs::hard_link(std::env::current_exe()?, app.join("qbutt-webview.exe"))?;
    let new_source = || {
        let source = royalroad::fixture(
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
    for query in ["clearance", "clearance-success-status", "Acon Acoustica 8"] {
        let source = new_source();
        let before = fixture.hits.load(Ordering::SeqCst);
        let expected = usize::from(query != "Acon Acoustica 8");
        assert_eq!(source.search(query).await?.len(), expected);
        assert_eq!(fixture.hits.load(Ordering::SeqCst) - before, 2);
        let count = checks()?.len();
        assert_eq!(source.search(query).await?.len(), expected);
        assert_eq!(checks()?.len(), count, "reuse clearance and matching UA");
        source.close();
    }
    for query in [
        "persistent-challenge",
        "manual-check",
        "check-timeout",
        "check-fault",
        "foreign-session",
        "plain-denial",
        "plain-denial-utf8",
    ] {
        let source = new_source();
        let before = checks()?.len();
        let result = source.search(query).await;
        match query {
            "persistent-challenge" | "manual-check" | "check-timeout" => {
                assert!(matches!(result, Err(Error::SourceCheckRequired)));
            }
            "check-fault" => assert!(matches!(
                result,
                Err(Error::SourceCheck(webview::WebViewError::Failed))
            )),
            "foreign-session" => assert!(matches!(result, Err(Error::UnsafeAddress))),
            "plain-denial" | "plain-denial-utf8" => {
                assert!(matches!(result, Err(Error::HttpStatus(403))));
            }
            _ => unreachable!(),
        }
        assert_eq!(
            checks()?.len() - before,
            usize::from(!query.starts_with("plain-denial"))
        );
        if query == "manual-check" {
            source
                .complete_check(&SourceOperation::Search {
                    source: Source::RoyalRoad,
                    query: query.into(),
                    category: Category::Books,
                    language: None,
                })
                .await?;
            let records = checks()?;
            let silent = &records[records.len() - 2];
            let manual = &records[records.len() - 1];
            assert_eq!(silent["mode"], "silent");
            assert_eq!(manual["mode"], "interactive");
            assert_eq!(silent["url"], manual["url"]);
            assert_eq!(silent["profile"], manual["profile"]);
            assert_eq!(silent["configured"], manual["configured"]);
            assert_eq!(source.search(query).await?.len(), 1);
            assert_eq!(checks()?.len(), records.len());
        }
        source.close();
    }

    let source = new_source();
    source.search("clearance-expiry").await?;
    let count = checks()?.len();
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    source.search("clearance-expiry").await?;
    assert_eq!(
        checks()?.len(),
        count + 1,
        "expired clearance must be refreshed"
    );
    source.close();

    *fixture.fault.lock().unwrap() = Fault::Challenge;
    let source = new_source();
    let publication = PublicationRef {
        source: Source::RoyalRoad,
        id: "42".into(),
    };
    let work = source.inspect(&publication).await?;
    let count = checks()?.len();
    let chapters = source.chapters(&work.variants[0].reference).await?;
    let selected = &chapters[1].reference;
    assert_eq!(source.chapter_text(selected).await?.reference, *selected);
    assert_eq!(checks()?.len(), count, "reader reuses the source clearance");
    source.close();
    assert!(matches!(
        source.inspect(&publication).await,
        Err(Error::SourceCheck(webview::WebViewError::Stopped))
    ));
    assert_eq!(checks()?.len(), count, "close retires the helper owner");
    *fixture.fault.lock().unwrap() = Fault::None;

    std::fs::remove_file(app.join("qbutt-webview.exe"))?;
    std::fs::remove_file(root.join("checks.jsonl"))?;
    std::fs::remove_dir(app)?;
    std::fs::remove_dir(root)?;
    println!(
        "PASS Royal Road owned check: silent recovery, exact replay, cookie/UA reuse, persistent challenge, explicit same-profile retry, typed timeout/fault, foreign session rejection, plain denial, reader reuse, close"
    );
    Ok(())
}
