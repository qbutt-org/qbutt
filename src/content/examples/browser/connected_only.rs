use std::path::Path;
use std::time::{Duration, Instant};

use qbutt_content::browser::{BrowserBroker, BrowserChoice, BrowserError, BrowserReply};
use serde_json::json;
use tokio::time::timeout;

use super::fixture::{FixtureResult, assert_error, cookies, extension, read, wait_status, write};

pub(super) async fn connected_only(discovery: &Path) -> FixtureResult {
    let broker = BrowserBroker::start(discovery.to_path_buf()).await?;
    let handle = broker.handle();
    assert_error(
        timeout(
            Duration::from_secs(1),
            handle.request_connected(BrowserChoice::Auto, cookies()),
        )
        .await?,
        BrowserError::BrowserRequired,
    );
    let chrome = extension(discovery, "chrome", "optional chrome", true).await?;
    wait_status(&handle, |status| status.connections.len() == 1).await?;
    assert_error(
        timeout(
            Duration::from_secs(1),
            handle.request_connected(BrowserChoice::Edge, cookies()),
        )
        .await?,
        BrowserError::BrowserRequired,
    );
    let edge = extension(discovery, "edge", "optional edge", true).await?;
    wait_status(&handle, |status| status.connections.len() == 2).await?;
    assert_error(
        handle
            .request_connected(BrowserChoice::Auto, cookies())
            .await,
        BrowserError::ChoiceRequired,
    );
    drop(chrome);
    drop(edge);
    wait_status(&handle, |status| status.connections.is_empty()).await?;

    for _ in 0..8 {
        let edge = extension(discovery, "edge", "optional stale status", true).await?;
        wait_status(&handle, |status| status.connections.len() == 1).await?;
        // The caller observed a connected browser, but it disappears before
        // dispatch. Optional cookies must fail promptly without reconnecting.
        assert_eq!(handle.status().connections.len(), 1);
        drop(edge);
        let result = timeout(
            Duration::from_secs(1),
            handle.request_connected(BrowserChoice::Edge, cookies()),
        )
        .await?;
        assert!(matches!(
            result,
            Err(BrowserError::BrowserRequired | BrowserError::Disconnected)
        ));
        wait_status(&handle, |status| status.connections.is_empty()).await?;
    }

    let mut edge = extension(discovery, "edge", "optional active work", true).await?;
    wait_status(&handle, |status| status.connections.len() == 1).await?;
    let connected = handle.clone();
    let pending = tokio::spawn(async move {
        connected
            .request_connected(BrowserChoice::Edge, cookies())
            .await
    });
    let wire = read(&mut edge).await?;
    write(&mut edge, &json!({"id":wire["id"],"result":[]})).await?;
    assert!(matches!(pending.await?, Ok(BrowserReply::Cookies(_))));

    let connected = handle.clone();
    let pending = tokio::spawn(async move {
        connected
            .request_connected(BrowserChoice::Edge, cookies())
            .await
    });
    let wire = read(&mut edge).await?;
    pending.abort();
    assert!(matches!(pending.await, Err(error) if error.is_cancelled()));
    assert_eq!(
        read(&mut edge).await?,
        json!({"id":wire["id"],"method":"cancel"})
    );

    let began = Instant::now();
    let connected = handle.clone();
    let pending = tokio::spawn(async move {
        connected
            .request_connected(BrowserChoice::Edge, cookies())
            .await
    });
    let wire = read(&mut edge).await?;
    assert_error(
        timeout(Duration::from_secs(23), pending).await??,
        BrowserError::Timeout,
    );
    assert!(began.elapsed() >= Duration::from_secs(20));
    assert!(began.elapsed() < Duration::from_secs(21));
    assert_eq!(
        read(&mut edge).await?,
        json!({"id":wire["id"],"method":"cancel"})
    );
    broker.shutdown().await?;
    Ok(())
}
