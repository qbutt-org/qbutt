use std::fs;
use std::path::Path;
use std::time::Duration;

use qbutt_content::browser::{
    BrowserChoice, BrowserCommand, BrowserError, BrowserHandle, BrowserReply, BrowserStatus,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

pub(super) const MAX_FRAME: usize = 768 * 1024;
pub(super) type FixtureResult = Result<(), Box<dyn std::error::Error>>;

pub(super) fn cookies() -> BrowserCommand {
    BrowserCommand::Cookies {
        url: "https://fixture.example/".parse().unwrap(),
    }
}

pub(super) fn request(
    handle: &BrowserHandle,
    choice: BrowserChoice,
    command: BrowserCommand,
) -> JoinHandle<Result<BrowserReply, BrowserError>> {
    let handle = handle.clone();
    tokio::spawn(async move { handle.request(choice, command).await })
}

pub(super) async fn exchange(
    handle: &BrowserHandle,
    edge: &mut NamedPipeClient,
    command: Value,
    result: Value,
    expected: Option<BrowserError>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let command: BrowserCommand = serde_json::from_value(command)?;
    let pending = request(handle, BrowserChoice::Edge, command);
    let wire = read(edge).await?;
    write_fragmented(edge, &json!({"id":wire["id"],"result":result})).await?;
    match (pending.await?, expected) {
        (Ok(reply), None) => Ok(serde_json::to_value(reply)?),
        (Err(error), Some(expected)) if error == expected => Ok(Value::Null),
        (Err(error), expected) => {
            panic!("Unexpected bounded error {error:?}; expected {expected:?}")
        }
        (Ok(_), Some(expected)) => panic!("Expected {expected:?}"),
    }
}

pub(super) fn assert_error(result: Result<BrowserReply, BrowserError>, expected: BrowserError) {
    match result {
        Err(error) => assert_eq!(error, expected),
        Ok(_) => panic!("Expected {expected:?}"),
    }
}

pub(super) fn hello(browser: &str, user_agent: &str, vk_search: bool) -> Value {
    json!({
        "type": "hello",
        "version": 1,
        "browser": browser,
        "userAgent": user_agent,
        "capabilities": if vk_search { vec!["vk-search"] } else { vec![] },
    })
}

pub(super) async fn connect(
    discovery: &Path,
) -> Result<NamedPipeClient, Box<dyn std::error::Error>> {
    let value: Value = serde_json::from_slice(&fs::read(discovery)?)?;
    let pipe = value["pipe"].as_str().ok_or("Missing discovery pipe")?;
    for _ in 0..200 {
        match ClientOptions::new().open(pipe) {
            Ok(client) => return Ok(client),
            Err(error) if error.raw_os_error() == Some(231) => {
                sleep(Duration::from_millis(10)).await
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err("Pipe connection timeout".into())
}

pub(super) async fn extension(
    discovery: &Path,
    browser: &str,
    agent: &str,
    vk_search: bool,
) -> Result<NamedPipeClient, Box<dyn std::error::Error>> {
    let mut client = connect(discovery).await?;
    write(&mut client, &hello(browser, agent, vk_search)).await?;
    Ok(client)
}

pub(super) async fn wait_status(
    handle: &BrowserHandle,
    predicate: impl Fn(&BrowserStatus) -> bool,
) -> FixtureResult {
    let mut status = handle.subscribe();
    timeout(Duration::from_secs(3), async {
        while !predicate(&status.borrow()) {
            status.changed().await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await??;
    Ok(())
}

pub(super) async fn closed(client: &mut NamedPipeClient) -> FixtureResult {
    let mut byte = [0u8; 1];
    let result = timeout(Duration::from_secs(7), client.read(&mut byte)).await?;
    assert!(
        matches!(result, Ok(0))
            || matches!(result, Err(ref error) if matches!(error.raw_os_error(), Some(109 | 232 | 233)))
    );
    Ok(())
}

pub(super) async fn read(
    client: &mut NamedPipeClient,
) -> Result<Value, Box<dyn std::error::Error>> {
    timeout(Duration::from_secs(3), async {
        let length = client.read_u32_le().await? as usize;
        if length == 0 || length > MAX_FRAME {
            return Err("Invalid fixture frame".into());
        }
        let mut body = vec![0u8; length];
        client.read_exact(&mut body).await?;
        Ok(serde_json::from_slice(&body)?)
    })
    .await?
}

pub(super) async fn write(client: &mut NamedPipeClient, value: &Value) -> FixtureResult {
    let bytes = serde_json::to_vec(value)?;
    client
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await?;
    client.write_all(&bytes).await?;
    Ok(())
}

pub(super) async fn write_fragmented(client: &mut NamedPipeClient, value: &Value) -> FixtureResult {
    let body = serde_json::to_vec(value)?;
    let bytes: Vec<_> = (body.len() as u32)
        .to_le_bytes()
        .into_iter()
        .chain(body)
        .collect();
    for piece in bytes.chunks(31) {
        client.write_all(piece).await?;
        tokio::task::yield_now().await;
    }
    Ok(())
}
