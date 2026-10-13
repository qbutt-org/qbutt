//! A render candidate supplies its context evidence before the final selection.

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ChapterRef;

use super::super::context::{
    ConfirmedFact, ContextAdditions, ContextEvidence, EvidenceSegment, ImageObservation,
    TranslationForm, TranslationTerm,
};
use super::{MAX_REGIONS, MAX_RENDERS, PageRegion, Rect};
use qbutt_codex::{self as codex, CodexError, InputImage};

const MAX_CONTEXT_ITEMS: usize = 20;

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateEvidence {
    region_id: String,
    quote: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateTerm {
    source: String,
    target: String,
    forms: Vec<CandidateForm>,
    evidence: CandidateEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateForm {
    source: String,
    target: String,
    evidence: CandidateEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateFact {
    subject: String,
    attribute: String,
    value: String,
    evidence: CandidateEvidence,
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
        chapter: &ChapterRef,
        page: u32,
        original: &InputImage,
    ) -> codex::Result<(ContextAdditions, Vec<EvidenceSegment>)> {
        if self.terms.len() > MAX_CONTEXT_ITEMS || self.facts.len() > MAX_CONTEXT_ITEMS {
            return Err(CodexError::Tool(
                "Provide at most 20 context terms and 20 facts".into(),
            ));
        }
        let digest = format!("{:x}", Sha256::digest(&original.bytes));
        let segments: Vec<_> = self
            .regions
            .iter()
            .map(|region| EvidenceSegment {
                chapter: chapter.clone(),
                id: region.id.clone(),
                text: region.source_text.clone(),
                image_observation: Some(ImageObservation {
                    page,
                    original_digest: digest.clone(),
                    bounds: observation_bounds(&region.bounds),
                }),
            })
            .collect();

        let evidence = |raw: &CandidateEvidence| -> codex::Result<ContextEvidence> {
            let segment = segments
                .iter()
                .find(|segment| segment.id == raw.region_id)
                .ok_or_else(|| {
                    CodexError::Tool(
                        "Context evidence must cite a region in this render candidate".into(),
                    )
                })?;
            Ok(ContextEvidence {
                chapter: segment.chapter.clone(),
                segment_id: segment.id.clone(),
                quote: raw.quote.clone(),
                image_observation: segment.image_observation.clone(),
            })
        };

        let terms = self
            .terms
            .iter()
            .map(|term| -> codex::Result<TranslationTerm> {
                let forms = term
                    .forms
                    .iter()
                    .map(|form| {
                        Ok(TranslationForm {
                            source: form.source.clone(),
                            target: form.target.clone(),
                            evidence: evidence(&form.evidence)?,
                        })
                    })
                    .collect::<codex::Result<_>>()?;

                Ok(TranslationTerm {
                    source: term.source.clone(),
                    target: term.target.clone(),
                    forms,
                    evidence: evidence(&term.evidence)?,
                })
            })
            .collect::<codex::Result<_>>()?;
        let facts = self
            .facts
            .iter()
            .map(|fact| {
                Ok(ConfirmedFact {
                    subject: fact.subject.clone(),
                    attribute: fact.attribute.clone(),
                    value: fact.value.clone(),
                    evidence: evidence(&fact.evidence)?,
                })
            })
            .collect::<codex::Result<_>>()?;

        Ok((ContextAdditions { terms, facts }, segments))
    }
}

fn observation_bounds(bounds: &Rect) -> [u32; 4] {
    let left = ((bounds.x * 1_000_000.0).round() as u32).min(999_999);
    let top = ((bounds.y * 1_000_000.0).round() as u32).min(999_999);
    let right = (((bounds.x + bounds.width) * 1_000_000.0).round() as u32)
        .max(left + 1)
        .min(1_000_000);
    let bottom = (((bounds.y + bounds.height) * 1_000_000.0).round() as u32)
        .max(top + 1)
        .min(1_000_000);
    [left, top, right, bottom]
}

pub(super) fn render_candidate_schema(region: Value) -> Value {
    let text = |limit| {
        json!({
            "type": "string",
            "minLength": 1,
            "maxLength": limit,
        })
    };
    let evidence = json!({
        "type": "object",
        "properties": {
            "region_id": text(64),
            "quote": text(500),
        },
        "required": ["region_id", "quote"],
        "additionalProperties": false,
    });
    let form = json!({
        "type": "object",
        "description": "The form's source must occur with exact spelling and case in its own evidence quote.",
        "properties": {
            "source": text(200),
            "target": text(300),
            "evidence": evidence.clone(),
        },
        "required": ["source", "target", "evidence"],
        "additionalProperties": false,
    });
    let term = json!({
        "type": "object",
        "description": "The evidence quote must contain the exact source spelling and case from sourceText, or one supplied form. A lemma absent from the quote requires a matching form.",
        "properties": {
            "source": text(200),
            "target": text(300),
            "forms": {
                "type": "array",
                "maxItems": 16,
                "items": form,
            },
            "evidence": evidence.clone(),
        },
        "required": ["source", "target", "forms", "evidence"],
        "additionalProperties": false,
    });
    let fact = json!({
        "type": "object",
        "properties": {
            "subject": text(200),
            "attribute": text(80),
            "value": text(300),
            "evidence": evidence,
        },
        "required": ["subject", "attribute", "value", "evidence"],
        "additionalProperties": false,
    });

    json!({
        "type": "object",
        "properties": {
            "regions": {
                "type": "array",
                "maxItems": MAX_REGIONS,
                "items": region,
            },
            "terms": {
                "type": "array",
                "maxItems": MAX_CONTEXT_ITEMS,
                "items": term,
            },
            "facts": {
                "type": "array",
                "maxItems": MAX_CONTEXT_ITEMS,
                "items": fact,
            },
        },
        "required": ["regions", "terms", "facts"],
        "additionalProperties": false,
    })
}

pub(super) fn page_selection_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "revision": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RENDERS,
            },
        },
        "required": ["revision"],
        "additionalProperties": false,
    })
}
