mod http_acceptance;
mod pipe;
mod tls;

use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::browser::{BrowserBroker, BrowserChoice, BrowserError};
use crate::http::SourceSession;
use serde_json::json;
use tokio::time::timeout;

use pipe::{connect, read, wait_agent, write};

type FixtureResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) async fn run() -> FixtureResult {
    let directory = std::env::temp_dir().join(format!(
        "qbutt-source-session-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&directory)?;
    let discovery = directory.join("browser.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let handle = broker.handle();
    let fixture = tls::Fixture::start().await?;
    let url = fixture.origin.join("/forum/start")?;
    assert!(matches!(
        timeout(
            Duration::from_millis(500),
            handle.session_connected(BrowserChoice::Auto, url.clone())
        )
        .await?,
        Err(BrowserError::BrowserRequired)
    ));
    assert!(matches!(
        handle
            .session_connected(BrowserChoice::Auto, "http://fixture.example/".parse()?)
            .await,
        Err(BrowserError::InvalidRequest)
    ));
    let mut edge = connect(&discovery, "edge", "fixture edge original").await?;
    wait_agent(&handle, "fixture edge original").await?;
    let mut chrome = connect(&discovery, "chrome", "fixture chrome").await?;
    wait_agent(&handle, "fixture chrome").await?;
    assert!(matches!(
        handle
            .session_connected(BrowserChoice::Auto, url.clone())
            .await,
        Err(BrowserError::ChoiceRequired)
    ));

    let selected = handle.clone();
    let requested = url.clone();
    let mut pending = tokio::spawn(async move {
        selected
            .session_connected(BrowserChoice::Edge, requested)
            .await
    });
    let wire = read(&mut edge).await?;
    assert_eq!(wire["method"], "cookies");
    assert_eq!(wire["url"], url.as_str());
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
    let cookies = json!([
        {"name":"sid","value":"old","domain":"fixture.example","path":"/forum","secure":true,"hostOnly":true,"expirationDate":now+3600.0},
        {"name":"wide","value":"root","domain":".fixture.example","path":"/","secure":true,"hostOnly":false},
        {"name":"gone","value":"remove","domain":"fixture.example","path":"/","secure":false,"hostOnly":true},
        {"name":"expired","value":"never","domain":"fixture.example","path":"/","secure":true,"hostOnly":true,"expirationDate":now-1.0}
    ]);
    // Another connected peer cannot supply either cookies or metadata for Edge.
    write(&mut chrome, &json!({"id":wire["id"],"result":[]})).await?;
    assert!(
        timeout(Duration::from_millis(50), &mut pending)
            .await
            .is_err()
    );
    // Concurrent metadata changes on another browser retain the selected UA.
    let replacement_chrome = connect(&discovery, "chrome", "fixture chrome updated").await?;
    wait_agent(&handle, "fixture chrome updated").await?;
    write(&mut edge, &json!({"id":wire["id"],"result":cookies})).await?;
    let imported = pending.await??;
    assert_eq!(imported.connection.user_agent, "fixture edge original");
    assert_eq!(imported.cookies.len(), 4);
    let mut source = SourceSession::from_browser(fixture.origin.clone(), imported)?;
    println!("PASS atomic selected browser cookies and user agent");

    let selected = handle.clone();
    let requested = url.clone();
    let canceled = tokio::spawn(async move {
        selected
            .session_connected(BrowserChoice::Edge, requested)
            .await
    });
    let wire = read(&mut edge).await?;
    canceled.abort();
    assert!(matches!(canceled.await, Err(error) if error.is_cancelled()));
    assert_eq!(
        read(&mut edge).await?,
        json!({"id":wire["id"],"method":"cancel"})
    );

    let selected = handle.clone();
    let requested = url.clone();
    let disconnected = tokio::spawn(async move {
        selected
            .session_connected(BrowserChoice::Edge, requested)
            .await
    });
    read(&mut edge).await?;
    let replacement = connect(&discovery, "edge", "fixture edge replacement").await?;
    wait_agent(&handle, "fixture edge replacement").await?;
    assert!(matches!(
        disconnected.await?,
        Err(BrowserError::Disconnected)
    ));
    println!("PASS browser cancellation and replacement isolation");

    http_acceptance::run(&fixture, &mut source).await?;
    drop(replacement);
    drop(replacement_chrome);
    drop(chrome);
    drop(edge);
    broker.shutdown().await?;
    assert!(!discovery.exists());
    fixture.shutdown().await?;
    fs::remove_dir(directory)?;
    println!("PASS private source session integration");
    Ok(())
}
