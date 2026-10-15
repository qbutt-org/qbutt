//! One job owns visual feedback, the latest valid map and native preparation.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use qbutt_codex::{
    self as codex, CodexError, FunctionTool, InputImage, ToolCall, ToolFuture, ToolHandler,
    ToolOutput,
};

use super::{MAX_INSPECTED_PIXELS, NativeCancellation, context_tools, native_work, windows};
use crate::translation::context::{ContextAdditions, EvidenceSegment};
use crate::translation::page::{
    self, CandidateFact, CandidateTerm, PageJob, PageRegion, PageRenderer, PreparedPage,
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

pub(super) struct PageCandidate {
    pub(super) regions: Vec<PageRegion>,
    pub(super) clipped: Vec<page::Rect>,
    pub(super) additions: ContextAdditions,
    pub(super) evidence: Vec<EvidenceSegment>,
}

struct NativePreparation {
    cancellation: NativeCancellation,
    task: JoinHandle<codex::Result<(PreparedPage, u64)>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmissionDiagnostic {
    outcome: &'static str,
    reason: Option<&'static str>,
    preview_images: usize,
    preview_bytes: usize,
}

impl SubmissionDiagnostic {
    fn reject(&mut self, error: &CodexError) {
        self.outcome = if matches!(error, CodexError::Cancelled) {
            "cancelled"
        } else {
            "rejected"
        };
        self.reason = match error {
            CodexError::Cancelled => Some("cancelled"),
            CodexError::Timeout => Some("timed out"),
            CodexError::Image => Some("invalid or oversized image"),
            CodexError::Tool(_) => self.reason,
            _ => Some("provider or protocol failure"),
        };
    }
}

pub(super) struct PageTools<'a, R, C> {
    job: &'a PageJob,
    renderer: Arc<R>,
    context: &'a mut C,
    cancelled: watch::Receiver<bool>,
    window: Option<&'a windows::Inspection>,
    inspected_pixels: &'a mut u64,
    tool_calls: usize,
    submit_calls: usize,
    limit_rejections: usize,
    submissions: Vec<SubmissionDiagnostic>,
    pub(super) model_decision: Option<bool>,
    pub(super) outcome: &'static str,
    preparation: Option<NativePreparation>,
    pub(super) candidate: Option<PageCandidate>,
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
            tool_calls: 0,
            submit_calls: 0,
            limit_rejections: 0,
            submissions: Vec::new(),
            model_decision: None,
            outcome: "interrupted",
            preparation: None,
            candidate: None,
        }
    }

    // The transport may cancel and drop a tool future while Qt is working.
    // Retain its handle here and drain it before the page request returns.
    pub(super) async fn drain_preparation(&mut self) {
        if let Some(preparation) = self.preparation.take() {
            preparation.cancellation.0.store(true, Ordering::Release);
            let _ = preparation.task.await;
        }
    }

    async fn submit(&mut self, mut value: Value) -> codex::Result<(PageCandidate, PreparedPage)> {
        let diagnostic = self.submissions.last_mut().unwrap();
        let clipped = match self.window {
            Some(window) => window.validate(&mut value)?,
            None => Vec::new(),
        };
        diagnostic.reason = Some("malformed proposal");
        let proposal: TranslationProposal = serde_json::from_value(value)
            .map_err(|_| CodexError::Tool("Invalid translation proposal".into()))?;
        let (additions, evidence) = if self.window.is_some() && proposal.regions.is_empty() {
            diagnostic.reason = Some("context evidence validation");
            if !proposal.terms.is_empty() || !proposal.facts.is_empty() {
                return Err(CodexError::Tool(
                    "An empty window cannot add context".into(),
                ));
            }
            (ContextAdditions::default(), Vec::new())
        } else {
            diagnostic.reason = Some("region geometry or style validation");
            page::validate_regions(&proposal.regions)?;
            diagnostic.reason = Some("context evidence validation");
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
        diagnostic.reason = Some("window or seam validation");
        if let Some(window) = self.window {
            window.validate_regions(&proposal.regions, &clipped)?;
        }
        diagnostic.reason = Some("native typography or preview validation");
        let prepared = if proposal.regions.is_empty() {
            PreparedPage {
                diagnostics: Vec::new(),
                previews: Vec::new(),
            }
        } else {
            let original = self.job.original.clone();
            let regions = proposal.regions.clone();
            let renderer = self.renderer.clone();
            let cancellation = NativeCancellation(Arc::new(AtomicBool::new(false)));
            let flag = cancellation.0.clone();
            let preview_pixels = MAX_INSPECTED_PIXELS
                .saturating_sub(*self.inspected_pixels)
                .min(16_000_000);
            self.preparation = Some(NativePreparation {
                cancellation,
                task: tokio::task::spawn_blocking(move || {
                    let prepared =
                        renderer.prepare(&original, &regions, preview_pixels, flag.clone())?;
                    page::validate_rendered_regions(&regions, &prepared.diagnostics)?;
                    if prepared.previews.is_empty() {
                        return Err(CodexError::Tool("Native page previews are missing".into()));
                    }
                    let pixels = image_batch_pixels(renderer.as_ref(), &prepared.previews, flag)?;
                    if pixels > preview_pixels {
                        return Err(CodexError::Tool(
                            "The page image inspection budget is exhausted".into(),
                        ));
                    }
                    Ok((prepared, pixels))
                }),
            });
            let result = (&mut self.preparation.as_mut().unwrap().task).await;
            self.preparation = None;
            let (prepared, pixels) = result.map_err(|_| CodexError::Provider)??;
            *self.inspected_pixels += pixels;
            prepared
        };
        if *self.cancelled.borrow() {
            return Err(CodexError::Cancelled);
        }
        Ok((
            PageCandidate {
                regions: proposal.regions,
                clipped,
                additions,
                evidence,
            },
            prepared,
        ))
    }

    async fn inspect_context(&mut self, call: ToolCall) -> codex::Result<ToolOutput> {
        let output = self.context.call(call).await?;
        if output.images.is_empty() {
            return Ok(output);
        }
        let renderer = self.renderer.clone();
        let (output, batch_pixels) = native_work(self.cancelled.clone(), move |flag| {
            let pixels = image_batch_pixels(renderer.as_ref(), &output.images, flag)?;
            Ok((output, pixels))
        })
        .await?;
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
            self.tool_calls += 1;
            if call.name == "submit_page" {
                self.submit_calls += 1;
            }
            if *self.cancelled.borrow() {
                return Err(CodexError::Cancelled);
            }
            if call.name != "submit_page" {
                return self.inspect_context(call).await;
            }
            if self.submissions.len() == MAX_SUBMISSIONS {
                self.limit_rejections += 1;
                return Err(CodexError::Tool(
                    "All three page submissions were evaluated; accept the latest valid preview or decline".into(),
                ));
            }
            self.submissions.push(SubmissionDiagnostic {
                outcome: "evaluating",
                reason: Some("window or seam validation"),
                preview_images: 0,
                preview_bytes: 0,
            });
            let (candidate, prepared) = match self.submit(call.arguments).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.submissions.last_mut().unwrap().reject(&error);
                    return Err(CodexError::Tool(format!(
                        "Page submission {}/{MAX_SUBMISSIONS} rejected: {error}",
                        self.submissions.len()
                    )));
                }
            };
            let diagnostic = self.submissions.last_mut().unwrap();
            diagnostic.outcome = "validated";
            diagnostic.reason = None;
            diagnostic.preview_images = prepared.previews.len();
            diagnostic.preview_bytes = prepared
                .previews
                .iter()
                .map(|image| image.bytes.len())
                .sum();
            self.candidate = Some(candidate);
            Ok(ToolOutput {
                text: json!({
                    "validated": true,
                    "submission": self.submissions.len(),
                    "remainingSubmissions": MAX_SUBMISSIONS - self.submissions.len(),
                    "diagnostics": prepared.diagnostics,
                })
                .to_string(),
                images: prepared.previews,
            })
        })
    }
}

