//! Narrow, read-only chapter inspection for an isolated translation job.
//! The caller supplies an exact publication roster and one bounded source.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;

use super::codex::{
    self, CodexError, FunctionTool, InputImage, ToolCall, ToolFuture, ToolHandler, ToolOutput,
};
use super::context::{EvidenceSegment, TranslationContext};
use crate::{Chapter, ChapterRef};

const MAX_TOOL_CALLS: usize = 8;
const MAX_TOOL_TEXT: usize = 12_000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

pub struct ChapterExcerpt {
    pub chapter: ChapterRef,
    pub offset: usize,
    pub total_parts: usize,
    pub parts: Vec<EvidenceSegment>,
    pub image: Option<InputImage>,
}

pub trait WorkChapterSource: Send + Sync {
    fn load(
        &self,
        chapter: ChapterRef,
        offset: usize,
    ) -> Pin<Box<dyn Future<Output = codex::Result<ChapterExcerpt>> + Send + '_>>;
}

pub fn tool_definitions() -> Vec<FunctionTool> {
    vec![
        FunctionTool {
            name: "list_chapters",
            description: "List chapter titles and IDs from this exact publication, 30 at a time.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "offset": { "type": "integer", "minimum": 0 },
                    "count": { "type": "integer", "minimum": 1, "maximum": 30 }
                },
                "required": ["offset", "count"],
                "additionalProperties": false
            }),
        },
        FunctionTool {
            name: "get_chapter",
            description: "Read one bounded excerpt or image of an allowed chapter in this publication.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "chapter_id": { "type": "string" },
                    "offset": { "type": "integer", "minimum": 0 }
                },
                "required": ["chapter_id", "offset"],
                "additionalProperties": false
            }),
        },
        FunctionTool {
            name: "search_work_text",
            description: "Search only the selected excerpts from up to four named chapters in this publication.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "minLength": 1, "maxLength": 200 },
                    "chapter_ids": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 4,
                        "items": { "type": "string" }
                    },
                    "offset": { "type": "integer", "minimum": 0 }
                },
                "required": ["query", "chapter_ids", "offset"],
                "additionalProperties": false
            }),
        },
        FunctionTool {
            name: "read_context",
            description: "Find confirmed names, forms and facts for a source term or character.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "minLength": 1, "maxLength": 200 }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
    ]
}

pub struct WorkContextTools<'a, S: ?Sized> {
    chapters: HashMap<String, ChapterRef>,
    ordered: &'a [Chapter],
    source: &'a S,
    context: TranslationContext,
    cancelled: watch::Receiver<bool>,
    evidence: Vec<EvidenceSegment>,
    calls: usize,
}

impl<'a, S: WorkChapterSource + ?Sized> WorkContextTools<'a, S> {
    pub fn new(
        selected: &ChapterRef,
        chapters: &'a [Chapter],
        source: &'a S,
        context: TranslationContext,
        cancelled: watch::Receiver<bool>,
    ) -> codex::Result<Self> {
        selected.validate().map_err(|_| CodexError::Protocol)?;
        let mut allowed = HashMap::new();
        for chapter in chapters {
            chapter
                .reference
                .validate()
                .map_err(|_| CodexError::Protocol)?;
            if chapter.reference.variant != selected.variant
                || allowed
                    .insert(chapter.reference.id.clone(), chapter.reference.clone())
                    .is_some()
            {
                return Err(CodexError::Protocol);
            }
        }
        if !allowed.contains_key(&selected.id) {
            return Err(CodexError::Protocol);
        }
        Ok(Self {
            chapters: allowed,
            ordered: chapters,
            source,
            context,
            cancelled,
            evidence: Vec::new(),
            calls: 0,
        })
    }

    pub fn chapter_map(&self) -> &HashMap<String, ChapterRef> {
        &self.chapters
    }

    pub fn set_context(&mut self, context: TranslationContext) {
        self.context = context;
    }

    pub fn add_evidence(&mut self, segment: EvidenceSegment) {
        self.evidence.push(segment);
    }

    pub fn evidence(&self) -> &[EvidenceSegment] {
        &self.evidence
    }

    pub fn into_evidence(self) -> Vec<EvidenceSegment> {
        self.evidence
    }

