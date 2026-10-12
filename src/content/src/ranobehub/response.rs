use std::collections::{BTreeMap, HashSet};

use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::Number;
use url::Url;

use crate::{Category, Error, PublicationRef, Result, Source, VariantSelector, Work};

#[derive(Deserialize)]
pub(super) struct SearchResponse {
    pub books: Vec<Book>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Book {
    id: u64,
    slug: String,
    title: String,
    original_title: Option<String>,
    description: Option<String>,
    poster_url: Option<String>,
    year: Option<u16>,
}

impl Book {
    pub(super) fn into_work(self, origin: &Url) -> Result<Work> {
        if self.id == 0
            || self.slug.is_empty()
            || self.slug.len() > 200
            || !self
                .slug
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(Error::InvalidResponse("invalid RanobeHub book identity"));
        }
        validate_title(&self.title)?;
        let reference = PublicationRef {
            source: Source::RanobeHub,
            id: self.id.to_string(),
        };
        let url = origin
            .join(&format!("/ranobe/{}-{}", self.id, self.slug))
            .map_err(|_| Error::InvalidResponse("invalid RanobeHub book URL"))?;
        let aliases = self
            .original_title
            .filter(|title| !title.trim().is_empty() && title != &self.title)
            .into_iter()
            .collect();
        Ok(Work {
            reference,
            title: self.title,
            category: Category::Books,
            url,
            cover: self
                .poster_url
                .as_deref()
                .map(|poster| cover_url(origin, poster))
                .transpose()?,
            description: self.description,
            year: self.year,
            aliases,
            identifiers: BTreeMap::new(),
            variants: Vec::new(),
            needs_inspection: true,
        })
    }
}

pub(super) fn work_page(reference: &PublicationRef, origin: &Url, page: &str) -> Result<Work> {
    let document = Html::parse_document(page);
    let canonical = document
        .select(&Selector::parse("link[rel=canonical]").expect("fixed canonical selector"))
        .next()
        .and_then(|element| element.attr("href"))
        .ok_or(Error::InvalidResponse("missing RanobeHub canonical URL"))?;
    let url = Url::parse(canonical)
        .map_err(|_| Error::InvalidResponse("invalid RanobeHub canonical URL"))?;
    crate::http::validate_https_url(&url)?;
    if url.origin() != origin.origin()
        || url.query().is_some()
        || url.fragment().is_some()
        || url
            .path()
            .strip_prefix("/ranobe/")
            .and_then(|path| path.split_once('-'))
            .is_none_or(|(id, slug)| id != reference.id || slug.is_empty() || slug.contains('/'))
    {
        return Err(Error::InvalidResponse(
            "RanobeHub publication identity changed",
        ));
    }
    let title = text(&document, ".book-copy h1")
        .ok_or(Error::InvalidResponse("missing RanobeHub title"))?;
    validate_title(&title)?;
    let alias = text(&document, ".book-original-title").filter(|alias| alias != &title);
    let cover = document
        .select(&Selector::parse("img.book-cover-image").expect("fixed cover selector"))
        .next()
        .and_then(|element| element.attr("src"))
        .map(|src| {
            let image = origin
                .join(src)
                .map_err(|_| Error::InvalidResponse("invalid RanobeHub poster URL"))?;
            let poster = image
                .query_pairs()
                .find(|(name, _)| name == "url")
                .ok_or(Error::InvalidResponse("missing RanobeHub poster URL"))?
                .1
                .into_owned();
            cover_url(origin, &poster)
        })
        .transpose()?;
    let year = document
        .select(&Selector::parse(".book-kicker-year").expect("fixed year selector"))
        .next()
        .and_then(|element| element.attr("href"))
        .and_then(|href| href.strip_prefix("/year/"))
        .and_then(|year| year.parse().ok());
    Ok(Work {
        reference: reference.clone(),
        title,
        category: Category::Books,
        url,
        cover,
        description: text(&document, ".book-description-copy"),
        year,
        aliases: alias.into_iter().collect(),
        identifiers: BTreeMap::new(),
        variants: Vec::new(),
        needs_inspection: false,
    })
}

fn text(document: &Html, selector: &str) -> Option<String> {
    document
        .select(&Selector::parse(selector).expect("fixed RanobeHub metadata selector"))
        .next()
        .map(|element| {
            element
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_owned()
        })
        .filter(|text| !text.is_empty())
}

fn cover_url(origin: &Url, poster: &str) -> Result<Url> {
    let mut url = origin
        .join(poster)
        .map_err(|_| Error::InvalidResponse("invalid RanobeHub poster URL"))?;
    crate::http::validate_https_url(&url)?;
    if url.origin() != origin.origin()
        || url.fragment().is_some()
        || !url
            .path()
            .strip_prefix("/api/media/")
            .is_some_and(crate::model::canonical_decimal)
    {
        return Err(Error::InvalidResponse("invalid RanobeHub poster URL"));
    }
    // The query selects resized renditions. The query-free public media endpoint
    // is the original poster and remains suitable for persistent library records.
    url.set_query(None);
    Ok(url)
}

fn validate_title(title: &str) -> Result<()> {
    if title.trim().is_empty() || title.len() > 2048 || title.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid RanobeHub title"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChapterListing {
    pub items: Vec<ChapterRecord>,
    pub total: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChapterRecord {
    pub id: u64,
    book_id: u64,
    pub title: String,
    pub volume: Number,
    pub number: Number,
    is_draft: bool,
    published_at: Option<String>,
    #[serde(default)]
    paid: bool,
    pub translation_options: Vec<Translation>,
}

impl ChapterRecord {
    pub(super) fn validate(&self, book: u64) -> Result<()> {
        if self.id == 0 || self.book_id != book {
            return Err(Error::InvalidResponse(
                "RanobeHub chapter belongs to another publication",
            ));
        }
        validate_title(&self.title)?;
        let mut seen = HashSet::new();
        for branch in self
            .translation_options
            .iter()
            .filter(|branch| branch.is_human())
        {
            branch.validate()?;
            if !seen.insert(&branch.slug) {
                return Err(Error::InvalidResponse(
                    "duplicate RanobeHub translation branch",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn is_public(&self) -> bool {
        !self.is_draft
            && !self.paid
            && self
                .published_at
                .as_ref()
                .is_some_and(|date| !date.is_empty())
    }

    pub(super) fn translation(&self, slug: &str) -> Option<&Translation> {
        if !self.is_public() {
            return None;
        }
        self.translation_options
            .iter()
            .find(|branch| branch.slug == slug && branch.is_human())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Translation {
    pub slug: String,
    pub name: String,
    pub chapter_id: Option<u64>,
    pub member_details: Vec<Member>,
}

impl Translation {
    pub(super) fn is_human(&self) -> bool {
        !self.member_details.is_empty()
            && self
                .member_details
                .iter()
                .all(|member| member.is_ai == Some(false))
    }

    fn validate(&self) -> Result<()> {
        VariantSelector::RanobeHubTranslation {
            branch: self.slug.clone(),
        }
        .validate()?;
        validate_title(&self.name)?;
        for member in &self.member_details {
            validate_title(&member.name)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Member {
    pub name: String,
    is_ai: Option<bool>,
}

#[derive(Deserialize)]
pub(super) struct ChapterResponse {
    pub chapter: ChapterBody,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChapterBody {
    pub id: u64,
    pub book_id: u64,
    pub html: String,
    pub translation_branch: SelectedTranslation,
    #[serde(default)]
    pub paid: bool,
    #[serde(default)]
    pub is_draft: bool,
}

#[derive(Deserialize)]
pub(super) struct SelectedTranslation {
    pub slug: String,
}
