//! Accepted translation conventions for one exact publication and language.
//! Image observations identify model-read text on immutable page bytes; they
//! are not independently verified OCR.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{ChapterRef, PublicationRef};

const MAX_TERMS: usize = 2_000;
const MAX_FACTS: usize = 2_000;
const MAX_PROMPT_CONTEXT_BYTES: usize = 12_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextEvidence {
    pub chapter: ChapterRef,
    pub segment_id: String,
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_observation: Option<ImageObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageObservation {
    pub page: u32,
    pub original_digest: String,
    /// Left, top, right and bottom in millionths of the original page.
    pub bounds: [u32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationForm {
    pub source: String,
    pub target: String,
    pub evidence: ContextEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationTerm {
    pub source: String,
    pub target: String,
    pub forms: Vec<TranslationForm>,
    pub evidence: ContextEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedFact {
    pub subject: String,
    pub attribute: String,
    pub value: String,
    pub evidence: ContextEvidence,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationContext {
    pub revision: u64,
    pub terms: Vec<TranslationTerm>,
    pub facts: Vec<ConfirmedFact>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextAdditions {
    pub terms: Vec<TranslationTerm>,
    pub facts: Vec<ConfirmedFact>,
}

#[derive(Clone, Debug)]
pub struct EvidenceSegment {
    pub chapter: ChapterRef,
    pub id: String,
    pub text: String,
    pub image_observation: Option<ImageObservation>,
}

impl TranslationContext {
    pub fn validate(&self, publication: &PublicationRef) -> Result<(), &'static str> {
        if self.terms.len() > MAX_TERMS || self.facts.len() > MAX_FACTS {
            return Err("translation context exceeds its limit");
        }
        let mut terms = HashSet::new();
        for term in &self.terms {
            validate_term(term, publication)?;
            if !terms.insert(&term.source) {
                return Err("translation context contains duplicate terms");
            }
        }
        let mut facts = HashSet::new();
        for fact in &self.facts {
            validate_fact(fact, publication)?;
            if !facts.insert((&fact.subject, &fact.attribute)) {
                return Err("translation context contains duplicate facts");
            }
        }
        Ok(())
    }

    /// A conflicting spelling or confirmed fact is kept out of the persistent
    /// context. Resolving it requires new evidence and a separate decision.
    pub fn apply(
        &mut self,
        publication: &PublicationRef,
        additions: ContextAdditions,
        segments: &[EvidenceSegment],
    ) -> Result<bool, &'static str> {
        self.validate(publication)?;
        let evidence: HashMap<_, _> = segments
            .iter()
            .filter(|segment| segment.chapter.variant.publication == *publication)
            .map(|segment| ((&segment.chapter, segment.id.as_str()), segment))
            .collect();

        let mut next = self.clone();
        for term in additions.terms {
            validate_term(&term, publication)?;
            validate_evidence(&term.evidence, &evidence)?;
            if !term.evidence.quote.contains(&term.source)
                && !term
                    .forms
                    .iter()
                    .any(|form| term.evidence.quote.contains(&form.source))
            {
                return Err("translation term is absent from its source evidence");
            }
            for form in &term.forms {
                validate_evidence(&form.evidence, &evidence)?;
                if !form.evidence.quote.contains(&form.source) {
                    return Err("translation form is absent from its source evidence");
                }
            }
            if let Some(existing) = next
                .terms
                .iter_mut()
                .find(|entry| entry.source == term.source)
            {
                if existing.target != term.target {
                    return Err("translation term conflicts with confirmed context");
                }
                for form in term.forms {
                    if let Some(saved) = existing
                        .forms
                        .iter()
                        .find(|saved| saved.source == form.source)
                    {
                        if saved.target != form.target {
                            return Err("translation form conflicts with confirmed context");
                        }
                    } else {
                        existing.forms.push(form);
                    }
                }
                continue;
            }
            if next.terms.len() >= MAX_TERMS {
                return Err("translation context exceeds its limit");
            }
            next.terms.push(term);
        }
        for fact in additions.facts {
            validate_fact(&fact, publication)?;
            validate_evidence(&fact.evidence, &evidence)?;
            if !fact.evidence.quote.contains(&fact.subject)
                && !next.terms.iter().any(|term| {
                    term.target == fact.subject
                        && (fact.evidence.quote.contains(&term.source)
                            || term
                                .forms
                                .iter()
                                .any(|form| fact.evidence.quote.contains(&form.source)))
                })
            {
                return Err("translation fact has no subject in its source evidence");
            }
            if let Some(existing) = next
                .facts
                .iter()
                .find(|entry| entry.subject == fact.subject && entry.attribute == fact.attribute)
            {
                if existing.value != fact.value {
                    return Err("translation fact conflicts with confirmed context");
                }
                continue;
            }
            if next.facts.len() >= MAX_FACTS {
                return Err("translation context exceeds its limit");
            }
            next.facts.push(fact);
        }
        if next == *self {
            return Ok(false);
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("translation context revision overflow")?;
        *self = next;
        Ok(true)
    }

    pub fn relevant(&self, source: &str, limit: usize) -> Self {
        let mentioned_targets: HashSet<_> = self
            .terms
            .iter()
            .filter(|term| {
                source.contains(&term.source)
                    || term.forms.iter().any(|form| source.contains(&form.source))
            })
            .map(|term| term.target.as_str())
            .collect();
        self.select(
            limit,
            |term| {
                source.contains(&term.source)
                    || term.forms.iter().any(|form| source.contains(&form.source))
            },
            |fact| {
                source.contains(&fact.subject) || mentioned_targets.contains(fact.subject.as_str())
            },
        )
    }

    pub fn search(&self, query: &str, limit: usize) -> Self {
        self.select(
            limit,
            |term| {
                term.source.contains(query)
                    || term.target.contains(query)
                    || term
                        .forms
                        .iter()
                        .any(|form| form.source.contains(query) || form.target.contains(query))
            },
            |fact| {
                fact.subject.contains(query)
                    || fact.attribute.contains(query)
                    || fact.value.contains(query)
            },
        )
    }

    fn select(
        &self,
        limit: usize,
        term_matches: impl Fn(&TranslationTerm) -> bool,
        fact_matches: impl Fn(&ConfirmedFact) -> bool,
    ) -> Self {
        let mut selected = Self {
            revision: self.revision,
            ..Self::default()
        };
        let mut bytes = 0;
        for term in self
            .terms
            .iter()
            .filter(|term| term_matches(term))
            .take(limit)
        {
            let size = serde_json::to_vec(term)
                .expect("validated translation term serializes")
                .len();
            if bytes + size > MAX_PROMPT_CONTEXT_BYTES {
                break;
            }
            selected.terms.push(term.clone());
            bytes += size;
        }
        for fact in self
            .facts
            .iter()
            .filter(|fact| fact_matches(fact))
            .take(limit)
        {
            let size = serde_json::to_vec(fact)
                .expect("validated translation fact serializes")
                .len();
            if bytes + size > MAX_PROMPT_CONTEXT_BYTES {
                break;
            }
            selected.facts.push(fact.clone());
            bytes += size;
        }
        selected
    }
}

fn validate_term(term: &TranslationTerm, publication: &PublicationRef) -> Result<(), &'static str> {
    validate_text(&term.source, 200)?;
    validate_text(&term.target, 300)?;
    if term.forms.len() > 16 {
        return Err("translation term has too many forms");
    }
    let mut forms = HashSet::new();
    for form in &term.forms {
        validate_text(&form.source, 200)?;
        validate_text(&form.target, 300)?;
        validate_reference(&form.evidence, publication)?;
        if !forms.insert(&form.source) {
            return Err("translation term has duplicate forms");
        }
    }
    validate_reference(&term.evidence, publication)
}

fn validate_fact(fact: &ConfirmedFact, publication: &PublicationRef) -> Result<(), &'static str> {
    validate_text(&fact.subject, 200)?;
    validate_text(&fact.attribute, 80)?;
    validate_text(&fact.value, 300)?;
    validate_reference(&fact.evidence, publication)
}

fn validate_reference(
    evidence: &ContextEvidence,
    publication: &PublicationRef,
) -> Result<(), &'static str> {
    evidence
        .chapter
        .validate()
        .map_err(|_| "invalid evidence chapter")?;
    if evidence.chapter.variant.publication != *publication {
        return Err("evidence belongs to another publication");
    }
    validate_text(&evidence.segment_id, 128)?;
    validate_text(&evidence.quote, 500)?;
    if let Some(image) = &evidence.image_observation {
        image.validate()?;
    }
    Ok(())
}

fn validate_evidence(
    evidence: &ContextEvidence,
    segments: &HashMap<(&ChapterRef, &str), &EvidenceSegment>,
) -> Result<(), &'static str> {
    let segment = segments
        .get(&(&evidence.chapter, evidence.segment_id.as_str()))
        .ok_or("evidence segment was not supplied to the job")?;
    if segment.image_observation != evidence.image_observation {
        return Err("evidence image does not match the supplied page region");
    }
    if !segment.text.contains(&evidence.quote) {
        return Err("evidence quote is absent from its source segment");
    }
    Ok(())
}

impl ImageObservation {
    fn validate(&self) -> Result<(), &'static str> {
        if self.original_digest.len() != 64
            || !self
                .original_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.bounds[0] >= self.bounds[2]
            || self.bounds[1] >= self.bounds[3]
            || self.bounds[2] > 1_000_000
            || self.bounds[3] > 1_000_000
        {
            return Err("image observation is invalid");
        }
        Ok(())
    }
}

fn validate_text(value: &str, limit: usize) -> Result<(), &'static str> {
    if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err("translation context text is invalid");
    }
    Ok(())
}
