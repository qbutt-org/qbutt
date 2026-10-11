mod response;

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::sync::Mutex;
use tokio::time::Instant;
use url::Url;

use crate::http::SourceHttp;
use crate::model::canonical_language;
use crate::{
    Chapter, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result, Source,
    Transport, Variant, VariantRef, VariantSelector, Work,
};
use response::{AtHome, ChapterRecord, Collection, EntityResponse, MangaRecord};

const API_ORIGIN: &str = "https://api.mangadex.org";
const FEED_LIMIT: usize = 500;
const MAX_CHAPTERS: usize = 10_000;

/// MangaDex's public API. Futures are owned and cancelled by the caller. The
/// service shares API pacing across search, inspection and chapter requests.
pub struct MangaDex {
    http: SourceHttp,
    origin: Url,
    next_api_request: Mutex<Instant>,
    next_manifest_request: Mutex<Instant>,
}

impl MangaDex {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(API_ORIGIN).expect("fixed MangaDex API origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone())?,
            origin,
            next_api_request: Mutex::new(Instant::now()),
            next_manifest_request: Mutex::new(Instant::now()),
        })
    }

    pub async fn search(&self, query: &str, language: &str) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        validate_language(language)?;
        let mut url = self.endpoint("/manga");
        url.query_pairs_mut()
            .append_pair("title", query)
            .append_pair("limit", "50")
            .append_pair("includes[]", "cover_art");
        include_ratings(&mut url);
        let listing: Collection<MangaRecord> = self.api(url).await?;
        listing.validate(0, 50)?;
        let mut seen = HashSet::new();
        listing
            .data
            .into_iter()
            .map(|manga| {
                if !seen.insert(manga.id.clone()) {
                    return Err(Error::InvalidResponse("duplicate manga identity"));
                }
                manga.into_work(language, true)
            })
            .collect()
    }

    pub async fn inspect(&self, publication: &PublicationRef, language: &str) -> Result<Work> {
        validate_publication(publication)?;
        validate_language(language)?;
        let mut url = self.endpoint(&format!("/manga/{}", publication.id));
        url.query_pairs_mut().append_pair("includes[]", "cover_art");
        let response: EntityResponse<MangaRecord> = self.api(url).await?;
        let manga = response.entity()?;
        if manga.id != publication.id {
            return Err(Error::InvalidResponse("manga identity changed"));
        }
        let mut work = manga.into_work(language, false)?;
        let chapters = self.feed(publication).await?;
        let mut branches = BTreeMap::new();
        for chapter in &chapters {
            let branch = chapter.branch()?;
            let key = (branch.language.clone(), branch.groups.clone());
            let entry = branches.entry(key).or_insert((branch, 0usize));
            entry.1 += 1;
        }
        let mut branches = branches.into_values().collect::<Vec<_>>();
        branches.sort_by(|(left, left_count), (right, right_count)| {
            (right.language == language)
                .cmp(&(left.language == language))
                .then_with(|| right_count.cmp(left_count))
                .then_with(|| left.language.cmp(&right.language))
                .then_with(|| left.groups.cmp(&right.groups))
        });
        work.variants = branches
            .into_iter()
            .map(|(branch, _)| Variant {
                reference: VariantRef {
                    publication: publication.clone(),
                    selector: VariantSelector::MangaTranslation {
                        language: branch.language.clone(),
                        groups: branch.groups,
                    },
                },
                title: None,
                transport: Transport::Manga,
                episode: None,
                quality: None,
                translation: branch.translation,
                languages: vec![branch.language],
                player: None,
                size: None,
                seeders: None,
                info_hash: None,
                reported_video_ads: false,
            })
            .collect();
        Ok(work)
    }

    /// Refetch this translation branch. Upload UUIDs remain separate even when
    /// two uploads have the same chapter number; no other branch is substituted.
    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        let VariantSelector::MangaTranslation { language, groups } = &selected.selector else {
            return Err(Error::InvalidReference);
        };
        let mut chapters = Vec::new();
        for record in self.feed(&selected.publication).await? {
            let branch = record.branch()?;
            if branch.language == *language && branch.groups == *groups {
                chapters.push(record.into_chapter(selected.clone())?);
            }
        }
        if chapters.is_empty() {
            return Err(Error::VariantUnavailable);
        }
        chapters.sort_by(response::compare_chapters);
        Ok(chapters)
    }

    /// Validate the upload's actual manga, language and groups before retrieving
    /// its short-lived manifest. Never persist or share the returned image URLs.
    pub async fn chapter_pages(&self, selected: &ChapterRef) -> Result<ChapterPages> {
        selected.validate()?;
        let mut url = self.endpoint(&format!("/chapter/{}", selected.id));
        url.query_pairs_mut()
            .append_pair("includes[]", "scanlation_group");
        let response: EntityResponse<ChapterRecord> = self.api(url).await?;
        let chapter = response.entity()?;
        chapter.matches(selected)?;

        // The endpoint has a separate allowance of 40 requests per minute.
        let mut next = self.next_manifest_request.lock().await;
        tokio::time::sleep_until(*next).await;
        *next = Instant::now() + Duration::from_millis(1500);
        drop(next);

        let url = self.endpoint(&format!("/at-home/server/{}", selected.id));
        let manifest: AtHome = self.api(url).await?;
        manifest.into_pages(selected.clone(), chapter.attributes.pages)
    }

    pub async fn image_bytes(&self, image: &ChapterImage) -> Result<Vec<u8>> {
        self.http.image(image.url.clone()).await
    }

    async fn feed(&self, publication: &PublicationRef) -> Result<Vec<ChapterRecord>> {
        validate_publication(publication)?;
        let mut chapters = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = 0;
        loop {
            let mut url = self.endpoint(&format!("/manga/{}/feed", publication.id));
            url.query_pairs_mut()
                .append_pair("limit", &FEED_LIMIT.to_string())
                .append_pair("offset", &offset.to_string())
                .append_pair("includes[]", "scanlation_group")
                .append_pair("order[volume]", "asc")
                .append_pair("order[chapter]", "asc");
            include_ratings(&mut url);
            let listing: Collection<ChapterRecord> = self.api(url).await?;
            listing.validate(offset, FEED_LIMIT)?;
            if listing.total > MAX_CHAPTERS {
                return Err(Error::InvalidResponse(
                    "manga chapter feed exceeds its limit",
                ));
            }
            let count = listing.data.len();
            for chapter in listing.data {
                chapter.validate(publication)?;
                if !seen.insert(chapter.id.clone()) {
                    return Err(Error::InvalidResponse("duplicate chapter identity"));
                }
                if chapter.readable() {
                    chapters.push(chapter);
                }
            }
            offset += count;
            if offset >= listing.total {
                break;
            }
            if count == 0 || offset + FEED_LIMIT > MAX_CHAPTERS {
                return Err(Error::InvalidResponse("incomplete manga chapter feed"));
            }
        }
        Ok(chapters)
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut url = self.origin.clone();
        url.set_path(path);
        url
    }

    async fn api<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let mut next = self.next_api_request.lock().await;
        tokio::time::sleep_until(*next).await;
        *next = Instant::now() + Duration::from_millis(200);
        drop(next);
        self.http.json(url).await
    }
}

fn validate_publication(publication: &PublicationRef) -> Result<()> {
    publication.validate()?;
    if publication.source == Source::MangaDex {
        Ok(())
    } else {
        Err(Error::InvalidReference)
    }
}

fn validate_language(language: &str) -> Result<()> {
    if canonical_language(language) {
        Ok(())
    } else {
        Err(Error::InvalidReference)
    }
}

fn include_ratings(url: &mut Url) {
    for rating in ["safe", "suggestive", "erotica", "pornographic"] {
        url.query_pairs_mut().append_pair("contentRating[]", rating);
    }
}
