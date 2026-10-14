//! One job owns proposal feedback, accepted data and native preflight lifetime.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use qbutt_codex::{
    self as codex, CodexError, FunctionTool, ToolCall, ToolFuture, ToolHandler, ToolOutput,
};

use super::{MAX_INSPECTED_PIXELS, NativeCancellation, context_tools, native_work, windows};
use crate::translation::context::{ContextAdditions, EvidenceSegment};
use crate::translation::page::{
    self, CandidateFact, CandidateTerm, PageJob, PageRegion, PageRenderer, RegionDiagnostic,
    SourceObservation,
};

const MAX_SUBMISSIONS: usize = 3;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TranslationProposal {
    regions: Vec<PageRegion>,
    terms: Vec<CandidateTerm>,
    facts: Vec<CandidateFact>,
}

pub(super) struct AcceptedPage {
    pub(super) regions: Vec<PageRegion>,
    pub(super) clipped: Vec<page::Rect>,
    pub(super) additions: ContextAdditions,
    pub(super) evidence: Vec<EvidenceSegment>,
}

struct NativePreflight {
    cancellation: NativeCancellation,
    task: JoinHandle<codex::Result<Vec<RegionDiagnostic>>>,
}

pub(super) struct PageTools<'a, R, C> {
    job: &'a PageJob,
    renderer: Arc<R>,
    context: &'a mut C,
    cancelled: watch::Receiver<bool>,
    window: Option<&'a windows::Inspection>,
    inspected_pixels: &'a mut u64,
    submissions: usize,
    preflight: Option<NativePreflight>,
    pub(super) accepted: Option<AcceptedPage>,
}

impl<'a, R: PageRenderer, C: ToolHandler + Send> PageTools<'a, R, C> {
    pub(super) fn new(
        job: &'a PageJob,
        renderer: Arc<R>,
        context: &'a mut C,
        cancelled: watch::Receiver<bool>,
        window: Option<&'a windows::Inspection>,
        inspected_pixels: &'a mut u64,
    ) -> Self {
        Self {
            job,
            renderer,
            context,
            cancelled,
            window,
            inspected_pixels,
            submissions: 0,
            preflight: None,
            accepted: None,
        }
    }

    // The transport may cancel and drop a tool future while Qt is working.
    // Retain its handle here and drain it before the page request returns.
    pub(super) async fn drain_preflight(&mut self) {
        if let Some(preflight) = self.preflight.take() {
            preflight.cancellation.0.store(true, Ordering::Release);
            let _ = preflight.task.await;
        }
    }

    async fn submit(&mut self, mut value: Value) -> codex::Result<AcceptedPage> {
        let clipped = match self.window {
            Some(window) => window.validate(&mut value)?,
            None => Vec::new(),
        };
        let proposal: TranslationProposal = serde_json::from_value(value)
            .map_err(|_| CodexError::Tool("Invalid translation proposal".into()))?;
        let (additions, evidence) = if self.window.is_some() && proposal.regions.is_empty() {
            if !proposal.terms.is_empty() || !proposal.facts.is_empty() {
                return Err(CodexError::Tool(
                    "An empty window cannot add context".into(),
                ));
            }
            (ContextAdditions::default(), Vec::new())
        } else {
            page::validate_regions(&proposal.regions)?;
            let (additions, evidence) = page::decode_context(
                self.job,
                proposal.regions.iter().map(|region| SourceObservation {
                    id: &region.id,
                    text: &region.source_text,
                    bounds: &region.bounds,
                }),
                &proposal.terms,
                &proposal.facts,
            )?;
            let mut context = self.job.work_context.clone();
            context
                .apply(
                    &self.job.chapter.variant.publication,
                    additions.clone(),
                    &evidence,
                )
                .map_err(|reason| CodexError::Tool(reason.into()))?;
            (additions, evidence)
        };
        if let Some(window) = self.window {
            window.validate_regions(&proposal.regions, &clipped)?;
        }
        if !proposal.regions.is_empty() {
            let original = self.job.original.clone();
            let regions = proposal.regions.clone();
            let renderer = self.renderer.clone();
            let cancellation = NativeCancellation(Arc::new(AtomicBool::new(false)));
            let flag = cancellation.0.clone();
            self.preflight = Some(NativePreflight {
                cancellation,
                task: tokio::task::spawn_blocking(move || {
                    renderer.prepare(&original, &regions, flag)
                }),
            });
            let result = (&mut self.preflight.as_mut().unwrap().task).await;
            self.preflight = None;
            let diagnostics = result.map_err(|_| CodexError::Provider)??;
            page::validate_rendered_regions(&proposal.regions, &diagnostics)?;
        }
        if *self.cancelled.borrow() {
            return Err(CodexError::Cancelled);
        }
        Ok(AcceptedPage {
            regions: proposal.regions,
            clipped,
            additions,
            evidence,
        })
    }

