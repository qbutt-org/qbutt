use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::browser::BrowserHandle;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;
#[cfg(windows)]
pub(super) type BrowserClient = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
pub(super) type BrowserClient = tokio::net::UnixStream;
#[cfg(windows)]
use tokio::time::sleep;
use tokio::time::timeout;

use super::FixtureResult;

pub(super) fn hello(browser: &str, user_agent: &str) -> Value {
    json!({"type":"hello","version":1,"browser":browser,"userAgent":user_agent})
}

pub(super) async fn connect(
    discovery: &Path,
    browser: &str,
    agent: &str,
) -> FixtureResult<BrowserClient> {
    let mut client = open(discovery).await?;
    write(&mut client, &hello(browser, agent)).await?;
    Ok(client)
}

pub(super) async fn open(discovery: &Path) -> FixtureResult<BrowserClient> {
    let value: Value = serde_json::from_slice(&fs::read(discovery)?)?;
    let pipe = value["pipe"].as_str().ok_or("Missing discovery pipe")?;
    #[cfg(unix)]
    return Ok(BrowserClient::connect(pipe).await?);
    #[cfg(windows)]
    timeout(Duration::from_secs(3), async {
        loop {
            match ClientOptions::new().open(pipe) {
                Ok(client) => return Ok(client),
                Err(error) if error.raw_os_error() == Some(231) => {
                    sleep(Duration::from_millis(10)).await
                }
                Err(error) => return Err(error.into()),
            }
        }
    })
    .await?
}

pub(super) async fn wait_agent(handle: &BrowserHandle, agent: &str) -> FixtureResult {
    let mut status = handle.subscribe();
    timeout(Duration::from_secs(3), async {
        while !status
            .borrow()
            .connections
            .iter()
            .any(|connection| connection.user_agent == agent)
        {
            status.changed().await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await?
}

pub(super) async fn read(client: &mut BrowserClient) -> FixtureResult<Value> {
    timeout(Duration::from_secs(3), async {
        let size = client.read_u32_le().await? as usize;
        if size == 0 || size > 768 * 1024 {
            return Err("Invalid fixture frame size".into());
        }
        let mut bytes = vec![0; size];
        client.read_exact(&mut bytes).await?;
        Ok(serde_json::from_slice(&bytes)?)
    })
    .await?
}

pub(super) async fn write(client: &mut BrowserClient, value: &Value) -> FixtureResult {
    let bytes = serde_json::to_vec(value)?;
    client.write_u32_le(bytes.len() as u32).await?;
    client.write_all(&bytes).await?;
    client.flush().await?;
    Ok(())
}
