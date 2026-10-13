// Explicit native OpenRouter text inference. No environment key, model fallback,
// browser/source session, external tool or helper process belongs to this path.

use std::collections::HashSet;
use std::time::Duration;

use qbutt_codex::{FunctionTool, TokenUsage, ToolCall, ToolHandler};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::OpenRouterSettings;
use crate::secrets::Secret;

const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOOL_TEXT_BYTES: usize = 64 * 1024;
const MAX_TOOL_CALLS: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum OpenRouterError {
    #[error("Configure an OpenRouter model and reasoning effort")]
    Configuration,
    #[error("OpenRouter rejected the API key or model access")]
    Authentication,
    #[error("OpenRouter has insufficient credits or has reached its limit")]
    Limit,
    #[error("The OpenRouter connection failed")]
    Network,
    #[error("OpenRouter could not complete the request")]
    Provider,
    #[error("OpenRouter returned invalid or incomplete translation data")]
    Protocol,
    #[error("OpenRouter translation was cancelled")]
    Cancelled,
    #[error("OpenRouter translation timed out")]
    Timeout,
    #[error("OpenRouter requested an unavailable context tool")]
    Tool,
}

pub type Result<T> = std::result::Result<T, OpenRouterError>;

pub struct OpenRouterRequest {
    pub instructions: String,
    pub prompt: String,
    pub output_schema: Value,
    pub tools: Vec<FunctionTool>,
}

pub struct OpenRouterCompletion {
    pub value: Value,
    pub model: String,
    pub effort: String,
    pub usage: Option<TokenUsage>,
}

pub struct OpenRouter {
    http: reqwest::Client,
    settings: OpenRouterSettings,
    authorization: HeaderValue,
}

impl OpenRouter {
    pub(crate) fn new(settings: OpenRouterSettings, key: Secret) -> Result<Self> {
        if !settings.valid() {
            return Err(OpenRouterError::Configuration);
        }
        let authorization = key
            .authorization()
            .map_err(|_| OpenRouterError::Authentication)?;
        let http = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(COMPLETION_TIMEOUT)
            .build()
            .map_err(|_| OpenRouterError::Network)?;
        Ok(Self {
            http,
            settings,
            authorization,
        })
    }

    pub fn settings(&self) -> &OpenRouterSettings {
        &self.settings
    }

    pub async fn complete(
        &self,
        request: OpenRouterRequest,
        tools: &mut impl ToolHandler,
        mut cancelled: watch::Receiver<bool>,
    ) -> Result<OpenRouterCompletion> {
        if *cancelled.borrow() {
            return Err(OpenRouterError::Cancelled);
        }
        tokio::select! {
            biased;
            _ = cancelled.changed() => Err(OpenRouterError::Cancelled),
            result = tokio::time::timeout(COMPLETION_TIMEOUT, self.run(request, tools)) => {
                result.map_err(|_| OpenRouterError::Timeout)?
            }
        }
    }