    async fn inspect_context(&mut self, call: ToolCall) -> codex::Result<ToolOutput> {
        let output = self.context.call(call).await?;
        let mut batch_pixels = 0;
        for image in &output.images {
            let original = image.clone();
            let renderer = self.renderer.clone();
            let (width, height) = native_work(self.cancelled.clone(), move |flag| {
                renderer.dimensions(&original, flag)
            })
            .await?;
            batch_pixels += u64::from(width) * u64::from(height);
            if !windows::size_supported(width, height)
                || windows::needs_windows(width, height)
                || batch_pixels > 16_000_000
            {
                return Err(CodexError::Tool(
                    "Context images exceed the 16000-pixel side or 16M-pixel batch limit".into(),
                ));
            }
        }
        if self.inspected_pixels.saturating_add(batch_pixels) > MAX_INSPECTED_PIXELS {
            return Err(CodexError::Tool(
                "The page image inspection budget is exhausted".into(),
            ));
        }
        *self.inspected_pixels += batch_pixels;
        Ok(output)
    }
}

impl<R: PageRenderer, C: ToolHandler + Send> ToolHandler for PageTools<'_, R, C> {
    fn call(&mut self, call: ToolCall) -> ToolFuture<'_> {
        Box::pin(async move {
            if *self.cancelled.borrow() {
                return Err(CodexError::Cancelled);
            }
            if call.name != "submit_page" {
                return self.inspect_context(call).await;
            }
            if self.accepted.is_some() {
                return Err(CodexError::Tool(
                    "The page proposal is already accepted".into(),
                ));
            }
            if self.submissions == MAX_SUBMISSIONS {
                return Err(CodexError::Tool(
                    "All three page submissions were rejected; finalize with accepted=false".into(),
                ));
            }
            self.submissions += 1;
            let accepted = self.submit(call.arguments).await.map_err(|error| {
                CodexError::Tool(format!(
                    "Page submission {}/{MAX_SUBMISSIONS} rejected: {error}",
                    self.submissions
                ))
            })?;
            self.accepted = Some(accepted);
            Ok(ToolOutput {
                text: json!({"accepted": true}).to_string(),
                images: Vec::new(),
            })
        })
    }
}

pub(super) fn tool_definitions(windowed: bool) -> Vec<FunctionTool> {
    let mut tools: Vec<_> = context_tools::tool_definitions()
        .into_iter()
        .filter(|tool| tool.name != "search_work_text")
        .collect();
    let mut properties = page::context_properties();
    properties["regions"] = json!({
        "type": "array",
        "minItems": if windowed { 0 } else { 1 },
        "maxItems": page::MAX_REGIONS,
        "items": page::region_schema(),
    });
    let mut schema = json!({
        "type": "object",
        "properties": properties,
        "required": ["regions", "terms", "facts"],
        "additionalProperties": false,
    });
    if windowed {
        schema["properties"]["clippedRegions"] = windows::clipped_schema();
        schema["properties"]["unsupportedReason"] = json!({
            "type": ["string", "null"],
            "minLength": 1,
            "maxLength": 500,
        });
        schema["required"] = json!([
            "regions",
            "terms",
            "facts",
            "clippedRegions",
            "unsupportedReason"
        ]);
    }
    tools.push(FunctionTool {
        name: "submit_page",
        description: "Validate the complete translation map and native typography. Repair errors and resubmit at most three times; an accepted proposal is immutable.",
        input_schema: schema,
    });
    tools
}
