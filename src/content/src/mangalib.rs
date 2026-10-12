mod response;

use std::collections::{BTreeMap, BTreeSet, HashSet};

use reqwest::header::{HeaderMap, HeaderValue, ORIGIN, REFERER};
use url::Url;

use crate::http::SourceHttp;
use crate::{
    Chapter, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result, Transport,
    Variant, VariantRef, VariantSelector, Work,
};
use response::{ChapterRecord, ChapterResponse, Constants, MangaRecord, Response};

const API_ORIGIN: &str = "https://api.cdnlibs.org";
const SITE_ORIGIN: &str = "https://mangalib.me";
const MAX_CHAPTERS: usize = 10_000;

/// MangaLib's public catalog and exact translation branches. The caller owns
/// the runtime, bounded concurrency and cancellation of each operation.
pub struct MangaLib {
    http: SourceHttp,
    origin: Url,
}

impl MangaLib {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(API_ORIGIN).expect("fixed MangaLib API origin");
        let mut headers = HeaderMap::new();
        headers.insert("site-id", HeaderValue::from_static("1"));
        headers.insert(ORIGIN, HeaderValue::from_static(SITE_ORIGIN));
        headers.insert(REFERER, HeaderValue::from_static(SITE_ORIGIN));
        Ok(Self {
            http: SourceHttp::new(origin.clone(), headers)?,
            origin,
        })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let mut url = self.endpoint("/api/manga");
        url.query_pairs_mut()
            .append_pair("site_id[0]", "1")
            .append_pair("q", query);
        let listing: Response<Vec<MangaRecord>> = self.http.json(url).await?;
        if listing.data.len() > 60 {
            return Err(Error::InvalidResponse("oversized MangaLib search"));
        }
        let mut seen = HashSet::new();
        listing
            .data
            .into_iter()
            .map(|record| {
                let work = record.into_work(true)?;
                if !seen.insert(work.reference.clone()) {
                    return Err(Error::InvalidResponse("duplicate MangaLib publication"));
                }
                Ok(work)
            })
            .collect()
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        publication.mangalib_id()?;
        let mut url = self.endpoint(&format!("/api/manga/{}", publication.id));
        for field in ["summary", "otherNames"] {
            url.query_pairs_mut().append_pair("fields[]", field);
        }
        let (metadata, chapters) = tokio::try_join!(
            self.http.json::<Response<MangaRecord>>(url),
            self.listing(publication)
        )?;
        let mut work = metadata.data.into_work(false)?;
        if work.reference != *publication {
            return Err(Error::InvalidResponse(
                "MangaLib publication identity changed",
            ));
        }
        let mut translations = BTreeMap::<Option<u64>, BTreeSet<String>>::new();
        for chapter in chapters {
            for branch in chapter.branches {
                let teams = translations.entry(branch.branch_id).or_default();
                for team in branch.teams {
                    teams.insert(team.name);
                }
            }
        }
        work.variants = translations
            .into_iter()
            .map(|(branch, teams)| Variant {
                reference: VariantRef {
                    publication: publication.clone(),
                    selector: VariantSelector::LibTranslation { branch },
                },
                title: None,
                transport: Transport::Manga,
                episode: None,
                quality: None,
                translation: (!teams.is_empty())
                    .then(|| teams.into_iter().collect::<Vec<_>>().join(", ")),
                languages: Vec::new(),
                player: None,
                size: None,
                seeders: None,
                info_hash: None,
                reported_video_ads: false,
            })
            .collect();
        Ok(work)
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        let VariantSelector::LibTranslation { branch } = selected.selector else {
            return Err(Error::InvalidReference);
        };
        let mut chapters = Vec::new();
        for record in self.listing(&selected.publication).await? {
            if let Some(upload) = record
                .branches
                .iter()
                .find(|upload| upload.branch_id == branch)
            {
                chapters.push(Chapter {
                    reference: ChapterRef {
                        variant: selected.clone(),
                        id: upload.id.to_string(),
                    },
                    title: record.name.filter(|title| !title.trim().is_empty()),
                    volume: Some(record.volume),
                    number: Some(record.number),
                    // The listing does not report page counts. The exact manifest
                    // supplies this when the chapter is opened.
                    pages: 0,
                });
            }
        }
        if chapters.is_empty() {
            return Err(Error::VariantUnavailable);
        }
        Ok(chapters)
    }

    pub async fn chapter_pages(&self, selected: &ChapterRef) -> Result<ChapterPages> {
        selected.validate()?;
        let VariantSelector::LibTranslation { branch } = selected.variant.selector else {
            return Err(Error::InvalidReference);
        };
        let chapters = self.listing(&selected.variant.publication).await?;
        let record = chapters
            .iter()
            .find(|chapter| {
                chapter.branches.iter().any(|upload| {
                    upload.id.to_string() == selected.id && upload.branch_id == branch
                })
            })
            .ok_or(Error::VariantUnavailable)?;
        let mut url = self.endpoint(&format!(
            "/api/manga/{}/chapter",
            selected.variant.publication.id
        ));
        url.query_pairs_mut()
            .append_pair("branch_id", &branch.unwrap_or(0).to_string())
            .append_pair("number", &record.number)
            .append_pair("volume", &record.volume);
        let chapter: Response<ChapterResponse> = self.http.json(url).await?;
        chapter.data.validate(selected, record)?;
        let mut url = self.endpoint("/api/constants");
        url.query_pairs_mut()
            .append_pair("fields[]", "imageServers");
        let constants: Response<Constants> = self.http.json(url).await?;
        let server = constants.data.download_server()?;
        chapter.data.into_pages(selected.clone(), &server)
    }

    pub async fn image_bytes(&self, image: &ChapterImage) -> Result<Vec<u8>> {
        response::validate_image_url(&image.url)?;
        self.http
            .image(image.url.clone(), response::IMAGE_HOSTS)
            .await
    }

    async fn listing(&self, publication: &PublicationRef) -> Result<Vec<ChapterRecord>> {
        publication.mangalib_id()?;
        let url = self.endpoint(&format!("/api/manga/{}/chapters", publication.id));
        let listing: Response<Vec<ChapterRecord>> = self.http.json(url).await?;
        if listing.data.len() > MAX_CHAPTERS {
            return Err(Error::InvalidResponse("too many MangaLib chapters"));
        }
        let mut seen = HashSet::new();
        let mut ordering = HashSet::new();
        for record in &listing.data {
            record.validate()?;
            if !ordering.insert(record.index) {
                return Err(Error::InvalidResponse("duplicate chapter order"));
            }
            for branch in &record.branches {
                if !seen.insert(branch.id) {
                    return Err(Error::InvalidResponse("duplicate chapter upload"));
                }
            }
        }
        let mut chapters = listing.data;
        chapters.sort_by_key(|chapter| chapter.index);
        Ok(chapters)
    }

    fn endpoint(&self, path: &str) -> Url {
        self.origin.join(path).expect("validated MangaLib API path")
    }
}
