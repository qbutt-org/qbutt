// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use url::Url;

use crate::http::validate_https_url;
use crate::model::canonical_decimal;
use crate::{
    Category, Chapter, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result,
    Source, Transport, Variant, VariantRef, VariantSelector, Work,
};

#[derive(Deserialize)]
pub(super) struct Cdn {
    image_servers: Vec<Url>,
    thumb_servers: Vec<Url>,
}

impl Cdn {
    pub(super) fn validate(&self) -> Result<()> {
        for (servers, prefix) in [(&self.image_servers, 'i'), (&self.thumb_servers, 't')] {
            if servers.is_empty() || servers.len() > 16 {
                return Err(Error::InvalidResponse("invalid nhentai CDN list"));
            }
            for server in servers {
                validate_https_url(server)?;
                if !cdn_host(server, prefix)
                    || server.port().is_some()
                    || server.path() != "/"
                    || server.query().is_some()
                {
                    return Err(Error::UnsafeAddress);
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
pub(super) struct Search {
    result: Vec<SearchRow>,
    num_pages: u32,
    per_page: u32,
}

#[derive(Deserialize)]
struct SearchRow {
    id: u64,
    media_id: String,
    english_title: String,
    japanese_title: String,
    thumbnail: String,
    num_pages: u32,
    blacklisted: bool,
}

pub(super) fn search(search: Search, cdn: &Cdn, origin: &Url) -> Result<Vec<Work>> {
    if search.per_page == 0
        || search.per_page > 100
        || search.result.len() > search.per_page as usize
        || (!search.result.is_empty() && search.num_pages == 0)
    {
        return Err(Error::InvalidResponse("invalid nhentai search page"));
    }
    let mut seen = HashSet::new();
    let mut works = Vec::new();
    for row in search.result.into_iter().filter(|row| !row.blacklisted) {
        if !seen.insert(row.id) {
            return Err(Error::InvalidResponse("duplicate nhentai search result"));
        }
        validate_counts(row.id, &row.media_id, row.num_pages)?;
        let mut work = base_work(row.id, row.english_title, origin)?;
        if !row.japanese_title.is_empty() && row.japanese_title != work.title {
            work.aliases.push(row.japanese_title);
        }
        work.cover = Some(image_url(
            &cdn.thumb_servers[0],
            &row.thumbnail,
            &row.media_id,
        )?);
        works.push(work);
    }
    Ok(works)
}

#[derive(Deserialize)]
pub(super) struct Gallery {
    id: u64,
    media_id: String,
    title: Titles,
    cover: Image,
    tags: Vec<Tag>,
    scanlator: String,
    num_pages: u32,
    pages: Vec<Page>,
}

#[derive(Deserialize)]
struct Titles {
    pretty: String,
    english: String,
    japanese: String,
}

#[derive(Deserialize)]
struct Tag {
    #[serde(rename = "type")]
    kind: String,
    name: String,
}

#[derive(Deserialize)]
struct Image {
    path: String,
}

#[derive(Deserialize)]
struct Page {
    number: u32,
    path: String,
}

impl Gallery {
    pub(super) fn validate(&self, publication: &PublicationRef) -> Result<()> {
        if self.id.to_string() != publication.id {
            return Err(Error::VariantUnavailable);
        }
        validate_counts(self.id, &self.media_id, self.num_pages)?;
        if self.pages.len() != self.num_pages as usize || self.tags.len() > 256 {
            return Err(Error::InvalidResponse("invalid nhentai gallery manifest"));
        }
        Ok(())
    }

    pub(super) fn work(&self, cdn: &Cdn, origin: &Url) -> Result<Work> {
        let mut work = base_work(self.id, self.title.pretty.clone(), origin)?;
        for title in [&self.title.english, &self.title.japanese] {
            if !title.is_empty() && *title != work.title && !work.aliases.contains(title) {
                work.aliases.push(title.clone());
            }
        }
        work.cover = Some(image_url(
            &cdn.thumb_servers[0],
            &self.cover.path,
            &self.media_id,
        )?);
        let mut languages: Vec<String> = self
            .tags
            .iter()
            .filter(|tag| tag.kind == "language")
            .filter_map(|tag| language(&tag.name).map(str::to_owned))
            .collect();
        languages.sort();
        languages.dedup();
        work.variants.push(Variant {
            reference: VariantRef {
                publication: work.reference.clone(),
                selector: VariantSelector::NhentaiPages,
            },
            title: None,
            transport: Transport::Manga,
            episode: None,
            quality: None,
            translation: (!self.scanlator.trim().is_empty()).then(|| self.scanlator.clone()),
            languages,
            player: None,
            size: None,
            seeders: None,
            info_hash: None,
            reported_video_ads: false,
        });
        work.needs_inspection = false;
        Ok(work)
    }

    pub(super) fn chapter(&self, selected: &VariantRef) -> Chapter {
        Chapter {
            reference: ChapterRef {
                variant: selected.clone(),
                id: selected.publication.id.clone(),
            },
            title: Some(self.title.pretty.clone()),
            volume: None,
            number: None,
            pages: self.num_pages,
        }
    }

    pub(super) fn pages(mut self, selected: &ChapterRef, cdn: &Cdn) -> Result<ChapterPages> {
        self.pages.sort_by_key(|page| page.number);
        let mut images = Vec::with_capacity(self.pages.len());
        let mut seen = HashSet::new();
        for (index, page) in self.pages.into_iter().enumerate() {
            if page.number as usize != index + 1 || !seen.insert(page.path.clone()) {
                return Err(Error::InvalidResponse("invalid nhentai page order"));
            }
            images.push(ChapterImage {
                url: image_url(&cdn.image_servers[0], &page.path, &self.media_id)?,
            });
        }
        Ok(ChapterPages {
            reference: selected.clone(),
            images,
        })
    }
}

fn base_work(id: u64, title: String, origin: &Url) -> Result<Work> {
    if title.trim().is_empty() || title.len() > 2048 || title.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid nhentai title"));
    }
    let id = id.to_string();
    Ok(Work {
        reference: PublicationRef {
            source: Source::Nhentai,
            id: id.clone(),
        },
        title,
        category: Category::Manga,
        url: origin.join(&format!("/g/{id}/")).unwrap(),
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::from([("nhentai".into(), id)]),
        variants: Vec::new(),
        notes: Vec::new(),
        needs_inspection: true,
    })
}

fn validate_counts(id: u64, media: &str, pages: u32) -> Result<()> {
    if id == 0 || !canonical_decimal(media) || !(1..=2000).contains(&pages) {
        return Err(Error::InvalidResponse(
            "invalid nhentai gallery identity or page count",
        ));
    }
    Ok(())
}

fn image_url(server: &Url, path: &str, media: &str) -> Result<Url> {
    let filename = path.strip_prefix(&format!("galleries/{media}/"));
    if !filename.is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 200
            && name != "."
            && name != ".."
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    }) {
        return Err(Error::UnsafeAddress);
    }
    Ok(server.join(path).expect("validated nhentai image path"))
}

fn cdn_host(url: &Url, prefix: char) -> bool {
    url.host_str()
        .and_then(|host| host.strip_suffix(".nhentai.net"))
        .and_then(|host| host.strip_prefix(prefix))
        .is_some_and(|number| {
            !number.is_empty()
                && number.len() <= 3
                && number.bytes().all(|byte| byte.is_ascii_digit())
        })
}

pub(super) fn validate_image(url: &Url) -> Result<()> {
    validate_https_url(url)?;
    if !cdn_host(url, 'i')
        || url.port().is_some()
        || url.query().is_some()
        || !url.path().starts_with("/galleries/")
    {
        return Err(Error::UnsafeAddress);
    }
    Ok(())
}

fn language(name: &str) -> Option<&'static str> {
    Some(match name {
        "english" => "en",
        "japanese" => "ja",
        "chinese" => "zh",
        "russian" => "ru",
        "korean" => "ko",
        "spanish" => "es",
        "portuguese" => "pt",
        "french" => "fr",
        "german" => "de",
        "italian" => "it",
        "vietnamese" => "vi",
        "thai" => "th",
        "indonesian" => "id",
        "polish" => "pl",
        "turkish" => "tr",
        "ukrainian" => "uk",
        _ => return None,
    })
}
