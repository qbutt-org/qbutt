//! One built-in image edit plus the consumer's structured acceptance metadata.
//! The transport reclaims only its validated completed-event PNG after app-server reap.

use base64::Engine;
use serde_json::Value;
use tokio::sync::watch;

use super::{
    CodexError, CodexRequest, Completion, ModelModality, RequestMode, Result, TokenUsage,
    ToolHandler, infer,
};

pub const MAX_GENERATED_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENCODED_IMAGE_BYTES: usize = MAX_GENERATED_IMAGE_BYTES.div_ceil(3) * 4;
const MAX_REVISED_PROMPT_BYTES: usize = 64 * 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub struct ImageGenerationCompletion {
    pub value: Value,
    pub png: Vec<u8>,
    pub revised_prompt: Option<String>,
    pub model: String,
    pub effort: String,
    /// Orchestrator-turn usage. The built-in image backend has separate usage
    /// that the current app-server item does not expose.
    pub usage: Option<TokenUsage>,
}

/// Enables only the built-in image-generation tool for this isolated request.
/// The caller supplies its own output schema and read-only context tools.
/// Both a completed PNG event and the final JSON are required for success.
pub async fn generate_image(
    request: CodexRequest,
    tools: &mut impl ToolHandler,
    cancelled: watch::Receiver<bool>,
) -> Result<ImageGenerationCompletion> {
    if request.modality != ModelModality::Image || request.image.is_none() {
        return Err(CodexError::Image);
    }
    match infer(request, tools, cancelled, RequestMode::ImageGeneration).await? {
        Completion::ImageGeneration(result) => Ok(result),
        Completion::Structured(_) => Err(CodexError::Isolation),
    }
}

#[derive(Default)]
pub(super) struct ImageGenerationState {
    call_id: Option<String>,
    output: Option<(Vec<u8>, Option<String>)>,
}

impl ImageGenerationState {
    pub(super) fn context_tools_allowed(&self) -> bool {
        self.call_id.is_none()
    }

    pub(super) fn start(&mut self, item: &Value) -> Result<()> {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or(CodexError::Protocol)?;
        if self.call_id.is_some()
            || id.is_empty()
            || id.len() > 256
            || item.get("status").and_then(Value::as_str) != Some("in_progress")
        {
            return Err(CodexError::Protocol);
        }
        self.call_id = Some(id.to_owned());
        Ok(())
    }

    pub(super) fn complete(&mut self, item: &Value) -> Result<&[u8]> {
        if self.call_id.is_none()
            || item.get("id").and_then(Value::as_str) != self.call_id.as_deref()
            || self.output.is_some()
        {
            return Err(CodexError::Protocol);
        }
        match item.get("status").and_then(Value::as_str) {
            Some("completed") => {}
            Some("failed") => return Err(CodexError::Provider),
            _ => return Err(CodexError::Protocol),
        }
        let encoded = item
            .get("result")
            .and_then(Value::as_str)
            .ok_or(CodexError::Protocol)?;
        if encoded.len() > MAX_ENCODED_IMAGE_BYTES {
            return Err(CodexError::Image);
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| CodexError::Image)?;
        if bytes.len() > MAX_GENERATED_IMAGE_BYTES || !bytes.starts_with(PNG_SIGNATURE) {
            return Err(CodexError::Image);
        }
        let revised_prompt = match item.get("revisedPrompt") {
            None | Some(Value::Null) => None,
            Some(Value::String(prompt)) if prompt.len() <= MAX_REVISED_PROMPT_BYTES => {
                Some(prompt.clone())
            }
            _ => return Err(CodexError::Protocol),
        };
        let (png, _) = self.output.insert((bytes, revised_prompt));
        Ok(png)
    }

    pub(super) fn finish(
        self,
        value: Value,
        model: String,
        effort: String,
        usage: Option<TokenUsage>,
    ) -> Result<ImageGenerationCompletion> {
        let (png, revised_prompt) = self.output.ok_or(CodexError::ImageGenerationUnavailable)?;
        Ok(ImageGenerationCompletion {
            value,
            png,
            revised_prompt,
            model,
            effort,
            usage,
        })
    }
}
