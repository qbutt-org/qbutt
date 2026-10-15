//! Bounded API DTOs and source-specific identity mapping.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use url::Url;

use crate::http::validate_https_url;
use crate::model::{canonical_language, canonical_uuid};
use crate::{
    Category, Chapter, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result,
    Source, VariantRef, VariantSelector, Work,
};

#[derive(Deserialize)]
pub(crate) struct Collection<T> {
    pub data: Vec<T>,
    result: String,
    limit: usize,
    offset: usize,
    pub total: usize,
}

impl<T> Collection<T> {
    pub(crate) fn validate(&self, offset: usize, limit: usize) -> Result<()> {
        if self.result != "ok"
            || self.offset != offset
            || self.limit != limit
            || self.data.len() > limit
            || self.data.len() > self.total.saturating_sub(offset)
        {
            Err(Error::InvalidResponse("invalid collection result"))
        } else {
            Ok(())
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct EntityResponse<T> {
    result: String,
    data: T,
}

impl<T> EntityResponse<T> {
    pub(crate) fn entity(self) -> Result<T> {
        if self.result == "ok" {
            Ok(self.data)
        } else {
            Err(Error::InvalidResponse("invalid entity result"))
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelationshipAttributes {
    name: Option<String>,
    file_name: Option<String>,
}

#[derive(Deserialize)]
struct Relationship {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    attributes: Option<RelationshipAttributes>,
}

#[derive(Deserialize)]
pub(crate) struct MangaRecord {
    pub id: String,
    #[serde(rename = "type")]
    kind: String,
    attributes: MangaAttributes,
    #[serde(default)]
    relationships: Vec<Relationship>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MangaAttributes {
    title: BTreeMap<String, String>,
    #[serde(default)]
    alt_titles: Vec<BTreeMap<String, String>>,
    #[serde(default)]
    description: BTreeMap<String, String>,
    year: Option<u16>,
    #[serde(default)]
    available_translated_languages: Vec<String>,
}

impl MangaRecord {
    pub(crate) fn into_work(self, language: &str, needs_inspection: bool) -> Result<Work> {
        if self.kind != "manga"
            || !canonical_uuid(&self.id)
            || self.attributes.title.is_empty()
            || self.attributes.title.len() > 100
            || self.attributes.alt_titles.len() > 500
            || self.attributes.description.len() > 100
            || self.relationships.len() > 1000
            || self.attributes.available_translated_languages.len() > 100
            || self
                .attributes
                .available_translated_languages
                .iter()
                .any(|value| !canonical_language(value))
        {
            return Err(Error::InvalidResponse("invalid manga identity or metadata"));
        }
        let names =
            std::iter::once(&self.attributes.title).chain(self.attributes.alt_titles.iter());
        for name in names.clone() {
            if name.len() > 100 || name.values().any(|value| value.len() > 2048) {
                return Err(Error::InvalidResponse("manga titles exceed their limit"));
            }
        }
        if self
            .attributes
            .description
            .values()
            .any(|value| value.len() > 128 * 1024)
        {
            return Err(Error::InvalidResponse(
                "manga description exceeds its limit",
            ));
        }
        let title = names
            .clone()
            .filter_map(|name| name.get(language))
            .find(|value| !value.trim().is_empty())
            .or_else(|| self.attributes.title.get("en"))
            .or_else(|| self.attributes.title.values().next())
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or(Error::InvalidResponse("missing manga title"))?;
        let mut aliases = Vec::new();
        for value in names.flat_map(|name| name.values()) {
            if !value.trim().is_empty() && value != &title && !aliases.contains(value) {
                aliases.push(value.clone());
            }
        }
        let cover = self
            .relationships
            .iter()
            .find(|relation| relation.kind == "cover_art")
            .and_then(|relation| relation.attributes.as_ref())
            .and_then(|attributes| attributes.file_name.as_ref())
            .map(|filename| {
                validate_filename(filename)?;
                let mut url = Url::parse("https://uploads.mangadex.org")
                    .expect("fixed MangaDex cover origin");
                url.path_segments_mut()
                    .expect("fixed HTTPS origin")
                    .extend(["covers", &self.id, &format!("{filename}.256.jpg")]);
                Ok(url)
            })
            .transpose()?;
        let reference = PublicationRef {
            source: Source::MangaDex,
            id: self.id.clone(),
        };
        Ok(Work {
            reference,
            title,
            category: Category::Manga,
            url: Url::parse(&format!("https://mangadex.org/title/{}", self.id))
                .expect("validated MangaDex UUID"),
            cover,
            description: self
                .attributes
                .description
                .get(language)
                .or_else(|| self.attributes.description.get("en"))
                .filter(|value| !value.trim().is_empty())
                .cloned(),
            year: self.attributes.year,
            aliases,
            identifiers: BTreeMap::from([("mangadex".into(), self.id)]),
            variants: Vec::new(),
            notes: Vec::new(),
            needs_inspection,
        })
    }
}

#[derive(Deserialize)]
pub(crate) struct ChapterRecord {
    pub id: String,
    #[serde(rename = "type")]
    kind: String,
    pub attributes: ChapterAttributes,
    relationships: Vec<Relationship>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChapterAttributes {
    volume: Option<String>,
    chapter: Option<String>,
    title: Option<String>,
    translated_language: String,
    external_url: Option<String>,
    #[serde(default)]
    is_unavailable: bool,
    pub pages: u32,
}

pub(crate) struct Branch {
    pub language: String,
    pub groups: Vec<String>,
    pub translation: Option<String>,
}

impl ChapterRecord {
    pub(crate) fn validate(&self, publication: &PublicationRef) -> Result<()> {
        let mangas = self
            .relationships
            .iter()
            .filter(|relation| relation.kind == "manga")
            .collect::<Vec<_>>();
        if self.kind != "chapter"
            || !canonical_uuid(&self.id)
            || self.relationships.len() > 100
            || mangas.len() != 1
            || mangas[0].id != publication.id
            || self.attributes.pages > 2000
            || [&self.attributes.volume, &self.attributes.chapter]
                .into_iter()
                .flatten()
                .any(|number| number.len() > 64 || number.chars().any(char::is_control))
            || self
                .attributes
                .title
                .as_ref()
                .is_some_and(|title| title.len() > 2048)
        {
            return Err(Error::InvalidResponse(
                "invalid chapter identity or metadata",
            ));
        }
        self.branch()?;
        Ok(())
    }

    pub(crate) fn readable(&self) -> bool {
        self.attributes.pages != 0
            && self.attributes.external_url.is_none()
            && !self.attributes.is_unavailable
    }

    pub(crate) fn branch(&self) -> Result<Branch> {
        if !canonical_language(&self.attributes.translated_language) {
            return Err(Error::InvalidResponse("invalid chapter language"));
        }
        let mut groups = self
            .relationships
            .iter()
            .filter(|relation| relation.kind == "scanlation_group")
            .collect::<Vec<_>>();
        groups.sort_by(|left, right| left.id.cmp(&right.id));
        if groups.len() > 32
            || groups.iter().any(|group| !canonical_uuid(&group.id))
            || groups.windows(2).any(|pair| pair[0].id == pair[1].id)
        {
            return Err(Error::InvalidResponse("invalid chapter translation groups"));
        }
        let mut names = Vec::new();
        for group in &groups {
            if let Some(name) = group
                .attributes
                .as_ref()
                .and_then(|attributes| attributes.name.as_ref())
            {
                if name.len() > 2048 || name.chars().any(char::is_control) {
                    return Err(Error::InvalidResponse("invalid translation group name"));
                }
                if !name.trim().is_empty() {
                    names.push(name.as_str());
                }
            }
        }
        Ok(Branch {
            language: self.attributes.translated_language.clone(),
            groups: groups.into_iter().map(|group| group.id.clone()).collect(),
            translation: (!names.is_empty()).then(|| names.join(", ")),
        })
    }

    pub(crate) fn into_chapter(self, variant: VariantRef) -> Result<Chapter> {
        self.validate(&variant.publication)?;
        Ok(Chapter {
            reference: ChapterRef {
                variant,
                id: self.id,
            },
            title: self
                .attributes
                .title
                .filter(|title| !title.trim().is_empty()),
            volume: self.attributes.volume.filter(|volume| !volume.is_empty()),
            number: self
                .attributes
                .chapter
                .filter(|chapter| !chapter.is_empty()),
            pages: self.attributes.pages,
        })
    }

    pub(crate) fn matches(&self, selected: &ChapterRef) -> Result<()> {
        if self.id != selected.id {
            return Err(Error::InvalidResponse("chapter identity changed"));
        }
        self.validate(&selected.variant.publication)?;
        let branch = self.branch()?;
        let expected = VariantSelector::MangaTranslation {
            language: branch.language,
            groups: branch.groups,
        };
        if !self.readable() || selected.variant.selector != expected {
            Err(Error::VariantUnavailable)
        } else {
            Ok(())
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AtHome {
    result: String,
    base_url: Url,
    chapter: AtHomeChapter,
}

#[derive(Deserialize)]
struct AtHomeChapter {
    hash: String,
    data: Vec<String>,
}

impl AtHome {
    pub(crate) fn into_pages(self, reference: ChapterRef, pages: u32) -> Result<ChapterPages> {
        validate_https_url(&self.base_url)?;
        if self.result != "ok"
            || self.base_url.query().is_some()
            || self.base_url.as_str().len() > 8192
            || self.chapter.hash.len() != 32
            || !self
                .chapter
                .hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.chapter.data.is_empty()
            || self.chapter.data.len() > 2000
            || self.chapter.data.len() != pages as usize
        {
            return Err(Error::InvalidResponse("invalid chapter page manifest"));
        }
        let mut seen = HashSet::new();
        let mut images = Vec::with_capacity(self.chapter.data.len());
        for filename in self.chapter.data {
            validate_filename(&filename)?;
            if !seen.insert(filename.clone()) {
                return Err(Error::InvalidResponse("duplicate chapter image"));
            }
            let mut url = self.base_url.clone();
            url.path_segments_mut()
                .map_err(|_| Error::UnsafeAddress)?
                .pop_if_empty()
                .extend(["data", &self.chapter.hash, &filename]);
            images.push(ChapterImage { url });
        }
        Ok(ChapterPages { reference, images })
    }
}

fn validate_filename(value: &str) -> Result<()> {
    if !value.is_empty()
        && value.len() <= 256
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Ok(())
    } else {
        Err(Error::InvalidResponse("invalid image filename"))
    }
}

pub(crate) fn compare_chapters(left: &Chapter, right: &Chapter) -> Ordering {
    compare_number(&left.volume, &right.volume)
        .then_with(|| compare_number(&left.number, &right.number))
        .then_with(|| left.reference.id.cmp(&right.reference.id))
}

fn compare_number(left: &Option<String>, right: &Option<String>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left_number = left.parse::<f64>().ok().filter(|value| value.is_finite());
            let right_number = right.parse::<f64>().ok().filter(|value| value.is_finite());
            match (left_number, right_number) {
                (Some(left), Some(right)) => left.total_cmp(&right),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => left.cmp(right),
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
