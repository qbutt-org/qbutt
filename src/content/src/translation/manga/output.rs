//! The final page decision and its evidence-bearing additions to work context.

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ChapterRef;

use super::super::context::{
    ConfirmedFact, ContextAdditions, ContextEvidence, EvidenceSegment, ImageObservation,
    TranslationForm, TranslationTerm,
};
use super::{PageRegion, Rect};
use qbutt_codex::{self as codex, CodexError, InputImage};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PageOutput {
    pub(super) revision: usize,
    terms: Vec<OutputTerm>,
    facts: Vec<OutputFact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputEvidence {
    region_id: String,
    quote: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputTerm {
    source: String,
    target: String,
    forms: Vec<OutputForm>,
    evidence: OutputEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputForm {
    source: String,
    target: String,
    evidence: OutputEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputFact {
    subject: String,
    attribute: String,
    value: String,
    evidence: OutputEvidence,
}

impl PageOutput {
    pub(super) fn validate_selection(&self, render_count: usize) -> codex::Result<()> {
        if self.revision == 0
            || self.revision > render_count
            || self.terms.len() > 20
            || self.facts.len() > 20
        {
            return Err(CodexError::Tool(
                "Codex selected an invalid page revision or too much context".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn decode_context(
        self,
        chapter: &ChapterRef,
        page: u32,
        original: &InputImage,
        regions: &[PageRegion],
    ) -> codex::Result<(ContextAdditions, Vec<EvidenceSegment>)> {
        let digest = format!("{:x}", Sha256::digest(&original.bytes));
        let segments: Vec<_> = regions
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

        let evidence = |raw: OutputEvidence| -> codex::Result<ContextEvidence> {
            let segment = segments
                .iter()
                .find(|segment| segment.id == raw.region_id)
                .ok_or(CodexError::Protocol)?;
            Ok(ContextEvidence {
                chapter: segment.chapter.clone(),
                segment_id: segment.id.clone(),
                quote: raw.quote,
                image_observation: segment.image_observation.clone(),
            })
        };

        let terms = self
            .terms
            .into_iter()
            .map(|term| -> codex::Result<TranslationTerm> {
                let forms = term
                    .forms
                    .into_iter()
                    .map(|form| {
                        Ok(TranslationForm {
                            source: form.source,
                            target: form.target,
                            evidence: evidence(form.evidence)?,
                        })
                    })
                    .collect::<codex::Result<_>>()?;

                Ok(TranslationTerm {
                    source: term.source,
                    target: term.target,
                    forms,
                    evidence: evidence(term.evidence)?,
                })
            })
            .collect::<codex::Result<_>>()?;
        let facts = self
            .facts
            .into_iter()
            .map(|fact| {
                Ok(ConfirmedFact {
                    subject: fact.subject,
                    attribute: fact.attribute,
                    value: fact.value,
                    evidence: evidence(fact.evidence)?,
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

pub(super) fn page_output_schema() -> Value {
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
            "revision": {
                "type": "integer",
                "minimum": 1,
                "maximum": 3,
            },
            "terms": {
                "type": "array",
                "maxItems": 20,
                "items": term,
            },
            "facts": {
                "type": "array",
                "maxItems": 20,
                "items": fact,
            },
        },
        "required": ["revision", "terms", "facts"],
        "additionalProperties": false,
    })
}
