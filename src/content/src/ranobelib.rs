mod response;

use std::collections::{BTreeMap, BTreeSet, HashSet};

use reqwest::header::{HeaderMap, HeaderValue, ORIGIN, REFERER};
use url::Url;

use crate::http::SourceHttp;
use crate::{
    Chapter, ChapterRef, Error, PublicationRef, Result, TextChapter, Transport, Variant,
    VariantRef, VariantSelector, Work,
};
use response::{BookRecord, ChapterRecord, ChapterResponse, Response};

const API_ORIGIN: &str = "https://api.cdnlibs.org";
const SITE_ORIGIN: &str = "https://ranobelib.me";

/// Public RanobeLib text and exact translation branches. The caller owns
/// concurrency and cancellation; dropping an operation cancels its requests.
pub struct RanobeLib {
    http: SourceHttp,
    origin: Url,
}

impl RanobeLib {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(API_ORIGIN).expect("fixed RanobeLib API origin");
        let mut headers = HeaderMap::new();
        headers.insert("site-id", HeaderValue::from_static("3"));
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
            .append_pair("site_id[0]", "3")
            .append_pair("q", query);
        let listing: Response<Vec<BookRecord>> = self.http.json(url).await?;
        if listing.data.len() > 60 {
            return Err(Error::InvalidResponse("oversized RanobeLib search"));
        }
        let mut seen = HashSet::new();
        listing
            .data
            .into_iter()
            .map(|record| {
                let work = record.into_work(true)?;
                if !seen.insert(work.reference.clone()) {
                    return Err(Error::InvalidResponse("duplicate RanobeLib publication"));
                }
                Ok(work)
            })
            .collect()
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        publication.ranobelib_id()?;
        let mut url = self.endpoint(&format!("/api/manga/{}", publication.id));
        for field in ["summary", "otherNames"] {
            url.query_pairs_mut().append_pair("fields[]", field);
        }
        let (metadata, chapters) = tokio::try_join!(
            self.http.json::<Response<BookRecord>>(url),
            self.listing(publication)
        )?;
        let mut work = metadata.data.into_work(false)?;
        if work.reference != *publication {
            return Err(Error::InvalidResponse(
                "RanobeLib publication identity changed",
            ));
        }
        let mut translations = BTreeMap::<Option<u64>, BTreeSet<String>>::new();
        for chapter in chapters {
            for branch in chapter.public_branches() {
                let teams = translations.entry(branch.branch_id).or_default();
                teams.extend(branch.teams.iter().map(|team| team.name.clone()));
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
                transport: Transport::Text,
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
            let Some(upload_id) = record
                .public_branches()
                .find(|upload| upload.branch_id == branch)
                .map(|upload| upload.id)
            else {
                continue;
            };
            chapters.push(Chapter {
                reference: ChapterRef {
                    variant: selected.clone(),
                    id: upload_id.to_string(),
                },
                title: record.name.filter(|title| !title.trim().is_empty()),
                volume: Some(record.volume),
                number: Some(record.number),
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
        let VariantSelector::LibTranslation { branch } = selected.variant.selector else {
            return Err(Error::InvalidReference);
        };
        let chapters = self.listing(&selected.variant.publication).await?;
        let record = chapters
            .iter()
            .find(|chapter| {
                chapter.public_branches().any(|upload| {
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
        let response: Response<ChapterResponse> = self.http.json(url).await?;
        response.data.into_text(selected, record)
    }

    async fn listing(&self, publication: &PublicationRef) -> Result<Vec<ChapterRecord>> {
        publication.ranobelib_id()?;
        let url = self.endpoint(&format!("/api/manga/{}/chapters", publication.id));
        let listing: Response<Vec<ChapterRecord>> = self.http.json(url).await?;
        response::validate_listing(listing.data)
    }

    fn endpoint(&self, path: &str) -> Url {
        self.origin
            .join(path)
            .expect("validated RanobeLib API path")
    }
}
