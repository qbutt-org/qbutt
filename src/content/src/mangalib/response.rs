use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use url::Url;

use crate::http::validate_https_url;
use crate::{
    Category, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result, Source, Work,
};

pub(super) const IMAGE_HOSTS: &[&str] = &["img3.cdnlibs.org", "img2.imglib.info"];

#[derive(Deserialize)]
pub(super) struct Response<T> {
    pub data: T,
}

#[derive(Deserialize)]
pub(super) struct MangaRecord {
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
    summary: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Cover {
    default: String,
}

impl MangaRecord {
    pub(super) fn into_work(self, needs_inspection: bool) -> Result<Work> {
        let reference = PublicationRef {
            source: Source::MangaLib,
            id: self.slug_url,
        };
        if self.model != "manga"
            || self.site != 1
            || reference.mangalib_id()? != self.id
            || self.other_names.len() > 100
            || [&self.name, &self.rus_name, &self.eng_name]
                .into_iter()
                .chain(&self.other_names)
                .any(|title| title.len() > 2048 || title.chars().any(char::is_control))
        {
            return Err(Error::InvalidResponse("invalid MangaLib identity or title"));
        }
        let title = [&self.rus_name, &self.eng_name, &self.name]
            .into_iter()
            .find(|title| !title.trim().is_empty())
            .ok_or(Error::InvalidResponse("missing MangaLib title"))?
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
            .map(|value| summary_text(&value))
            .filter(|text| !text.trim().is_empty());
        Ok(Work {
            url: Url::parse(&format!("https://mangalib.me/ru/manga/{}", reference.id))
                .expect("validated publication slug"),
            identifiers: BTreeMap::from([("mangalib".into(), self.id.to_string())]),
            reference,
            title,
            category: Category::Manga,
            cover,
            description,
            year: None,
            aliases,
            variants: Vec::new(),
            notes: Vec::new(),
            needs_inspection,
        })
    }
}

fn summary_text(value: &serde_json::Value) -> String {
    if let Some(text) = value.as_str() {
        return scraper::Html::parse_fragment(text)
            .root_element()
            .text()
            .collect();
    }
    if let Some(text) = value.get("text").and_then(serde_json::Value::as_str) {
        return text.to_owned();
    }
    let Some(children) = value.get("content").and_then(serde_json::Value::as_array) else {
        return String::new();
    };
    children
        .iter()
        .map(summary_text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Deserialize)]
pub(super) struct ChapterRecord {
    pub index: u32,
    pub volume: String,
    pub number: String,
    pub name: Option<String>,
    pub branches: Vec<Branch>,
}

#[derive(Deserialize)]
pub(super) struct Branch {
    pub id: u64,
    pub branch_id: Option<u64>,
    pub teams: Vec<Team>,
}

#[derive(Deserialize)]
pub(super) struct Team {
    pub name: String,
}

impl ChapterRecord {
    pub(super) fn validate(&self) -> Result<()> {
        let mut branches = HashSet::new();
        if self.index == 0
            || self.branches.len() > 100
            || [&self.volume, &self.number].into_iter().any(|number| {
                number.is_empty() || number.len() > 64 || number.chars().any(char::is_control)
            })
            || self
                .name
                .as_ref()
                .is_some_and(|name| name.len() > 2048 || name.chars().any(char::is_control))
            || self.branches.iter().any(|branch| {
                branch.id == 0
                    || branch.branch_id == Some(0)
                    || !branches.insert(branch.branch_id)
                    || branch.teams.len() > 32
                    || branch.teams.iter().any(|team| {
                        team.name.trim().is_empty()
                            || team.name.len() > 2048
                            || team.name.chars().any(char::is_control)
                    })
            })
        {
            return Err(Error::InvalidResponse("invalid MangaLib chapter or branch"));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
pub(super) struct ChapterResponse {
    id: u64,
    manga_id: u64,
    branch_id: Option<u64>,
    volume: String,
    number: String,
    pages: Vec<Page>,
}

#[derive(Deserialize)]
struct Page {
    id: u64,
    chapter_id: u64,
    slug: u32,
    url: String,
    external: u32,
    chunks: u32,
}

impl ChapterResponse {
    pub(super) fn validate(&self, selected: &ChapterRef, record: &ChapterRecord) -> Result<()> {
        let crate::VariantSelector::LibTranslation { branch } = selected.variant.selector else {
            return Err(Error::InvalidReference);
        };
        if self.id.to_string() != selected.id
            || self.manga_id != selected.variant.publication.mangalib_id()?
            || self.branch_id != branch
            || self.volume != record.volume
            || self.number != record.number
        {
            return Err(Error::VariantUnavailable);
        }
        if self.pages.is_empty() || self.pages.len() > 2000 {
            return Err(Error::InvalidResponse(
                "empty or oversized MangaLib chapter",
            ));
        }
        let mut seen = HashSet::new();
        for (index, page) in self.pages.iter().enumerate() {
            if page.id == 0
                || !seen.insert(page.id)
                || page.chapter_id != self.id
                || page.slug as usize != index + 1
                || page.external != 0
                || page.chunks != 0
                || page.url.is_empty()
                || page.url.len() > 2048
            {
                return Err(Error::InvalidResponse(
                    "invalid MangaLib page identity or order",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn into_pages(self, reference: ChapterRef, server: &Url) -> Result<ChapterPages> {
        let (_, slug) = reference
            .variant
            .publication
            .id
            .split_once("--")
            .expect("validated MangaLib slug");
        let prefix = format!("/manga/{slug}/chapters/{}/", reference.id);
        let mut seen = HashSet::new();
        let images = self
            .pages
            .into_iter()
            .map(|page| {
                let url = if page.url.starts_with("https://") {
                    Url::parse(&page.url).map_err(|_| Error::UnsafeAddress)?
                } else {
                    if !page.url.starts_with("//manga/") {
                        return Err(Error::UnsafeAddress);
                    }
                    server
                        .join(page.url.trim_start_matches('/'))
                        .map_err(|_| Error::UnsafeAddress)?
                };
                validate_image_url(&url)?;
                if url.origin() != server.origin() || !url.path().starts_with(&prefix) {
                    return Err(Error::UnsafeAddress);
                }
                if !seen.insert(url.clone()) {
                    return Err(Error::InvalidResponse("duplicate MangaLib page URL"));
                }
                Ok(ChapterImage { url })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ChapterPages { reference, images })
    }
}

#[derive(Deserialize)]
pub(super) struct Constants {
    #[serde(rename = "imageServers")]
    servers: Vec<ImageServer>,
}

#[derive(Deserialize)]
struct ImageServer {
    id: String,
    site_ids: Vec<u32>,
    url: String,
}

impl Constants {
    pub(super) fn download_server(self) -> Result<Url> {
        if self.servers.len() > 32 {
            return Err(Error::InvalidResponse("too many MangaLib image servers"));
        }
        let mut servers = self
            .servers
            .into_iter()
            .filter(|server| server.id == "download" && server.site_ids.contains(&1));
        let server = servers
            .next()
            .ok_or(Error::InvalidResponse("missing MangaLib download server"))?;
        if servers.next().is_some() {
            return Err(Error::InvalidResponse("duplicate MangaLib download server"));
        }
        let url = Url::parse(&server.url).map_err(|_| Error::UnsafeAddress)?;
        validate_image_url(&url)?;
        if url.path() != "/" {
            return Err(Error::UnsafeAddress);
        }
        Ok(url)
    }
}

pub(super) fn validate_image_url(url: &Url) -> Result<()> {
    validate_https_url(url)?;
    if !url
        .host_str()
        .is_some_and(|host| IMAGE_HOSTS.contains(&host))
        || url.port().is_some()
        || url.query().is_some()
    {
        return Err(Error::UnsafeAddress);
    }
    Ok(())
}