    async fn load(&self, id: &str, offset: usize) -> codex::Result<ChapterExcerpt> {
        let chapter = self.chapters.get(id).ok_or(CodexError::Protocol)?.clone();
        let mut cancelled = self.cancelled.clone();
        let excerpt = tokio::select! {
            result = tokio::time::timeout(FETCH_TIMEOUT, self.source.load(chapter.clone(), offset)) => {
                result.map_err(|_| CodexError::Timeout)??
            }
            _ = cancelled.changed() => return Err(CodexError::Cancelled),
        };
        if excerpt.chapter != chapter
            || excerpt.offset != offset
            || offset >= excerpt.total_parts
            || excerpt.parts.len() > 40
            || excerpt.parts.iter().any(|part| {
                part.chapter != chapter
                    || part.id.is_empty()
                    || part.id.len() > 128
                    || part.id.chars().any(char::is_control)
            })
            || excerpt
                .parts
                .iter()
                .map(|part| part.text.len())
                .sum::<usize>()
                > MAX_TOOL_TEXT
        {
            return Err(CodexError::Protocol);
        }
        Ok(excerpt)
    }

    async fn answer(&mut self, call: ToolCall) -> codex::Result<ToolOutput> {
        self.calls += 1;
        if self.calls > MAX_TOOL_CALLS || *self.cancelled.borrow() {
            return Err(CodexError::Cancelled);
        }
        let (value, image) = match call.name.as_str() {
            "list_chapters" => {
                let offset = argument_usize(&call.arguments, "offset")?;
                let count = argument_usize(&call.arguments, "count")?;
                if !(1..=30).contains(&count) {
                    return Err(CodexError::Protocol);
                }
                let chapters: Vec<_> = self
                    .ordered
                    .iter()
                    .skip(offset)
                    .take(count)
                    .map(|chapter| {
                        let title = chapter
                            .title
                            .as_deref()
                            .map(|value| value.chars().take(300).collect::<String>());
                        let number = chapter
                            .number
                            .as_deref()
                            .map(|value| value.chars().take(40).collect::<String>());
                        json!({"id":chapter.reference.id,"title":title,"number":number})
                    })
                    .collect();
                (
                    json!({"total":self.ordered.len(),"chapters":chapters}),
                    None,
                )
            }
            "get_chapter" => {
                let id = argument_str(&call.arguments, "chapter_id", 128)?;
                let offset = argument_usize(&call.arguments, "offset")?;
                let excerpt = self.load(id, offset).await?;
                let parts: Vec<_> = excerpt
                    .parts
                    .iter()
                    .map(|part| json!({"id":part.id,"text":part.text}))
                    .collect();
                self.evidence.extend(excerpt.parts);
                (
                    json!({"chapter_id":id,"offset":offset,
                    "total_parts":excerpt.total_parts,"parts":parts}),
                    excerpt.image,
                )
            }
            "search_work_text" => {
                let query = argument_str(&call.arguments, "query", 200)?;
                let offset = argument_usize(&call.arguments, "offset")?;
                let ids = call
                    .arguments
                    .get("chapter_ids")
                    .and_then(Value::as_array)
                    .filter(|ids| !ids.is_empty() && ids.len() <= 4)
                    .ok_or(CodexError::Protocol)?;
                let mut unique = HashSet::new();
                let mut matches = Vec::new();
                for value in ids {
                    let id = value
                        .as_str()
                        .filter(|id| unique.insert(*id))
                        .ok_or(CodexError::Protocol)?;
                    let excerpt = self.load(id, offset).await?;
                    for part in excerpt.parts {
                        if let Some(position) = part.text.find(query) {
                            let snippet = excerpt_window(&part.text, position, query.len());
                            matches.push(
                                json!({"chapter_id":id,"segment_id":part.id,"snippet":snippet}),
                            );
                            self.evidence.push(part);
                            if matches.len() >= 12 {
                                break;
                            }
                        }
                    }
                    if matches.len() >= 12 {
                        break;
                    }
                }
                (
                    json!({"query":query,"offset":offset,"matches":matches}),
                    None,
                )
            }
            "read_context" => {
                let query = argument_str(&call.arguments, "query", 200)?;
                (json!(self.context.search(query, 30)), None)
            }
            _ => return Err(CodexError::Protocol),
        };
        Ok(ToolOutput {
            text: value.to_string(),
            image,
        })
    }
}

impl<S: WorkChapterSource + ?Sized> ToolHandler for WorkContextTools<'_, S> {
    fn call(&mut self, call: ToolCall) -> ToolFuture<'_> {
        Box::pin(self.answer(call))
    }
}

fn argument_usize(arguments: &Value, name: &str) -> codex::Result<usize> {
    arguments
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|number| usize::try_from(number).ok())
        .ok_or(CodexError::Protocol)
}

fn argument_str<'a>(arguments: &'a Value, name: &str, limit: usize) -> codex::Result<&'a str> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= limit)
        .ok_or(CodexError::Protocol)
}

fn excerpt_window(text: &str, position: usize, length: usize) -> &str {
    let mut start = position.saturating_sub(200);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (position + length + 300).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[start..end]
}
