mod commands;
mod connected_only;
mod external_video;
mod fixture;
mod lifecycle;
mod native_host;
mod security;

use std::fs::{self, File};
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qbutt_content::browser::{
    BrowserBroker, BrowserChoice, BrowserCommand, BrowserError, BrowserId,
};
use serde_json::json;
use tokio::time::timeout;

use commands::command_replies;
use connected_only::connected_only;
use fixture::{
    FixtureResult, assert_error, closed, connect, cookies, exchange, extension, hello, read,
    request, wait_status, write_fragmented,
};
use lifecycle::{
    active_faults, cancellation, capacity, connection_grace, connection_identity, framing_faults,
    stale_discovery,
};
use native_host::native_host_relay;
use security::private_acl;

pub async fn run() -> FixtureResult {
    let directory = std::env::temp_dir().join(format!(
        "qbutt-native-browser-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&directory)?;
    let discovery = directory.join("browser-bridge.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let handle = broker.handle();
    assert!(handle.status().connections.is_empty());
    assert!(matches!(
        BrowserBroker::start(discovery.clone()).await,
        Err(BrowserError::OwnerExists)
    ));
    private_acl(File::open(&discovery)?.as_raw_handle());
    for address in [
        "http://127.0.0.1/",
        "https://10.0.0.1/",
        "https://localhost/",
        "https://host.local/",
        "https://[::1]/",
        "https://user:pass@example.com/",
        "file:///C:/Windows/",
    ] {
        assert_error(
            handle
                .request(
                    BrowserChoice::Auto,
                    serde_json::from_value(json!({"method":"cookies","url":address}))?,
                )
                .await,
            BrowserError::InvalidRequest,
        );
    }

    framing_faults(&discovery).await?;
    let mut edge = connect(&discovery).await?;
    private_acl(edge.as_raw_handle());
    write_fragmented(&mut edge, &hello("edge", "fixture edge", true)).await?;
    wait_status(&handle, |status| {
        status
            .connections
            .iter()
            .any(|connection| connection.user_agent == "fixture edge")
    })
    .await?;
    command_replies(&handle, &mut edge).await?;
    external_video::dispatch(&handle, &mut edge).await?;
    let mut chrome = extension(&discovery, "chrome", "fixture chrome", false).await?;
    wait_status(&handle, |status| status.connections.len() == 2).await?;
    assert_error(
        handle.request(BrowserChoice::Auto, cookies()).await,
        BrowserError::ChoiceRequired,
    );
    assert_error(
        handle
            .request(
                BrowserChoice::Chrome,
                BrowserCommand::vk_search("fixture".into())?,
            )
            .await,
        BrowserError::UpdateRequired,
    );
    connection_identity(&handle, &mut edge, &mut chrome).await?;
    cancellation(&handle, &mut edge).await?;
    capacity(&handle, &mut edge).await?;

    let pending = request(&handle, BrowserChoice::Edge, cookies());
    let _wire = read(&mut edge).await?;
    let mut replacement = extension(&discovery, "edge", "replacement edge", true).await?;
    wait_status(&handle, |status| {
        status
            .connections
            .iter()
            .any(|connection| connection.user_agent == "replacement edge")
    })
    .await?;
    assert_error(pending.await?, BrowserError::Disconnected);
    closed(&mut edge).await?;
    exchange(
        &handle,
        &mut replacement,
        json!({"method":"cookies","url":"https://fixture.example/"}),
        json!([]),
        None,
    )
    .await?;
    let pending = request(&handle, BrowserChoice::Edge, cookies());
    let _wire = read(&mut replacement).await?;
    drop(replacement);
    assert_error(pending.await?, BrowserError::Disconnected);
    wait_status(&handle, |status| {
        status.connections.len() == 1 && status.connections[0].browser == BrowserId::Chrome
    })
    .await?;
    let pending = request(&handle, BrowserChoice::Chrome, cookies());
    let _wire = read(&mut chrome).await?;
    timeout(Duration::from_secs(3), broker.shutdown()).await??;
    assert_error(pending.await?, BrowserError::Stopped);
    assert!(handle.status().stopped);
    assert!(!discovery.exists());
    closed(&mut chrome).await?;
    assert_error(
        handle.request(BrowserChoice::Chrome, cookies()).await,
        BrowserError::Stopped,
    );

    connection_grace(&discovery).await?;
    connected_only(&discovery).await?;
    active_faults(&discovery).await?;
    stale_discovery(&discovery).await?;
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let dropped = broker.handle();
    drop(broker);
    wait_status(&dropped, |status| status.stopped).await?;
    assert!(!discovery.exists());
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let native_host = match arguments.as_slice() {
        [executable, extension_id] => {
            native_host_relay(
                &discovery,
                PathBuf::from(executable),
                extension_id.to_string_lossy().into_owned(),
            )
            .await?;
            "ok"
        }
        [] => "not_run",
        _ => return Err("Expected qcatch.exe and extension ID arguments".into()),
    };
    assert!(!discovery.exists());
    fs::remove_dir(&directory)?;
    println!(
        "PASS browser_bridge=ok real_windows_pipe=ok private_acl=ok framing=ok exact_selection=ok connection_identity=ok cancellation=ok batch_cancel=ok connection_grace=ok connected_only=ok static_errors=ok bounded_pending=ok shutdown=ok native_host={native_host}"
    );
    Ok(())
}
