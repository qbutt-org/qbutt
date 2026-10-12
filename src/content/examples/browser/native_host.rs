use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use qbutt_content::browser::{BrowserBroker, BrowserChoice, BrowserReply};
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

use super::fixture::{FixtureResult, MAX_FRAME, cookies, hello, request, wait_status};

struct NativeHost(Child);

impl Drop for NativeHost {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub(super) async fn native_host_relay(
    discovery: &Path,
    executable: PathBuf,
    extension_id: String,
) -> FixtureResult {
    if extension_id.len() != 32
        || !extension_id
            .bytes()
            .all(|byte| (b'a'..=b'p').contains(&byte))
    {
        return Err("Invalid fixture extension ID".into());
    }
    let broker = BrowserBroker::start(discovery.to_path_buf()).await?;
    let handle = broker.handle();
    let mut child = NativeHost(
        Command::new(executable)
            .arg(format!("chrome-extension://{extension_id}/"))
            .env("QBUTT_BROWSER_BRIDGE_DISCOVERY", discovery)
            .creation_flags(0x0800_0000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let mut input = child.0.stdin.take().ok_or("Native host stdin missing")?;
    let mut output = child.0.stdout.take().ok_or("Native host stdout missing")?;
    let hello = serde_json::to_vec(&hello("edge", "actual qcatch relay", true))?;
    input.write_all(&(hello.len() as u32).to_le_bytes())?;
    input.write_all(&hello)?;
    input.flush()?;
    wait_status(&handle, |status| {
        status
            .connections
            .iter()
            .any(|connection| connection.user_agent == "actual qcatch relay")
    })
    .await?;
    let pending = request(&handle, BrowserChoice::Edge, cookies());
    let wire = tokio::task::spawn_blocking(move || {
        let mut header = [0u8; 4];
        output.read_exact(&mut header)?;
        let count = u32::from_le_bytes(header) as usize;
        if count == 0 || count > MAX_FRAME {
            return Err(std::io::Error::other("Native host frame invalid"));
        }
        let mut body = vec![0u8; count];
        output.read_exact(&mut body)?;
        serde_json::from_slice::<Value>(&body).map_err(std::io::Error::other)
    });
    let wire = timeout(Duration::from_secs(3), wire).await???;
    assert_eq!(wire["method"], "cookies");
    let result = serde_json::to_vec(&json!({"id":wire["id"],"result":[]}))?;
    input.write_all(&(result.len() as u32).to_le_bytes())?;
    input.write_all(&result)?;
    input.flush()?;
    assert!(matches!(pending.await?, Ok(BrowserReply::Cookies(_))));
    drop(input);
    let exit = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(exit) = child.0.try_wait()? {
                return Ok::<_, std::io::Error>(exit);
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    assert!(exit.success());
    broker.shutdown().await?;
    Ok(())
}
