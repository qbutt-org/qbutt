use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::time::{Instant, timeout_at};

use super::{
    Client, CodexCompletion, CodexError, CodexRequest, FunctionTool, ModelModality, Result,
    ToolCall, ToolHandler,
};

const TURN_TIMEOUT: Duration = Duration::from_secs(180);
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(5);
const TEXT_TOOL_CALL_LIMIT: usize = 12;
pub const IMAGE_TOOL_CALL_LIMIT: usize = 24;

impl Client {
    pub(super) async fn start_thread(
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
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema
                })
            })
            .collect();
        let result = self
            .request(
                "thread/start",
                json!({
                    "model": model,
                    "modelProvider": "openai",
                    "cwd": cwd,
                    "approvalPolicy": "never",
                    "sandbox": "read-only",
                    "ephemeral": true,
                    "baseInstructions": request.instructions,
                    "dynamicTools": tools,
                    "selectedCapabilityRoots": [],
                    "environments": [],
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

    pub(super) async fn run_turn(
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
        let start = self.request(
            "turn/start",
            json!({
                "threadId": thread,
                "input": input,
                "effort": effort,
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "readOnly", "networkAccess": false},
                "outputSchema": request.output_schema
            }),
        );
        let result = tokio::select! {
            ready = start => ready?,
            _ = cancelled.changed() => return Err(CodexError::Cancelled),
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
        let tool_call_limit = match request.modality {
            ModelModality::Text => TEXT_TOOL_CALL_LIMIT,
            ModelModality::Image => IMAGE_TOOL_CALL_LIMIT,
        };
        let mut tool_calls = 0;
        loop {
            let message = if let Some(queued) = self.queued.pop_front() {
                queued
            } else {
                tokio::select! {
                    biased;
                    _ = cancelled.changed() => {
                        self.interrupt(&thread, &turn_id).await;
                        return Err(CodexError::Cancelled);
                    }
                    response = self.read(deadline) => response?,
                }
            };
            match message.get("method").and_then(Value::as_str) {
                Some("item/tool/call") => {
                    tool_calls += 1;
                    if tool_calls > tool_call_limit {
                        self.interrupt(&thread, &turn_id).await;
                        return Err(CodexError::Tool("Codex tool limit exceeded".into()));
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
            _ = cancelled.changed() => {
                self.interrupt(thread, turn).await;
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
            Err(error) => json!({
                "id": id,
                "result": {
                    "success": false,
                    "contentItems": [{"type": "inputText", "text": error.to_string()}]
                }
            }),
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
