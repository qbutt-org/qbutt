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

mod imagegen;
mod imagegen_artifact;
mod model;
mod process;
mod rpc;
mod turn;

pub use imagegen::{ImageGenerationCompletion, MAX_GENERATED_IMAGE_BYTES, generate_image};
use imagegen_artifact::ImageArtifact;
pub use model::CodexModel;
use process::{Scratch, installed_codex};
pub use turn::IMAGE_TOOL_CALL_LIMIT;

/// Encoded image bytes in one input or, in aggregate, one tool response.
pub const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
/// Separate images in one tool response.
pub const MAX_TOOL_IMAGES: usize = 24;

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
    #[error("Image generation is unavailable in the installed Codex")]
    ImageGenerationUnavailable,
    #[error("Codex generated image cleanup failed: {reason}")]
    ArtifactCleanup {
        reason: String,
        #[source]
        primary: Option<Box<CodexError>>,
    },
    #[error("The image is too large or has an unsupported format")]
    Image,
    #[error("{0}")]
    Tool(String),
    #[error("{0}")]
    Declined(String),
}

impl CodexError {
    fn artifact_cleanup(error: std::io::Error, primary: Option<Self>) -> Self {
        let reason = match &primary {
            Some(primary) => format!("{error}; original request failed: {primary}"),
            None => error.to_string(),
        };
        Self::ArtifactCleanup {
            reason,
            primary: primary.map(Box::new),
        }
    }
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
    pub images: Vec<InputImage>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestMode {
    Structured,
    ImageGeneration,
}

enum Completion {
    Structured(CodexCompletion),
    ImageGeneration(ImageGenerationCompletion),
}

/// Structured inference keeps all built-in tools disabled. Cancellation drops
/// setup immediately; an active turn is interrupted and its process is reaped.
pub async fn complete(
    request: CodexRequest,
    tools: &mut impl ToolHandler,
    cancelled: watch::Receiver<bool>,
) -> Result<CodexCompletion> {
    match infer(request, tools, cancelled, RequestMode::Structured).await? {
        Completion::Structured(result) => Ok(result),
        Completion::ImageGeneration(_) => Err(CodexError::Isolation),
    }
}

async fn infer(
    request: CodexRequest,
    tools: &mut impl ToolHandler,
    mut cancelled: watch::Receiver<bool>,
    mode: RequestMode,
) -> Result<Completion> {
    if *cancelled.borrow() {
        return Err(CodexError::Cancelled);
    }
    if request.image.is_some() && request.modality != ModelModality::Image {
        return Err(CodexError::Image);
    }
    let workspace = Scratch::new()?;
    let executable = installed_codex().ok_or(CodexError::Unavailable)?;
    let mut client = Client::start(&executable, workspace.path(), &[], mode).await?;
    let prepare = async {
        client.prepare(&executable, workspace.path()).await?;
        if mode == RequestMode::ImageGeneration {
            let capabilities = client
                .request("modelProvider/capabilities/read", serde_json::json!({}))
                .await?;
            if capabilities.get("imageGeneration") != Some(&Value::Bool(true)) {
                return Err(CodexError::ImageGenerationUnavailable);
            }
        }
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
        if let Some(artifact) = &mut client.artifact {
            artifact
                .claim_thread(&thread)
                .map_err(|error| CodexError::artifact_cleanup(error, None))?;
        }
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
    if let Err(error) = client.stop().await {
        return Err(CodexError::artifact_cleanup(error, result.err()));
    }
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
    let mut client =
        Client::start(&executable, workspace.path(), &[], RequestMode::Structured).await?;
    let result = async {
        client.prepare(&executable, workspace.path()).await?;
        client.select_model(modality, model, effort).await
    }
    .await;
    let _ = client.stop().await;
    result
}

/// Read models that accept the requested input modality.
pub async fn available_models(modality: ModelModality) -> Result<Vec<CodexModel>> {
    let workspace = Scratch::new()?;
    let executable = installed_codex().ok_or(CodexError::Unavailable)?;
    let mut client =
        Client::start(&executable, workspace.path(), &[], RequestMode::Structured).await?;
    let result = async {
        client.prepare(&executable, workspace.path()).await?;
        client.available_models(modality).await
    }
    .await;
    let _ = client.stop().await;
    result
}

struct Client {
    mode: RequestMode,
    artifact: Option<ImageArtifact>,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    queued: VecDeque<Value>,
    usage: Option<TokenUsage>,
    answer: Option<String>,
}
