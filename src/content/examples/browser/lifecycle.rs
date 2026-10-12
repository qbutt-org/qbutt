use std::fs;
use std::path::Path;
use std::time::Duration;

use qbutt_content::browser::{
    BrowserBroker, BrowserChoice, BrowserError, BrowserHandle, BrowserReply,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::NamedPipeClient;
use tokio::time::{sleep, timeout};

use super::fixture::{
    FixtureResult, MAX_FRAME, assert_error, closed, connect, cookies, exchange, extension, read,
    request, wait_status, write,
};

pub(super) async fn framing_faults(discovery: &Path) -> FixtureResult {
    for value in [
        json!({"type":"hello","version":2,"browser":"edge","userAgent":"fixture"}),
        json!({"type":"hello","version":1,"browser":"firefox","userAgent":"fixture"}),
        json!({"type":"hello","version":1,"browser":"edge","userAgent":"fixture\r\nsecret"}),
        json!({"type":"hello","version":1,"browser":"edge","userAgent":"fixture","capabilities":["bad_capability"]}),
    ] {
        let mut client = connect(discovery).await?;
        write(&mut client, &value).await?;
        closed(&mut client).await?;
    }
    for bytes in [
        0u32.to_le_bytes().to_vec(),
        (MAX_FRAME as u32 + 1).to_le_bytes().to_vec(),
        vec![1, 0, 0, 0, 0xff],
    ] {
        let mut client = connect(discovery).await?;
        client.write_all(&bytes).await?;
        closed(&mut client).await?;
    }
    // These three independent slow clients exercise the same five-second
    // hello deadline without extending acceptance into a serial sleep loop.
    let mut idle = connect(discovery).await?;
    let mut header = connect(discovery).await?;
    header.write_all(&[20]).await?;
    let mut body = connect(discovery).await?;
    body.write_all(&20u32.to_le_bytes()).await?;
    body.write_all(b"{\"type\"").await?;
    let (a, b, c) = tokio::join!(closed(&mut idle), closed(&mut header), closed(&mut body));
    a?;
    b?;
    c?;
    Ok(())
}

pub(super) async fn connection_identity(
    handle: &BrowserHandle,
    edge: &mut NamedPipeClient,
    chrome: &mut NamedPipeClient,
) -> FixtureResult {
    let pending = request(handle, BrowserChoice::Edge, cookies());
    let wire = read(edge).await?;
    write(chrome, &json!({"id":wire["id"],"result":[]})).await?;
    sleep(Duration::from_millis(30)).await;
    assert!(!pending.is_finished());
    write(edge, &json!({"id":wire["id"],"result":[]})).await?;
    assert!(matches!(pending.await?, Ok(BrowserReply::Cookies(_))));
    Ok(())
}

pub(super) async fn cancellation(
    handle: &BrowserHandle,
    edge: &mut NamedPipeClient,
) -> FixtureResult {
    let pending = request(handle, BrowserChoice::Edge, cookies());
    let wire = read(edge).await?;
    pending.abort();
    assert!(matches!(pending.await, Err(error) if error.is_cancelled()));
    let cancel = read(edge).await?;
    assert_eq!(cancel["method"], "cancel");
    assert_eq!(cancel["id"], wire["id"]);
    write(edge, &json!({"id":wire["id"],"result":[]})).await?;
    exchange(
        handle,
        edge,
        json!({"method":"cookies","url":"https://fixture.example/"}),
        json!([]),
        None,
    )
    .await?;
    Ok(())
}

pub(super) async fn capacity(handle: &BrowserHandle, edge: &mut NamedPipeClient) -> FixtureResult {
    let mut pending = Vec::new();
    for _ in 0..16 {
        let request = request(handle, BrowserChoice::Edge, cookies());
        let wire = read(edge).await?;
        pending.push((request, wire));
    }
    assert_error(
        handle.request(BrowserChoice::Edge, cookies()).await,
        BrowserError::Busy,
    );
    for (request, wire) in pending {
        write(edge, &json!({"id":wire["id"],"result":[]})).await?;
        assert!(matches!(request.await?, Ok(BrowserReply::Cookies(_))));
    }
    Ok(())
}

pub(super) async fn stale_discovery(path: &Path) -> FixtureResult {
    fs::write(path, json!({"pipe":r"\\.\pipe\qbutt-content-browser-00000000-0000-4000-8000-000000000000","pid":0}).to_string())?;
    let broker = BrowserBroker::start(path.to_path_buf()).await?;
    assert!(path.exists());
    broker.shutdown().await?;
    assert!(!path.exists());
    fs::write(path, json!({"pipe":r"\\.\pipe\qbutt-content-browser-00000000-0000-4000-8000-000000000000","pid":std::process::id()}).to_string())?;
    let before = fs::read(path)?;
    assert!(matches!(
        BrowserBroker::start(path.to_path_buf()).await,
        Err(BrowserError::OwnerExists)
    ));
    assert_eq!(fs::read(path)?, before);
    fs::remove_file(path)?;
    Ok(())
}

pub(super) async fn active_faults(discovery: &Path) -> FixtureResult {
    let broker = BrowserBroker::start(discovery.to_path_buf()).await?;
    let handle = broker.handle();
    for malformed in [
        json!({"id":1,"result":[],"error":{"code":"browser_error","message":"invalid"}}),
        json!({"id":1,"result":[],"extra":"invalid"}),
        json!({"id":1.5,"result":[]}),
        json!({"id":0,"result":[]}),
    ] {
        let mut edge = extension(discovery, "edge", "malformed response", true).await?;
        wait_status(&handle, |status| status.connections.len() == 1).await?;
        let pending = request(&handle, BrowserChoice::Edge, cookies());
        let wire = read(&mut edge).await?;
        let mut malformed = malformed;
        if malformed["id"] == 1 {
            malformed["id"] = wire["id"].clone();
        }
        write(&mut edge, &malformed).await?;
        assert_error(pending.await?, BrowserError::Disconnected);
        closed(&mut edge).await?;
        wait_status(&handle, |status| status.connections.is_empty()).await?;
    }
    let mut edge = extension(discovery, "edge", "interleaved partial response", true).await?;
    wait_status(&handle, |status| status.connections.len() == 1).await?;
    let first = request(&handle, BrowserChoice::Edge, cookies());
    let first_wire = read(&mut edge).await?;
    let bytes = serde_json::to_vec(&json!({"id":first_wire["id"],"result":[]}))?;
    edge.write_all(&(bytes.len() as u32).to_le_bytes()[..2])
        .await?;
    let second = request(&handle, BrowserChoice::Edge, cookies());
    let second_wire = read(&mut edge).await?;
    edge.write_all(&(bytes.len() as u32).to_le_bytes()[2..])
        .await?;
    edge.write_all(&bytes).await?;
    write(&mut edge, &json!({"id":second_wire["id"],"result":[]})).await?;
    assert!(matches!(first.await?, Ok(BrowserReply::Cookies(_))));
    assert!(matches!(second.await?, Ok(BrowserReply::Cookies(_))));

    // A complete idle connection may stay open. An active request expires
    // after its fixed deadline and sends the same ID's cancellation frame.
    let began = std::time::Instant::now();
    let pending = request(&handle, BrowserChoice::Edge, cookies());
    let wire = read(&mut edge).await?;
    let cancel = timeout(Duration::from_secs(23), async {
        let length = edge.read_u32_le().await? as usize;
        let mut body = vec![0u8; length];
        edge.read_exact(&mut body).await?;
        serde_json::from_slice::<Value>(&body).map_err(std::io::Error::other)
    })
    .await??;
    assert_eq!(cancel, json!({"id":wire["id"],"method":"cancel"}));
    assert_error(pending.await?, BrowserError::Timeout);
    assert!(began.elapsed() >= Duration::from_secs(20));
    drop(edge);
    wait_status(&handle, |status| status.connections.is_empty()).await?;

    let mut blocked = extension(discovery, "edge", "blocked writer", true).await?;
    wait_status(&handle, |status| status.connections.len() == 1).await?;
    let mut pending = Vec::new();
    for _ in 0..16 {
        pending.push(request(&handle, BrowserChoice::Edge, serde_json::from_value(json!({
            "method":"fetch","url":"https://fixture.example/","init":{"method":"POST","body":"x".repeat(512 * 1024)}
        }))?));
    }
    for request in pending {
        assert_error(
            timeout(Duration::from_secs(7), request).await??,
            BrowserError::Disconnected,
        );
    }
    // Its unread command bytes may remain buffered after disconnect. We
    // verify owner state and task drain rather than interpreting them as EOF.
    wait_status(&handle, |status| status.connections.is_empty()).await?;
    blocked.shutdown().await?;
    drop(blocked);
    let mut partial = connect(discovery).await?;
    partial.write_all(&[20]).await?;
    timeout(Duration::from_secs(3), broker.shutdown()).await??;
    closed(&mut partial).await?;
    assert!(!discovery.exists());
    Ok(())
}
