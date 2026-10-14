//! Shared image-page identity, geometry and evidence-bearing context proposals.

use std::sync::{Arc, atomic::AtomicBool};

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
const MAX_TEXT_RUNS: usize = 32;
const MAX_TRANSLATION_BYTES: usize = 3000;

pub(super) const MAX_REGIONS: usize = 60;
const MAX_CLEANUP_RECTS: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

pub(super) fn valid_text_runs(runs: &[TextRun]) -> bool {
    !runs.is_empty()
        && runs.len() <= MAX_TEXT_RUNS
        && runs.iter().all(|run| !run.text.is_empty())
        && runs.iter().any(|run| !run.text.trim().is_empty())
        && runs.iter().map(|run| run.text.len()).sum::<usize>() <= MAX_TRANSLATION_BYTES
}

pub(super) fn text_runs_schema() -> Value {
    json!({
        "type": "array",
        "description": "Contiguous translated text with explicit bold and italic per fragment. The concatenated text must contain at most 3000 UTF-8 bytes.",
        "minItems": 1,
        "maxItems": MAX_TEXT_RUNS,
        "items": {
            "type": "object",
            "properties": {
                "text": { "type": "string", "minLength": 1 },
                "bold": { "type": "boolean" },
                "italic": { "type": "boolean" },
            },
            "required": ["text", "bold", "italic"],
            "additionalProperties": false,
        },
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FontFamily {
    Comic,
    Serif,
    Condensed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageRegion {
    pub id: String,
    pub source_text: String,
    pub translation: Vec<TextRun>,
    pub family: FontFamily,
    pub angle: f64,
    pub text_color: String,
    pub outline_color: Option<String>,
    /// Visible outward outline in original-image pixels. Zero requires no color.
    pub outline_width: f64,
    pub bounds: Rect,
    pub layout: Rect,
    pub cleanup_rects: Vec<Rect>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RenderStatus {
    Rendered,
    EmptyTranslation,
    InvalidGeometry,
    Overflow,
    MissingGlyph,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionDiagnostic {
    pub id: String,
    pub status: RenderStatus,
    pub font_pixels: u32,
    pub layout_lines: u32,
    pub cleanup_pixels: u32,
}

pub struct RenderedPage {
    pub png: Vec<u8>,
    pub diagnostics: Vec<RegionDiagnostic>,
}

pub struct PreparedPage {
    pub diagnostics: Vec<RegionDiagnostic>,
    pub previews: Vec<InputImage>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PageWindow {
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

pub struct RenderedWindow {
    pub window: PageWindow,
    pub png: Vec<u8>,
    pub regions: Vec<PageRegion>,
}

/// Qt validates typography and composes cleaned pixels outside the GUI thread.
pub trait PageRenderer: Send + Sync + 'static {
    fn dimensions(
        &self,
        original: &InputImage,
        cancelled: Arc<AtomicBool>,
    ) -> codex::Result<(u32, u32)>;

    fn crop(
        &self,
        original: &InputImage,
        window: PageWindow,
        cancelled: Arc<AtomicBool>,
    ) -> codex::Result<InputImage>;

    fn compose(
        &self,
        original: &InputImage,
        windows: &[RenderedWindow],
        cancelled: Arc<AtomicBool>,
    ) -> codex::Result<Vec<u8>>;

    fn prepare(
        &self,
        original: &InputImage,
        regions: &[PageRegion],
        preview_pixels: u64,
        cancelled: Arc<AtomicBool>,
    ) -> codex::Result<PreparedPage>;

    fn render(
        &self,
        original: &InputImage,
        cleaned: &[u8],
        regions: &[PageRegion],
        cancelled: Arc<AtomicBool>,
    ) -> codex::Result<RenderedPage>;
}

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

pub fn input_image(bytes: Vec<u8>) -> codex::Result<InputImage> {
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err(CodexError::Image);
    };
    Ok(InputImage { mime, bytes })
}

pub(super) fn validate_regions(regions: &[PageRegion]) -> codex::Result<()> {
    if regions.is_empty()
        || regions.len() > MAX_REGIONS
        || regions
            .iter()
            .map(|region| region.cleanup_rects.len())
            .sum::<usize>()
            > MAX_CLEANUP_RECTS
    {
        return Err(CodexError::Tool(
            "Provide one to 60 regions with at most 4096 cleanup rectangles".into(),
        ));
    }
    for (index, region) in regions.iter().enumerate() {
        let reason = if region.id.trim().is_empty()
            || region.id.len() > 64
            || region.id.chars().any(char::is_control)
            || regions[..index]
                .iter()
                .any(|previous| previous.id == region.id)
        {
            "id must be nonempty and unique on this page"
        } else if region.source_text.trim().is_empty() || region.source_text.len() > 2000 {
            "sourceText must contain the visible original text"
        } else if !valid_text_runs(&region.translation) {
            "translation must contain one to 32 styled text runs, at most 3000 bytes in total"
        } else if !region.angle.is_finite() || !(-45.0..=45.0).contains(&region.angle) {
            "angle must be between -45 and 45 clockwise degrees"
        } else if !valid_color(&region.text_color) {
            "textColor must be #RRGGBB"
        } else if !region.outline_width.is_finite() || !(0.0..=16.0).contains(&region.outline_width)
        {
            "outlineWidth must be between zero and 16 original-image pixels"
        } else if (region.outline_width > 0.0) != region.outline_color.is_some()
            || region
                .outline_color
                .as_ref()
                .is_some_and(|color| !valid_color(color))
        {
            "outlineColor must be #RRGGBB for a positive outlineWidth, or null for zero width"
        } else if !region.bounds.valid() {
            "bounds must be a positive rectangle inside the normalized page"
        } else if !region.layout.valid() || !region.bounds.contains(&region.layout) {
            "layout must be a positive rectangle entirely inside bounds"
        } else if region.cleanup_rects.is_empty() || region.cleanup_rects.len() > 512 {
            "cleanupRects must contain one to 512 source-lettering rectangles"
        } else if region
            .cleanup_rects
            .iter()
            .any(|rect| !rect.valid() || !region.bounds.contains(rect))
        {
            "each cleanup rectangle must be a positive rectangle entirely inside bounds"
        } else {
            continue;
        };
        return Err(CodexError::Tool(format!("Region {}: {reason}", index + 1)));
    }
    Ok(())
}

fn valid_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

pub(super) fn validate_rendered_regions(
    regions: &[PageRegion],
    diagnostics: &[RegionDiagnostic],
) -> codex::Result<()> {
    if diagnostics.len() != regions.len() {
        return Err(CodexError::Tool(
            "Manga render region count does not match".into(),
        ));
    }
    let failures: Vec<_> = regions
        .iter()
        .zip(diagnostics)
        .filter(|(region, result)| {
            region.id != result.id
                || result.status != RenderStatus::Rendered
                || result.font_pixels < 9
                || result.layout_lines == 0
                || result.cleanup_pixels == 0
        })
        .take(5)
        .collect();
    if failures.is_empty() {
        return Ok(());
    }
    let details: Vec<_> = failures
        .into_iter()
        .map(|(region, result)| {
            json!({
                "id": region.id,
                "status": result.status,
                "fontPixels": result.font_pixels,
                "layoutLines": result.layout_lines,
            })
        })
        .collect();
    Err(CodexError::Tool(format!(
        "Manga typography has incomplete regions: {}",
        json!(details)
    )))
}

pub(super) fn region_schema() -> Value {
    let rect = rect_schema();
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string", "minLength": 1, "maxLength": 64},
            "sourceText": {"type": "string", "minLength": 1, "maxLength": 2000},
            "translation": text_runs_schema(),
            "family": {"type": "string", "enum": ["comic", "serif", "condensed"]},
            "angle": {"type": "number", "minimum": -45, "maximum": 45},
            "textColor": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"},
            "outlineColor": {"type": ["string", "null"], "pattern": "^#[0-9A-Fa-f]{6}$"},
            "outlineWidth": {"type": "number", "minimum": 0, "maximum": 16},
            "bounds": rect,
            "layout": rect,
            "cleanupRects": {"type": "array", "minItems": 1, "maxItems": 512, "items": rect},
        },
        "required": ["id", "sourceText", "translation", "family", "angle", "textColor", "outlineColor", "outlineWidth", "bounds", "layout", "cleanupRects"],
        "additionalProperties": false,
    })
}
