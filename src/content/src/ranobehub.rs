#[path = "ranobehub/response.rs"]
mod response;

use std::collections::{BTreeMap, BTreeSet, HashSet};

use reqwest::header::HeaderMap;
use url::Url;

use crate::http::SourceHttp;
use crate::{
    Chapter, ChapterRef, Error, PublicationRef, Result, TextChapter, Transport, Variant,
    VariantRef, VariantSelector, Work,
};
use response::{ChapterListing, ChapterRecord, ChapterResponse, SearchResponse};

const ORIGIN: &str = "https://ranobehub.org";
const PAGE_SIZE: usize = 250;
const MAX_CHAPTERS: usize = 30_000;

/// RanobeHub's public human translations. Operations borrow the caller's runtime;
/// dropping their futures cancels requests without a provider-owned task.
pub struct RanobeHub {
    http: SourceHttp,
    origin: Url,
}

impl RanobeHub {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed RanobeHub origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone(), HeaderMap::new())?,
            origin,
        })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }

        let mut url = self.origin.join("/api/search").expect("fixed search path");
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("rh_client", "modern");
        let response: SearchResponse = self.http.json(url).await?;
        if response.books.len() > 100 {
            return Err(Error::InvalidResponse("oversized RanobeHub search"));
        }

        let mut seen = HashSet::new();
        response
            .books
            .into_iter()
            .map(|book| {
                let work = book.into_work(&self.origin)?;
                if !seen.insert(work.reference.clone()) {
                    return Err(Error::InvalidResponse("duplicate RanobeHub publication"));
                }
                Ok(work)
            })
            .collect()
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        let id = publication.ranobehub_id()?;
        let page_url = self
            .origin
            .join(&format!("/ranobe/{id}"))
            .expect("numeric book path");
        let (page, chapters) =
            tokio::try_join!(self.http.html(page_url), self.listing(publication))?;
        let mut work = response::work_page(publication, &self.origin, &page)?;

        let mut translations = BTreeMap::<String, (String, BTreeSet<String>)>::new();
        for chapter in chapters.iter().filter(|chapter| chapter.is_public()) {
            for branch in chapter
                .translation_options
                .iter()
                .filter(|branch| branch.is_human())
            {
                let entry = translations
                    .entry(branch.slug.clone())
                    .or_insert_with(|| (branch.name.clone(), BTreeSet::new()));
                entry.1.extend(
                    branch
                        .member_details
                        .iter()
                        .map(|member| member.name.clone()),
                );
            }
        }
        work.variants = translations
            .into_iter()
            .map(|(branch, (name, members))| Variant {
                reference: VariantRef {
                    publication: publication.clone(),
                    selector: VariantSelector::RanobeHubTranslation { branch },
                },
                title: None,
                transport: Transport::Text,
                episode: None,
                quality: None,
                translation: Some(format!(
                    "{name} ({})",
                    members.into_iter().collect::<Vec<_>>().join(", ")
                )),
                // The public API does not declare the language of a translation.
                languages: Vec::new(),
                player: None,
                size: None,
                seeders: None,
                info_hash: None,
                reported_video_ads: false,
            })
            .collect();
        if work.variants.is_empty() {
            return Err(Error::VariantUnavailable);
        }
        Ok(work)
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        let VariantSelector::RanobeHubTranslation { branch } = &selected.selector else {
            return Err(Error::InvalidReference);
        };

        let mut chapters = Vec::new();
        let mut seen = HashSet::new();
        for chapter in self.listing(&selected.publication).await? {
            let Some(translation) = chapter.translation(branch) else {
                continue;
            };
            let id = translation.chapter_id.unwrap_or(chapter.id);
            if id == 0 || !seen.insert(id) {
                return Err(Error::InvalidResponse(
                    "duplicate RanobeHub translation chapter",
                ));
            }
            chapters.push(Chapter {
                reference: ChapterRef {
                    variant: selected.clone(),
                    id: id.to_string(),
                },
                title: Some(chapter.title),
                volume: Some(chapter.volume.to_string()),
                number: Some(chapter.number.to_string()),
                pages: 0,
            });
        }
        if chapters.is_empty() {
            return Err(Error::VariantUnavailable);
        }
        Ok(chapters)
    }

    pub async fn chapter_text(&self, selected: &ChapterRef) -> Result<TextChapter> {
        selected.validate()?;
        let VariantSelector::RanobeHubTranslation { branch } = &selected.variant.selector else {
            return Err(Error::InvalidReference);
        };
        let chapters = self.listing(&selected.variant.publication).await?;
        if !chapters.iter().any(|chapter| {
            chapter.translation(branch).is_some_and(|translation| {
                translation.chapter_id.unwrap_or(chapter.id).to_string() == selected.id
            })
        }) {
            return Err(Error::VariantUnavailable);
        }

        let mut url = self
            .origin
            .join(&format!("/api/chapters/{}", selected.id))
            .expect("validated numeric chapter path");
        url.query_pairs_mut().append_pair("translation", branch);
        let response: ChapterResponse = self.http.json(url).await?;
        let chapter = response.chapter;
        if chapter.id.to_string() != selected.id
            || chapter.book_id.to_string() != selected.variant.publication.id
            || chapter.translation_branch.slug != *branch
        {
            return Err(Error::InvalidResponse("RanobeHub chapter identity changed"));
        }
        if chapter.paid || chapter.is_draft {
            return Err(Error::VariantUnavailable);
        }
        TextChapter::from_html(selected.clone(), &chapter.html)
    }

    async fn listing(&self, publication: &PublicationRef) -> Result<Vec<ChapterRecord>> {
        let id = publication.ranobehub_id()?;
        let mut chapters = Vec::new();
        let mut seen = HashSet::new();
        let mut expected_total = None;
        loop {
            let offset = chapters.len();
            let mut url = self
                .origin
                .join(&format!("/api/books/{id}/chapters"))
                .expect("numeric chapter listing path");
            url.query_pairs_mut()
                .append_pair("limit", &PAGE_SIZE.to_string())
                .append_pair("offset", &offset.to_string())
                .append_pair("sort", "asc");
            let page: ChapterListing = self.http.json(url).await?;
            if page.total > MAX_CHAPTERS || page.items.len() > PAGE_SIZE {
                return Err(Error::ResponseTooLarge);
            }
            if page.offset != offset || expected_total.is_some_and(|total| total != page.total) {
                return Err(Error::InvalidResponse("RanobeHub chapter listing changed"));
            }
            expected_total = Some(page.total);
            for chapter in page.items {
                chapter.validate(id)?;
                if !seen.insert(chapter.id) {
                    return Err(Error::InvalidResponse("duplicate RanobeHub chapter"));
                }
                chapters.push(chapter);
            }
            if chapters.len() == page.total && page.next_offset.is_none() {
                return Ok(chapters);
            }
            let next = page.next_offset.ok_or(Error::InvalidResponse(
                "incomplete RanobeHub chapter listing",
            ))?;
            if next <= offset || next != chapters.len() || chapters.len() >= page.total {
                return Err(Error::InvalidResponse("invalid RanobeHub pagination"));
            }
        }
    }
}
