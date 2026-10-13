//! Shared image-page identity, geometry and evidence-bearing context proposals.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ChapterRef;
use qbutt_codex::{self as codex, CodexError, InputImage};

use super::context::{
    ConfirmedFact, ContextAdditions, ContextEvidence, EvidenceSegment, ImageObservation,
    TranslationContext, TranslationForm, TranslationTerm,
};

const MAX_CONTEXT_ITEMS: usize = 20;

pub struct PageJob {
    pub chapter: ChapterRef,
    pub page: u32,
    pub original: InputImage,
    pub target_language: String,
    /// Accepted same-work conventions with source provenance; this is untrusted input.
    pub work_context: TranslationContext,
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl PageJob {
    pub(super) fn validate(&self) -> codex::Result<()> {
        if self.target_language.trim().is_empty() || self.target_language.len() > 32 {
            return Err(CodexError::Protocol);
        }
        self.chapter.validate().map_err(|_| CodexError::Protocol)?;
        self.work_context
            .validate(&self.chapter.variant.publication)
            .map_err(|_| CodexError::Protocol)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub(super) fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
            && self.x + self.width <= 1.0 + 1e-6
            && self.y + self.height <= 1.0 + 1e-6
    }

    pub(super) fn contains(&self, other: &Self) -> bool {
        self.x - 1e-6 <= other.x
            && self.y - 1e-6 <= other.y
            && self.x + self.width + 1e-6 >= other.x + other.width
            && self.y + self.height + 1e-6 >= other.y + other.height
    }

    fn observation_bounds(&self) -> [u32; 4] {
        let left = ((self.x * 1_000_000.0).round() as u32).min(999_999);
        let top = ((self.y * 1_000_000.0).round() as u32).min(999_999);
        let right = (((self.x + self.width) * 1_000_000.0).round() as u32)
            .max(left + 1)
            .min(1_000_000);
        let bottom = (((self.y + self.height) * 1_000_000.0).round() as u32)
            .max(top + 1)
            .min(1_000_000);
        [left, top, right, bottom]
    }
}

pub(super) struct SourceObservation<'a> {
    pub id: &'a str,
    pub text: &'a str,
    pub bounds: &'a Rect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateEvidence {
    region_id: String,
    quote: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateTerm {
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
pub(super) struct CandidateFact {
    subject: String,
    attribute: String,
    value: String,
    evidence: CandidateEvidence,
}

pub(super) fn decode_context<'a>(
    job: &PageJob,
    observations: impl IntoIterator<Item = SourceObservation<'a>>,
    terms: &[CandidateTerm],
    facts: &[CandidateFact],
) -> codex::Result<(ContextAdditions, Vec<EvidenceSegment>)> {
    if terms.len() > MAX_CONTEXT_ITEMS || facts.len() > MAX_CONTEXT_ITEMS {
        return Err(CodexError::Tool(
            "Provide at most 20 context terms and 20 facts".into(),
        ));
    }
    let digest = format!("{:x}", Sha256::digest(&job.original.bytes));
    let segments: Vec<_> = observations
        .into_iter()
        .map(|observation| EvidenceSegment {
            chapter: job.chapter.clone(),
            id: observation.id.to_owned(),
            text: observation.text.to_owned(),
            image_observation: Some(ImageObservation {
                page: job.page,
                original_digest: digest.clone(),
                bounds: observation.bounds.observation_bounds(),
            }),
        })
        .collect();

    let evidence = |raw: &CandidateEvidence| -> codex::Result<ContextEvidence> {
        let segment = segments
            .iter()
            .find(|segment| segment.id == raw.region_id)
            .ok_or_else(|| {
                CodexError::Tool("Context evidence must cite a region in this page proposal".into())
            })?;
        Ok(ContextEvidence {
            chapter: segment.chapter.clone(),
            segment_id: segment.id.clone(),
            quote: raw.quote.clone(),
            image_observation: segment.image_observation.clone(),
        })
    };
    let terms = terms
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
    let facts = facts
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

pub(super) fn rect_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "x": { "type": "number", "minimum": 0, "maximum": 1 },
            "y": { "type": "number", "minimum": 0, "maximum": 1 },
            "width": { "type": "number", "exclusiveMinimum": 0, "maximum": 1 },
            "height": { "type": "number", "exclusiveMinimum": 0, "maximum": 1 },
        },
        "required": ["x", "y", "width", "height"],
        "additionalProperties": false,
    })
}

pub(super) fn context_properties() -> Value {
    let text = |limit| json!({ "type": "string", "minLength": 1, "maxLength": limit });
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
        "terms": { "type": "array", "maxItems": MAX_CONTEXT_ITEMS, "items": term },
        "facts": { "type": "array", "maxItems": MAX_CONTEXT_ITEMS, "items": fact },
    })
}
