use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use url::Url;

use crate::http::validate_https_url;
use crate::{
    Category, ChapterRef, Error, PublicationRef, Result, Source, TextBlock, TextBlockKind,
    TextChapter, TextSpan, VariantSelector, Work,
};

#[derive(Deserialize)]
pub(super) struct Response<T> {
    pub data: T,
}

#[derive(Deserialize)]
pub(super) struct BookRecord {
    id: u64,
    slug_url: String,
    name: String,
    rus_name: String,
    eng_name: String,
    model: String,
    site: u32,
    cover: Option<Cover>,
    #[serde(default, rename = "otherNames")]
    other_names: Vec<String>,
    summary: Option<Content>,
}

#[derive(Deserialize)]
struct Cover {
    default: String,
}

impl BookRecord {
    pub(super) fn into_work(self, needs_inspection: bool) -> Result<Work> {
        let reference = PublicationRef {
            source: Source::RanobeLib,
            id: self.slug_url,
        };
        if self.model != "manga"
            || self.site != 3
            || reference.ranobelib_id()? != self.id
            || self.other_names.len() > 100
            || [&self.name, &self.rus_name, &self.eng_name]
                .into_iter()
                .chain(&self.other_names)
                .any(|title| title.len() > 2048 || title.chars().any(char::is_control))
        {
            return Err(Error::InvalidResponse(
                "invalid RanobeLib identity or title",
            ));
        }
        let title = [&self.rus_name, &self.eng_name, &self.name]
            .into_iter()
            .find(|title| !title.trim().is_empty())
            .ok_or(Error::InvalidResponse("missing RanobeLib title"))?
            .clone();
        let mut aliases = vec![self.name, self.eng_name, self.rus_name];
        aliases.extend(self.other_names);
        aliases.retain(|alias| !alias.trim().is_empty() && *alias != title);
        aliases.sort();
        aliases.dedup();
        let cover = self
            .cover
            .map(|cover| Url::parse(&cover.default).map_err(|_| Error::UnsafeAddress))
            .transpose()?;
        if let Some(cover) = &cover {
            validate_https_url(cover)?;
            if cover.host_str() != Some("cover.cdnlibs.org")
                || cover.query().is_some()
                || cover.port().is_some()
            {
                return Err(Error::UnsafeAddress);
            }
        }
        let description = self
            .summary
            .map(|summary| summary.plain_text())
            .transpose()?
            .filter(|text| !text.trim().is_empty());
        Ok(Work {
            url: Url::parse(&format!("https://ranobelib.me/ru/book/{}", reference.id))
                .expect("validated RanobeLib publication slug"),
            identifiers: BTreeMap::from([("ranobelib".into(), self.id.to_string())]),
            reference,
            title,
            category: Category::Books,
            cover,
            description,
            year: None,
            aliases,
            variants: Vec::new(),
            needs_inspection,
        })
    }
}

