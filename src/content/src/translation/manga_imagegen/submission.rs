//! One job owns visual feedback, the latest valid map and native preparation.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use serde_json::{Value, json};
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

pub(super) struct PageTools<'a, R, C> {
    job: &'a PageJob,
    renderer: Arc<R>,
    context: &'a mut C,
    cancelled: watch::Receiver<bool>,
    window: Option<&'a windows::Inspection>,
    inspected_pixels: &'a mut u64,
    submissions: usize,
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
            submissions: 0,
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
            if *self.cancelled.borrow() {
                return Err(CodexError::Cancelled);
            }
            if call.name != "submit_page" {
                return self.inspect_context(call).await;
            }
            if self.submissions == MAX_SUBMISSIONS {
                return Err(CodexError::Tool(
                    "All three page submissions were evaluated; accept the latest valid preview or decline".into(),
                ));
            }
            self.submissions += 1;
            let (candidate, prepared) = self.submit(call.arguments).await.map_err(|error| {
                CodexError::Tool(format!(
                    "Page submission {}/{MAX_SUBMISSIONS} rejected: {error}",
                    self.submissions
                ))
            })?;
            self.candidate = Some(candidate);
            Ok(ToolOutput {
                text: json!({
                    "validated": true,
                    "submission": self.submissions,
                    "remainingSubmissions": MAX_SUBMISSIONS - self.submissions,
                    "diagnostics": prepared.diagnostics,
                })
                .to_string(),
                images: prepared.previews,
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
