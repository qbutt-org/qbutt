//! One isolated app-server process and ephemeral Codex thread per translation job.
//! The caller supplies only the page/work tools that the job is allowed to use.

use std::collections::VecDeque;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::watch;
use tokio::time::{Instant, timeout_at};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const TURN_TIMEOUT: Duration = Duration::from_secs(180);
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
const MAX_TOOL_CALLS: usize = 12;

const DISABLED_FEATURES: &[&str] = &[
    "shell_tool",
    "unified_exec",
    "shell_snapshot",
    "apps",
    "hooks",
    "multi_agent",
    "plugins",
    "memories",
    "browser_use",
    "computer_use",
    "view_image",
    "image_generation",
    "skill_search",
    "skill_mcp_dependency_install",
    "workspace_dependencies",
    "code_mode",
];

#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    #[error("Codex is not installed")]
    Unavailable,
    #[error("Sign in to the installed Codex to translate")]
    Authentication,
    #[error("The requested Codex model or reasoning effort is unavailable")]
    ModelUnavailable,
    #[error("Codex translation was cancelled")]
    Cancelled,
    #[error("Codex translation timed out")]
    Timeout,
    #[error("Codex tools could not be isolated from the reader job")]
    Isolation,
    #[error("Codex returned an invalid translation response")]
    Protocol,
    #[error("Codex could not complete the translation; the original is preserved")]
    Provider,
    #[error("The translation image is too large or has an unsupported format")]
    Image,
    #[error("{0}")]
    Tool(String),
}

pub type Result<T> = std::result::Result<T, CodexError>;
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<ToolOutput>> + Send + 'a>>;

#[derive(Clone)]
pub struct InputImage {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

impl InputImage {
    pub fn png(bytes: Vec<u8>) -> Self {
        Self {
            mime: "image/png",
            bytes,
        }
    }

    fn url(&self) -> Result<String> {
        if self.bytes.len() > MAX_IMAGE_BYTES
            || !matches!(self.mime, "image/png" | "image/jpeg" | "image/webp")
        {
            return Err(CodexError::Image);
        }
        Ok(format!(
            "data:{};base64,{}",
            self.mime,
            base64::engine::general_purpose::STANDARD.encode(&self.bytes)
        ))
    }
}

pub struct CodexRequest {
    pub instructions: String,
    pub prompt: String,
    pub image: Option<InputImage>,
    pub output_schema: Value,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub tools: Vec<FunctionTool>,
}

pub struct FunctionTool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
}

pub struct ToolOutput {
    pub text: String,
    pub image: Option<InputImage>,
}

pub trait ToolHandler {
    fn call(&mut self, call: ToolCall) -> ToolFuture<'_>;
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

pub struct CodexCompletion {
    pub value: Value,
    pub model: String,
    pub effort: String,
    /// Latest cumulative `total`, never a sum of repeated `last` notifications.
    pub usage: Option<TokenUsage>,
}

/// The only inference entry point. Cancellation drops setup immediately; an
/// active turn is first interrupted and then its owned process is reaped.
pub async fn complete(
    request: CodexRequest,
    tools: &mut impl ToolHandler,
    mut cancelled: watch::Receiver<bool>,
) -> Result<CodexCompletion> {
    if *cancelled.borrow() {
        return Err(CodexError::Cancelled);
    }
    let workspace = Scratch::new()?;
    let executable = installed_codex().ok_or(CodexError::Unavailable)?;
    let mut client = Client::start(&executable, workspace.path(), &[]).await?;
    let prepare = async {
        client.prepare(&executable, workspace.path()).await?;
        let (model, effort) = client
            .select_model(
                request.model.as_deref(),
                request.effort.as_deref(),
                request.image.is_some(),
            )
            .await?;
        let thread = client
            .start_thread(&request, workspace.path(), &model)
            .await?;
        Ok::<_, CodexError>((thread, model, effort))
    };
    let result = tokio::select! {
        ready = prepare => ready,
        changed = cancelled.changed() => {
            let _ = changed;
            Err(CodexError::Cancelled)
        }
    };
    let result = match result {
        Ok((thread, model, effort)) => {
            client
                .run_turn(thread, request, tools, &mut cancelled, model, effort)
                .await
        }
        Err(error) => Err(error),
    };
    client.stop().await;
    result
}

/// Resolve the installed account's current catalog selection before a disk
/// cache lookup. Catalog membership alone does not prove inference access.
pub async fn resolve_selection(
    model: Option<&str>,
    effort: Option<&str>,
    image: bool,
) -> Result<(String, String)> {
    let workspace = Scratch::new()?;
    let executable = installed_codex().ok_or(CodexError::Unavailable)?;
    let mut client = Client::start(&executable, workspace.path(), &[]).await?;
    let result = async {
        client.prepare(&executable, workspace.path()).await?;
        client.select_model(model, effort, image).await
    }
    .await;
    client.stop().await;
    result
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self> {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|_| CodexError::Provider)?;
        let name = format!("qbutt-translation-{}", u64::from_le_bytes(random));
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir(&path).map_err(|_| CodexError::Provider)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    queued: VecDeque<Value>,
    usage: Option<TokenUsage>,
    answer: Option<String>,
}

impl Client {
    async fn prepare(&mut self, executable: &Path, workspace: &Path) -> Result<()> {
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

    async fn start(executable: &Path, cwd: &Path, disabled_servers: &[String]) -> Result<Self> {
        let mut command = Command::new(executable);
        command.current_dir(cwd).env_clear();
        for name in [
            "APPDATA",
            "LOCALAPPDATA",
            "CODEX_HOME",
            "HOME",
            "USERPROFILE",
            "PATH",
            "PATHEXT",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "SYSTEMDRIVE",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for feature in DISABLED_FEATURES {
            command.arg("-c").arg(format!("features.{feature}=false"));
        }
        for server in disabled_servers {
            if !server
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            {
                return Err(CodexError::Isolation);
            }
            command
                .arg("-c")
                .arg(format!("mcp_servers.{server}.enabled=false"));
        }
        command
            .arg("-c")
            .arg("web_search=\"disabled\"")
            .arg("-c")
            .arg("notify=[]")
            .arg("-c")
            .arg("history.persistence=\"none\"")
            .arg("-c")
            .arg("project_doc_max_bytes=0")
            .arg("app-server")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| CodexError::Unavailable)?;
        let stdin = child.stdin.take().ok_or(CodexError::Protocol)?;
        let stdout = BufReader::new(child.stdout.take().ok_or(CodexError::Protocol)?);
        Ok(Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            queued: VecDeque::new(),
            usage: None,
            answer: None,
        })
    }