#[derive(Deserialize)]
pub(super) struct ChapterRecord {
    pub index: u32,
    pub volume: String,
    pub number: String,
    pub name: Option<String>,
    branches: Vec<Branch>,
    bundle_id: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct Branch {
    pub id: u64,
    pub branch_id: Option<u64>,
    pub teams: Vec<Team>,
    expired_type: u32,
}

#[derive(Deserialize)]
pub(super) struct Team {
    pub name: String,
}

impl ChapterRecord {
    pub(super) fn public_branches(&self) -> impl Iterator<Item = &Branch> {
        self.branches
            .iter()
            .filter(|branch| self.bundle_id.is_none() && branch.expired_type == 0)
    }
}

pub(super) fn validate_listing(mut records: Vec<ChapterRecord>) -> Result<Vec<ChapterRecord>> {
    if records.len() > 10_000 {
        return Err(Error::InvalidResponse("too many RanobeLib chapters"));
    }
    let mut uploads = HashSet::new();
    let mut ordering = HashSet::new();
    for record in &records {
        if record.index == 0
            || !ordering.insert(record.index)
            || record.branches.len() > 100
            || [&record.volume, &record.number].into_iter().any(|number| {
                number.is_empty() || number.len() > 64 || number.chars().any(char::is_control)
            })
            || record
                .name
                .as_ref()
                .is_some_and(|name| name.len() > 2048 || name.chars().any(char::is_control))
        {
            return Err(Error::InvalidResponse("invalid RanobeLib chapter"));
        }
        let mut branches = HashSet::new();
        for branch in &record.branches {
            if branch.id == 0
                || !uploads.insert(branch.id)
                || branch.branch_id == Some(0)
                || !branches.insert(branch.branch_id)
                || branch.teams.len() > 32
                || branch.teams.iter().any(|team| {
                    team.name.trim().is_empty()
                        || team.name.len() > 2048
                        || team.name.chars().any(char::is_control)
                })
            {
                return Err(Error::InvalidResponse("invalid RanobeLib branch"));
            }
        }
    }
    records.sort_by_key(|chapter| chapter.index);
    Ok(records)
}

#[derive(Deserialize)]
pub(super) struct ChapterResponse {
    id: u64,
    manga_id: u64,
    branch_id: Option<u64>,
    model: String,
    volume: String,
    number: String,
    expired_type: u32,
    bundle_id: Option<u64>,
    content: Option<Content>,
}

impl ChapterResponse {
    pub(super) fn into_text(
        self,
        selected: &ChapterRef,
        record: &ChapterRecord,
    ) -> Result<TextChapter> {
        selected.validate()?;
        let VariantSelector::LibTranslation { branch } = selected.variant.selector else {
            return Err(Error::InvalidReference);
        };
        if self.id.to_string() != selected.id
            || self.manga_id != selected.variant.publication.ranobelib_id()?
            || self.branch_id != branch
            || self.volume != record.volume
            || self.number != record.number
            || self.model != "chapter"
            || self.expired_type != 0
            || self.bundle_id.is_some()
            || !record
                .public_branches()
                .any(|upload| upload.id == self.id && upload.branch_id == self.branch_id)
        {
            return Err(Error::VariantUnavailable);
        }
        match self.content.ok_or(Error::VariantUnavailable)? {
            Content::Html(body) => TextChapter::from_html(selected.clone(), &body),
            Content::Document(document) => {
                TextChapter::from_blocks(selected.clone(), document.into_blocks()?)
            }
        }
    }
}

// Both formats are emitted by the current API, even within one publication.
// Structured text maps directly to domain blocks; neither format acquires assets.
#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Html(String),
    Document(Document),
}

impl Content {
    fn plain_text(self) -> Result<String> {
        match self {
            Self::Html(body) => Ok(scraper::Html::parse_fragment(&body)
                .root_element()
                .text()
                .collect()),
            Self::Document(document) => Ok(document
                .into_blocks()?
                .iter()
                .map(TextBlock::plain_text)
                .collect::<Vec<_>>()
                .join("\n")),
        }
    }
}

#[derive(Deserialize)]
struct Document {
    #[serde(rename = "type")]
    kind: String,
    content: Vec<Block>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Block {
    Paragraph {
        #[serde(default)]
        content: Vec<Inline>,
    },
    Heading {
        #[serde(default)]
        content: Vec<Inline>,
    },
    HorizontalRule,
    Image,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Inline {
    Text {
        text: String,
        #[serde(default)]
        marks: Vec<Mark>,
    },
    HardBreak,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Mark {
    Bold,
    Italic,
    Underline,
}

impl Document {
    fn into_blocks(self) -> Result<Vec<TextBlock>> {
        if self.kind != "doc" {
            return Err(Error::InvalidResponse("invalid RanobeLib document"));
        }
        let mut blocks = Vec::new();
        for block in self.content {
            let (kind, content) = match block {
                Block::Paragraph { content } => (TextBlockKind::Paragraph, content),
                Block::Heading { content } => (TextBlockKind::Heading, content),
                Block::HorizontalRule => (TextBlockKind::Separator, Vec::new()),
                Block::Image => continue,
            };
            let spans = content
                .into_iter()
                .map(|inline| match inline {
                    Inline::Text { text, marks } => TextSpan::Text {
                        text,
                        bold: marks.iter().any(|mark| matches!(mark, Mark::Bold)),
                        italic: marks.iter().any(|mark| matches!(mark, Mark::Italic)),
                        underline: marks.iter().any(|mark| matches!(mark, Mark::Underline)),
                    },
                    Inline::HardBreak => TextSpan::Text {
                        text: "\n".into(),
                        bold: false,
                        italic: false,
                        underline: false,
                    },
                })
                .collect::<Vec<_>>();
            if kind == TextBlockKind::Separator
                || spans.iter().any(
                    |span| matches!(span, TextSpan::Text { text, .. } if !text.trim().is_empty()),
                )
            {
                blocks.push(TextBlock {
                    id: String::new(),
                    kind,
                    spans,
                });
            }
        }
        Ok(blocks)
    }
}
