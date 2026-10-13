//! Isolated requests to the installed Codex app-server.
//! Each request owns an ephemeral thread and only the tools supplied by its caller.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::watch;

mod model;
mod process;
mod rpc;
mod turn;

pub use model::CodexModel;
use process::{Scratch, installed_codex};
pub use turn::IMAGE_TOOL_CALL_LIMIT;

const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    #[error("Codex is not installed")]
    Unavailable,
    #[error("Sign in to the installed Codex")]
    Authentication,
    #[error("The requested Codex model or reasoning effort is unavailable")]
    ModelUnavailable,
    #[error("Codex request was cancelled")]
    Cancelled,
    #[error("Codex request timed out")]
    Timeout,
    #[error("Codex tools could not be isolated from the request")]
    Isolation,
    #[error("Codex returned an invalid response")]
    Protocol,
    #[error("Codex could not complete the request")]
    Provider,
    #[error("The input image is too large or has an unsupported format")]
    Image,
    #[error("{0}")]
    Tool(String),
}

pub type Result<T> = std::result::Result<T, CodexError>;
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<ToolOutput>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelModality {
    Text,
    Image,
}

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
    pub modality: ModelModality,
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

pub struct NoTools;

impl ToolHandler for NoTools {
    fn call(&mut self, _: ToolCall) -> ToolFuture<'_> {
        Box::pin(async { Err(CodexError::Isolation) })
    }
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
    if request.image.is_some() && request.modality != ModelModality::Image {
        return Err(CodexError::Image);
    }
    let workspace = Scratch::new()?;
    let executable = installed_codex().ok_or(CodexError::Unavailable)?;
    let mut client = Client::start(&executable, workspace.path(), &[]).await?;
    let prepare = async {
        client.prepare(&executable, workspace.path()).await?;
        let (model, effort) = client
            .select_model(
                request.modality,
                request.model.as_deref(),
                request.effort.as_deref(),
            )
            .await?;
        let thread = client
            .start_thread(&request, workspace.path(), &model)
            .await?;
        Ok::<_, CodexError>((thread, model, effort))
    };
    let result = tokio::select! {
        ready = prepare => ready,
        _ = cancelled.changed() => Err(CodexError::Cancelled)
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
    modality: ModelModality,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<(String, String)> {
    let workspace = Scratch::new()?;
    let executable = installed_codex().ok_or(CodexError::Unavailable)?;
    let mut client = Client::start(&executable, workspace.path(), &[]).await?;
    let result = async {
        client.prepare(&executable, workspace.path()).await?;
        client.select_model(modality, model, effort).await
    }
    .await;
    client.stop().await;
    result
}

/// Read models that accept the requested input modality.
pub async fn available_models(modality: ModelModality) -> Result<Vec<CodexModel>> {
    let workspace = Scratch::new()?;
    let executable = installed_codex().ok_or(CodexError::Unavailable)?;
    let mut client = Client::start(&executable, workspace.path(), &[]).await?;
    let result = async {
        client.prepare(&executable, workspace.path()).await?;
        client.available_models(modality).await
    }
    .await;
    client.stop().await;
    result
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