    async fn stop(&mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
    }

    async fn send(&mut self, message: &Value) -> Result<()> {
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

    async fn read(&mut self, deadline: Instant) -> Result<Value> {
        let mut line = Vec::new();
        let size = timeout_at(deadline, self.stdout.read_until(b'\n', &mut line))
            .await
            .map_err(|_| CodexError::Timeout)?
            .map_err(|_| CodexError::Provider)?;
        if size == 0 || size > MAX_MESSAGE_BYTES {
            return Err(CodexError::Protocol);
        }
        serde_json::from_slice(&line).map_err(|_| CodexError::Protocol)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
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

    async fn select_model(
        &mut self,
        requested_model: Option<&str>,
        requested_effort: Option<&str>,
        image: bool,
    ) -> Result<(String, String)> {
        let mut cursor: Option<String> = None;
        let mut models = Vec::new();
        loop {
            let result = self
                .request(
                    "model/list",
                    json!({
                        "limit": 100, "includeHidden": false, "cursor": cursor
                    }),
                )
                .await?;
            models.extend(
                result
                    .get("data")
                    .and_then(Value::as_array)
                    .ok_or(CodexError::Protocol)?
                    .iter()
                    .cloned(),
            );
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() || models.len() >= 1000 {
                break;
            }
        }
        let model = models
            .iter()
            .find(|entry| {
                let selected = requested_model.is_some_and(|name| {
                    entry.get("model").and_then(Value::as_str) == Some(name)
                        || entry.get("id").and_then(Value::as_str) == Some(name)
                });
                let default =
                    requested_model.is_none() && entry.get("isDefault") == Some(&Value::Bool(true));
                (selected || default)
                    && (!image
                        || entry
                            .get("inputModalities")
                            .and_then(Value::as_array)
                            .is_some_and(|items| items.contains(&json!("image"))))
            })
            .or_else(|| {
                if requested_model.is_none() {
                    models.iter().find(|entry| {
                        !image
                            || entry
                                .get("inputModalities")
                                .and_then(Value::as_array)
                                .is_some_and(|items| items.contains(&json!("image")))
                    })
                } else {
                    None
                }
            })
            .ok_or(CodexError::ModelUnavailable)?;
        let name = model
            .get("model")
            .and_then(Value::as_str)
            .ok_or(CodexError::Protocol)?
            .to_owned();
        let supported = model
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .ok_or(CodexError::Protocol)?;
        let preferred = requested_effort.unwrap_or("low");
        let effort = if supported
            .iter()
            .any(|entry| entry.get("reasoningEffort") == Some(&json!(preferred)))
        {
            preferred.to_owned()
        } else if requested_effort.is_none() {
            model
                .get("defaultReasoningEffort")
                .and_then(Value::as_str)
                .ok_or(CodexError::Protocol)?
                .to_owned()
        } else {
            return Err(CodexError::ModelUnavailable);
        };
        Ok((name, effort))
    }

    async fn start_thread(
        &mut self,
        request: &CodexRequest,
        cwd: &Path,
        model: &str,
    ) -> Result<String> {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function", "name": tool.name, "description": tool.description,
                    "inputSchema": tool.input_schema
                })
            })
            .collect();
        let result = self
            .request(
                "thread/start",
                json!({
                    "model": model, "modelProvider": "openai", "cwd": cwd,
                    "approvalPolicy": "never", "sandbox": "read-only", "ephemeral": true,
                    "baseInstructions": request.instructions,
                    "dynamicTools": tools, "selectedCapabilityRoots": [], "environments": [],
                    "runtimeWorkspaceRoots": []
                }),
            )
            .await?;
        let thread = result.get("thread").ok_or(CodexError::Protocol)?;
        if thread.get("ephemeral") != Some(&Value::Bool(true))
            || !thread.get("path").is_some_and(Value::is_null)
        {
            return Err(CodexError::Isolation);
        }
        Ok(thread
            .get("id")
            .and_then(Value::as_str)
            .ok_or(CodexError::Protocol)?
            .to_owned())
    }

    async fn run_turn(
        &mut self,
        thread: String,
        request: CodexRequest,
        tools: &mut impl ToolHandler,
        cancelled: &mut watch::Receiver<bool>,
        model: String,
        effort: String,
    ) -> Result<CodexCompletion> {
        let mut input = vec![json!({"type": "text", "text": request.prompt, "text_elements": []})];
        if let Some(image) = request.image {
            input.push(json!({"type": "image", "url": image.url()?}));
        }
        let start = self.request("turn/start", json!({
            "threadId": thread, "input": input, "effort": effort,
            "approvalPolicy": "never", "sandboxPolicy": {"type": "readOnly", "networkAccess": false},
            "outputSchema": request.output_schema
        }));
        let result = tokio::select! {
            ready = start => ready?,
            changed = cancelled.changed() => {
                let _ = changed;
                return Err(CodexError::Cancelled);
            }
        };
        let turn = result.get("turn").ok_or(CodexError::Protocol)?;
        let turn_id = turn
            .get("id")
            .and_then(Value::as_str)
            .ok_or(CodexError::Protocol)?
            .to_owned();
        if turn.get("status") == Some(&json!("completed")) {
            return self.finish(turn, model, effort);
        }
        let deadline = Instant::now() + TURN_TIMEOUT;
        let mut tool_calls = 0;
        loop {
            let message = if let Some(queued) = self.queued.pop_front() {
                queued
            } else {
                tokio::select! {
                    biased;
                    changed = cancelled.changed() => {
                        if changed.is_ok() && *cancelled.borrow() {
                            self.interrupt(&thread, &turn_id).await;
                            return Err(CodexError::Cancelled);
                        }
                        continue;
                    }
                    response = self.read(deadline) => response?,
                }
            };
            match message.get("method").and_then(Value::as_str) {
                Some("item/tool/call") => {
                    tool_calls += 1;
                    if tool_calls > MAX_TOOL_CALLS {
                        self.interrupt(&thread, &turn_id).await;
                        return Err(CodexError::Tool("Translation tool limit exceeded".into()));
                    }
                    self.handle_tool(
                        message,
                        &thread,
                        &turn_id,
                        &request.tools,
                        tools,
                        cancelled,
                        deadline,
                    )
                    .await?;
                }
                Some("thread/tokenUsage/updated") => {
                    let params = message.get("params").ok_or(CodexError::Protocol)?;
                    if params.get("threadId") == Some(&json!(thread))
                        && params.get("turnId") == Some(&json!(turn_id))
                    {
                        self.usage = serde_json::from_value(
                            params
                                .pointer("/tokenUsage/total")
                                .cloned()
                                .ok_or(CodexError::Protocol)?,
                        )
                        .ok();
                    }
                }
                Some("item/completed") => {
                    let item = message
                        .pointer("/params/item")
                        .ok_or(CodexError::Protocol)?;
                    if item.get("type") == Some(&json!("agentMessage")) {
                        self.answer = item.get("text").and_then(Value::as_str).map(str::to_owned);
                    } else if !matches!(
                        item.get("type").and_then(Value::as_str),
                        Some(
                            "reasoning"
                                | "userMessage"
                                | "contextCompaction"
                                | "dynamicToolCall"
                                | "functionCallOutput"
                        )
                    ) {
                        return Err(CodexError::Isolation);
                    }
                }
                Some("item/started") => {
                    let kind = message.pointer("/params/item/type").and_then(Value::as_str);
                    if !matches!(
                        kind,
                        Some(
                            "agentMessage"
                                | "reasoning"
                                | "userMessage"
                                | "contextCompaction"
                                | "dynamicToolCall"
                                | "functionCallOutput"
                        )
                    ) {
                        return Err(CodexError::Isolation);
                    }
                }
                Some("turn/completed") => {
                    if message.pointer("/params/threadId") == Some(&json!(thread))
                        && message.pointer("/params/turn/id") == Some(&json!(turn_id))
                    {
                        return self.finish(
                            message
                                .pointer("/params/turn")
                                .ok_or(CodexError::Protocol)?,
                            model,
                            effort,
                        );
                    }
                }
                Some("error")
                    if message.pointer("/params/willRetry") == Some(&Value::Bool(false)) =>
                {
                    return Err(CodexError::Provider);
                }
                _ => {
                    if message.get("id").is_some() && message.get("method").is_some() {
                        return Err(CodexError::Isolation);
                    }
                }
            }
        }
    }

    async fn handle_tool(
        &mut self,
        message: Value,
        thread: &str,
        turn: &str,
        declared: &[FunctionTool],
        tools: &mut impl ToolHandler,
        cancelled: &mut watch::Receiver<bool>,
        deadline: Instant,
    ) -> Result<()> {
        let id = message.get("id").cloned().ok_or(CodexError::Protocol)?;
        let params = message.get("params").ok_or(CodexError::Protocol)?;
        let name = params
            .get("tool")
            .and_then(Value::as_str)
            .ok_or(CodexError::Protocol)?;
        if params.get("threadId") != Some(&json!(thread))
            || params.get("turnId") != Some(&json!(turn))
            || params.get("namespace") != Some(&Value::Null)
            || !declared.iter().any(|tool| tool.name == name)
        {
            return Err(CodexError::Isolation);
        }
        let call = ToolCall {
            name: name.to_owned(),
            arguments: params
                .get("arguments")
                .cloned()
                .ok_or(CodexError::Protocol)?,
        };
        let output = tokio::select! {
            biased;
            changed = cancelled.changed() => {
                if changed.is_ok() && *cancelled.borrow() {
                    self.interrupt(thread, turn).await;
                    return Err(CodexError::Cancelled);
                }
                return Err(CodexError::Cancelled);
            }
            result = timeout_at(deadline, tools.call(call)) => result.map_err(|_| CodexError::Timeout)?,
        };
        let reply = match output {
            Ok(output) => {
                let mut content = vec![json!({"type": "inputText", "text": output.text})];
                if let Some(image) = output.image {
                    content.push(json!({"type": "inputImage", "imageUrl": image.url()?}));
                }
                json!({"id": id, "result": {"success": true, "contentItems": content}})
            }
            Err(error) => json!({"id": id, "result": {"success": false,
                "contentItems": [{"type": "inputText", "text": error.to_string()}]}}),
        };
        self.send(&reply).await
    }

    async fn interrupt(&mut self, thread: &str, turn: &str) {
        let id = self.next_id;
        self.next_id += 1;
        let _ = self
            .send(&json!({"id": id, "method": "turn/interrupt",
            "params": {"threadId": thread, "turnId": turn}}))
            .await;
        let deadline = Instant::now() + INTERRUPT_TIMEOUT;
        while let Ok(message) = self.read(deadline).await {
            if message.get("method") == Some(&json!("turn/completed"))
                && message.pointer("/params/turn/id") == Some(&json!(turn))
            {
                break;
            }
        }
    }

    fn finish(&self, turn: &Value, model: String, effort: String) -> Result<CodexCompletion> {
        if turn.get("status") != Some(&json!("completed")) {
            return Err(CodexError::Provider);
        }
        let answer = turn
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .rev()
                    .find(|item| item.get("type") == Some(&json!("agentMessage")))
            })
            .and_then(|item| item.get("text").and_then(Value::as_str))
            .or(self.answer.as_deref())
            .ok_or(CodexError::Protocol)?;
        let value = serde_json::from_str(answer).map_err(|_| CodexError::Protocol)?;
        Ok(CodexCompletion {
            value,
            model,
            effort,
            usage: self.usage.clone(),
        })
    }
}

fn installed_codex() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        if let Some(directory) = program_files {
            let native = directory
                .join("nodejs/node_modules/@openai/codex/node_modules")
                .join("@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe");
            if native.is_file() {
                return Some(native);
            }
        }
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)?
            .join("OpenAI/Codex/bin");
        let mut candidates: Vec<_> = std::fs::read_dir(local)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join("codex.exe"))
            .filter(|path| path.is_file())
            .collect();
        candidates.sort_by_key(|path| path.metadata().and_then(|info| info.modified()).ok());
        candidates.pop()
    }
    #[cfg(not(windows))]
    {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|path| path.join("codex"))
            .find(|path| path.is_file())
    }
}
