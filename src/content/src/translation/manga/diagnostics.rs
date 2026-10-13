//! A bounded, content-free account of the page's tool calls.

use std::io::Write;
use std::time::{Duration, Instant};

use qbutt_codex::{CodexError, IMAGE_TOOL_CALL_LIMIT, Result, ToolCall};
use serde::Serialize;
use serde_json::json;

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum CallPhase {
    Validation,
    Native,
    Context,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolRecord {
    ordinal: usize,
    tool: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    layer: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crop: Option<&'static str>,
    phase: CallPhase,
    outcome: &'static str,
    elapsed_ms: u64,
    #[serde(skip_serializing)]
    started: Instant,
}

#[derive(Default)]
pub(super) struct ToolLedger {
    calls: Vec<ToolRecord>,
}

impl ToolLedger {
    pub(super) fn begin(&mut self, call: &ToolCall) -> Result<()> {
        if self.calls.len() >= IMAGE_TOOL_CALL_LIMIT {
            return Err(CodexError::Tool("Manga tool limit exceeded".into()));
        }
        let tool = match call.name.as_str() {
            "inspect_page" => "inspect_page",
            "render_page" => "render_page",
            "list_chapters" => "list_chapters",
            "get_chapter" => "get_chapter",
            "search_work_text" => "search_work_text",
            "read_context" => "read_context",
            _ => "unknown",
        };
        let (layer, crop) = if tool == "inspect_page" {
            let layer = match call.arguments.get("layer").and_then(|value| value.as_str()) {
                Some("original") => "original",
                Some("mask") => "mask",
                Some("cleaned") => "cleaned",
                Some("result") => "result",
                Some("comparison") => "comparison",
                _ => "invalid",
            };
            let crop = if call
                .arguments
                .get("crop")
                .is_some_and(|value| !value.is_null())
            {
                "requested"
            } else {
                "none"
            };
            (Some(layer), Some(crop))
        } else {
            (None, None)
        };
        self.calls.push(ToolRecord {
            ordinal: self.calls.len() + 1,
            tool,
            layer,
            crop,
            phase: CallPhase::Validation,
            outcome: "pending",
            elapsed_ms: 0,
            started: Instant::now(),
        });
        Ok(())
    }

    pub(super) fn phase(&mut self, phase: CallPhase) {
        self.calls
            .last_mut()
            .expect("Tool dispatch has a record")
            .phase = phase;
    }

    pub(super) fn finish<T>(&mut self, result: &Result<T>) {
        let record = self.calls.last_mut().expect("Tool dispatch has a record");
        record.outcome = outcome(result);
        record.elapsed_ms = record.started.elapsed().as_millis() as u64;
    }

    pub(super) fn report<T>(&mut self, result: &Result<T>, elapsed: Duration) {
        for record in &mut self.calls {
            if record.outcome == "pending" {
                record.elapsed_ms = record.started.elapsed().as_millis() as u64;
            }
        }
        let _ = writeln!(
            std::io::stderr(),
            "QBUTT_MANGA_TOOLS {}",
            json!({
                "callLimit": IMAGE_TOOL_CALL_LIMIT,
                "elapsedMs": elapsed.as_millis() as u64,
                "turnOutcome": outcome(result),
                "calls": self.calls,
            })
        );
    }
}

fn outcome<T>(result: &Result<T>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(CodexError::Unavailable) => "unavailable",
        Err(CodexError::Authentication) => "authentication",
        Err(CodexError::ModelUnavailable) => "model_unavailable",
        Err(CodexError::Cancelled) => "cancelled",
        Err(CodexError::Timeout) => "timeout",
        Err(CodexError::Isolation) => "isolation",
        Err(CodexError::Protocol) => "protocol",
        Err(CodexError::Provider) => "provider",
        Err(CodexError::Image) => "image",
        Err(CodexError::Tool(_)) => "tool_error",
    }
}
