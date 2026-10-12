use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::time::{Instant, timeout_at};

use super::process::DISABLED_FEATURES;
use super::{Client, CodexError, Result};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

impl Client {
    pub(super) async fn prepare(&mut self, executable: &Path, workspace: &Path) -> Result<()> {
        self.initialize().await?;
        let inherited = self.check_isolation().await?;
        if !inherited.is_empty() {
            self.stop().await;
            *self = Self::start(executable, workspace, &inherited).await?;
            self.initialize().await?;
            if !self.check_isolation().await?.is_empty() {
                return Err(CodexError::Isolation);
            }
        }
        let account = self
            .request("account/read", json!({"refreshToken": false}))
            .await?;
        if account.get("account").is_none_or(Value::is_null) {
            return Err(CodexError::Authentication);
        }
        Ok(())
    }

    pub(super) async fn send(&mut self, message: &Value) -> Result<()> {
        let line = serde_json::to_vec(message).map_err(|_| CodexError::Protocol)?;
        if line.len() > MAX_MESSAGE_BYTES {
            return Err(CodexError::Protocol);
        }
        self.stdin
            .write_all(&line)
            .await
            .map_err(|_| CodexError::Provider)?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(|_| CodexError::Provider)?;
        self.stdin.flush().await.map_err(|_| CodexError::Provider)
    }

    pub(super) async fn read(&mut self, deadline: Instant) -> Result<Value> {
        let mut line = Vec::new();
        loop {
            let available = timeout_at(deadline, self.stdout.fill_buf())
                .await
                .map_err(|_| CodexError::Timeout)?
                .map_err(|_| CodexError::Provider)?;
            if available.is_empty() {
                return Err(CodexError::Protocol);
            }

            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(available.len(), |position| position + 1);
            if count > MAX_MESSAGE_BYTES - line.len() {
                return Err(CodexError::Protocol);
            }
            line.extend_from_slice(&available[..count]);
            self.stdout.consume(count);
            if newline.is_some() {
                return serde_json::from_slice(&line).map_err(|_| CodexError::Protocol);
            }
        }
    }

    pub(super) async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"id": id, "method": method, "params": params}))
            .await?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            let message = self.read(deadline).await?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return message.get("result").cloned().ok_or(CodexError::Provider);
            }
            if message.get("id").is_some() && message.get("method").is_some() {
                return Err(CodexError::Isolation);
            }
            if self.queued.len() >= 128 {
                return Err(CodexError::Protocol);
            }
            self.queued.push_back(message);
        }
    }

    async fn initialize(&mut self) -> Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {"name": "qbutt_reader", "title": "qbutt reader", "version": "1.0.0"},
                "capabilities": {"experimentalApi": true}
            }),
        )
        .await?;
        self.send(&json!({"method": "initialized"})).await
    }

    async fn check_isolation(&mut self) -> Result<Vec<String>> {
        let result = self
            .request("config/read", json!({"includeLayers": false}))
            .await?;
        let config = result.get("config").ok_or(CodexError::Protocol)?;
        let features = config.get("features").ok_or(CodexError::Isolation)?;
        if DISABLED_FEATURES
            .iter()
            .any(|name| features.get(*name) != Some(&Value::Bool(false)))
            || config.get("web_search") != Some(&Value::String("disabled".into()))
            || config.get("notify") != Some(&json!([]))
        {
            return Err(CodexError::Isolation);
        }
        let servers = config.get("mcp_servers").and_then(Value::as_object);
        Ok(servers
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter(|(_, setting)| setting.get("enabled") != Some(&Value::Bool(false)))
            .map(|(name, _)| name.clone())
            .collect())
    }
}