    async fn run(
        &self,
        request: OpenRouterRequest,
        tools: &mut impl ToolHandler,
    ) -> Result<OpenRouterCompletion> {
        let definitions: Vec<_> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema
                    }
                })
            })
            .collect();
        let mut messages = vec![
            json!({"role": "system", "content": request.instructions}),
            json!({"role": "user", "content": request.prompt}),
        ];
        let mut call_ids = HashSet::new();
        let mut usage = None;
        loop {
            let mut body = json!({
                "model": self.settings.model,
                "messages": messages,
                "tools": definitions,
                "parallel_tool_calls": false,
                "provider": {"require_parameters": true},
                "plugins": [],
                "stream": false,
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "qbutt_text_translation",
                        "strict": true,
                        "schema": request.output_schema
                    }
                }
            });
            if self.settings.effort != "default" {
                body["reasoning"] = json!({"effort": self.settings.effort});
            }
            let response = self.send(body).await?;
            if response.get("error").is_some_and(|error| !error.is_null())
                || response.get("model").and_then(Value::as_str) != Some(&self.settings.model)
            {
                return Err(OpenRouterError::Provider);
            }
            accumulate_usage(&mut usage, response.get("usage"))?;
            let choices = response
                .get("choices")
                .and_then(Value::as_array)
                .filter(|choices| choices.len() == 1)
                .ok_or(OpenRouterError::Protocol)?;
            let choice = &choices[0];
            let message = choice
                .get("message")
                .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
                .ok_or(OpenRouterError::Protocol)?;
            match choice.get("finish_reason").and_then(Value::as_str) {
                Some("stop") => {
                    if message
                        .get("tool_calls")
                        .is_some_and(|calls| !calls.is_null() && calls != &json!([]))
                    {
                        return Err(OpenRouterError::Protocol);
                    }
                    let content = message
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or(OpenRouterError::Protocol)?;
                    let value =
                        serde_json::from_str(content).map_err(|_| OpenRouterError::Protocol)?;
                    return Ok(OpenRouterCompletion {
                        value,
                        model: self.settings.model.clone(),
                        effort: self.settings.effort.clone(),
                        usage,
                    });
                }
                Some("tool_calls") => {
                    let calls = message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .filter(|calls| !calls.is_empty())
                        .ok_or(OpenRouterError::Protocol)?;
                    if call_ids.len() + calls.len() > MAX_TOOL_CALLS {
                        return Err(OpenRouterError::Tool);
                    }
                    messages.push(message.clone());
                    for call in calls {
                        let id = call
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|id| {
                                !id.is_empty()
                                    && id.len() <= 256
                                    && !id.chars().any(char::is_control)
                            })
                            .ok_or(OpenRouterError::Protocol)?;
                        let name = call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .ok_or(OpenRouterError::Protocol)?;
                        if call.get("type").and_then(Value::as_str) != Some("function")
                            || !request.tools.iter().any(|tool| tool.name == name)
                            || !call_ids.insert(id.to_owned())
                        {
                            return Err(OpenRouterError::Tool);
                        }
                        let arguments = call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .ok_or(OpenRouterError::Protocol)?;
                        let arguments = serde_json::from_str(arguments)
                            .map_err(|_| OpenRouterError::Protocol)?;
                        let content = match tools
                            .call(ToolCall {
                                name: name.into(),
                                arguments,
                            })
                            .await
                        {
                            Ok(output) => {
                                if !output.images.is_empty()
                                    || output.text.len() > MAX_TOOL_TEXT_BYTES
                                {
                                    return Err(OpenRouterError::Tool);
                                }
                                output.text
                            }
                            Err(qbutt_codex::CodexError::Cancelled) => {
                                return Err(OpenRouterError::Cancelled);
                            }
                            Err(error) => json!({"error": error.to_string()}).to_string(),
                        };
                        if content.len() > MAX_TOOL_TEXT_BYTES {
                            return Err(OpenRouterError::Tool);
                        }
                        messages
                            .push(json!({"role": "tool", "tool_call_id": id, "content": content}));
                    }
                }
                _ => return Err(OpenRouterError::Protocol),
            }
        }
    }

    async fn send(&self, body: Value) -> Result<Value> {
        let body = serde_json::to_vec(&body).map_err(|_| OpenRouterError::Protocol)?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(OpenRouterError::Protocol);
        }
        let mut response = self
            .http
            .post(ENDPOINT)
            .header(AUTHORIZATION, self.authorization.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(network_error)?;
        match response.status().as_u16() {
            200 => {}
            401 | 403 => return Err(OpenRouterError::Authentication),
            402 | 429 => return Err(OpenRouterError::Limit),
            _ => return Err(OpenRouterError::Provider),
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(OpenRouterError::Protocol);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| OpenRouterError::Network)?
        {
            if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
                return Err(OpenRouterError::Protocol);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| OpenRouterError::Protocol)
    }
}

fn accumulate_usage(total: &mut Option<TokenUsage>, value: Option<&Value>) -> Result<()> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let required = |name| {
        value
            .get(name)
            .and_then(Value::as_u64)
            .ok_or(OpenRouterError::Protocol)
    };
    let usage = total.get_or_insert_with(TokenUsage::default);
    for (counter, count) in [
        (&mut usage.total_tokens, required("total_tokens")?),
        (&mut usage.input_tokens, required("prompt_tokens")?),
        (&mut usage.output_tokens, required("completion_tokens")?),
        (
            &mut usage.cached_input_tokens,
            value
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        (
            &mut usage.reasoning_output_tokens,
            value
                .pointer("/completion_tokens_details/reasoning_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
    ] {
        *counter = counter
            .checked_add(count)
            .ok_or(OpenRouterError::Protocol)?;
    }
    Ok(())
}

fn network_error(error: reqwest::Error) -> OpenRouterError {
    if error.is_timeout() {
        OpenRouterError::Timeout
    } else {
        OpenRouterError::Network
    }
}
