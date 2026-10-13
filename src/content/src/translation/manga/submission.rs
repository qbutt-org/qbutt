//! Native-render proposals reuse the image page's context evidence contract.

use serde::Deserialize;
use serde_json::{Value, json};

use super::super::context::{ContextAdditions, EvidenceSegment};
use super::super::page::{self, CandidateFact, CandidateTerm, PageJob, SourceObservation};
use super::{MAX_REGIONS, MAX_RENDERS, PageRegion};
use qbutt_codex::{self as codex, CodexError};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PageSelection {
    pub(super) revision: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RenderCandidate {
    pub(super) regions: Vec<PageRegion>,
    terms: Vec<CandidateTerm>,
    facts: Vec<CandidateFact>,
}

impl PageSelection {
    pub(super) fn validate_selection(&self, render_count: usize) -> codex::Result<()> {
        if self.revision == 0 || self.revision > render_count {
            return Err(CodexError::Tool(
                "Codex selected an invalid page revision".into(),
            ));
        }
        Ok(())
    }
}

impl RenderCandidate {
    pub(super) fn decode_context(
        &self,
        job: &PageJob,
    ) -> codex::Result<(ContextAdditions, Vec<EvidenceSegment>)> {
        page::decode_context(
            job,
            self.regions.iter().map(|region| SourceObservation {
                id: &region.id,
                text: &region.source_text,
                bounds: &region.bounds,
            }),
            &self.terms,
            &self.facts,
        )
    }
}

pub(super) fn render_candidate_schema(region: Value) -> Value {
    let mut properties = page::context_properties();
    properties["regions"] = json!({
        "type": "array",
        "maxItems": MAX_REGIONS,
        "items": region,
    });
    json!({
        "type": "object",
        "properties": properties,
        "required": ["regions", "terms", "facts"],
        "additionalProperties": false,
    })
}

pub(super) fn page_selection_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "revision": { "type": "integer", "minimum": 1, "maximum": MAX_RENDERS },
        },
        "required": ["revision"],
        "additionalProperties": false,
    })
}