impl<R, C> Drop for PageTools<'_, R, C> {
    fn drop(&mut self) {
        let cancelled = *self.cancelled.borrow();
        if cancelled {
            self.outcome = "cancelled";
        }
        for diagnostic in &mut self.submissions {
            if diagnostic.outcome == "evaluating" {
                diagnostic.outcome = if cancelled {
                    "cancelled"
                } else {
                    "interrupted"
                };
                diagnostic.reason = Some("tool evaluation interrupted");
            }
        }
        let preview_images: usize = self
            .submissions
            .iter()
            .map(|item| item.preview_images)
            .sum();
        let preview_bytes: usize = self.submissions.iter().map(|item| item.preview_bytes).sum();
        let summary = json!({
            "originalDigest": format!("{:x}", Sha256::digest(&self.job.original.bytes)),
            "page": self.job.page,
            "windowTop": self.window.map(|window| window.window.top),
            "toolCalls": self.tool_calls,
            "contextCalls": self.tool_calls - self.submit_calls,
            "submitCalls": self.submit_calls,
            "evaluatedSubmissions": self.submissions.len(),
            "limitRejections": self.limit_rejections,
            "returnedPreviewImages": preview_images,
            "returnedPreviewBytes": preview_bytes,
            "modelDecision": self.model_decision,
            "outcome": self.outcome,
            "submissions": self.submissions,
        });
        // Only bounded counters and fixed diagnostic categories leave this owner.
        // A closed stderr must not change translation or cancellation semantics.
        let _ = writeln!(
            std::io::stderr().lock(),
            "MANGA_SUBMISSION_SUMMARY {summary}"
        );
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
        description: "Validate the complete map and inspect native source/cleanup-coverage/lettering previews. Correct and replace the latest valid map within three submissions; only final acceptance starts cleanup.",
        input_schema: schema,
    });
    tools
}

fn image_batch_pixels<R: PageRenderer>(
    renderer: &R,
    images: &[InputImage],
    cancelled: Arc<AtomicBool>,
) -> codex::Result<u64> {
    if images.len() > codex::MAX_TOOL_IMAGES {
        return Err(CodexError::Image);
    }
    let mut bytes = 0;
    let mut pixels = 0;
    for image in images {
        bytes += image.bytes.len();
        if image.bytes.is_empty() || bytes > codex::MAX_IMAGE_BYTES {
            return Err(CodexError::Image);
        }
        let (width, height) = renderer.dimensions(image, cancelled.clone())?;
        pixels += u64::from(width) * u64::from(height);
        if !windows::size_supported(width, height)
            || windows::needs_windows(width, height)
            || pixels > 16_000_000
        {
            return Err(CodexError::Tool(
                "Tool images exceed the 16000-pixel side or 16M-pixel batch limit".into(),
            ));
        }
    }
    Ok(pixels)
}
