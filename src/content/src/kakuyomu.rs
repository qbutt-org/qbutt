#[path = "kakuyomu/response.rs"]
mod response;

use reqwest::header::HeaderMap;
use url::Url;

use crate::http::SourceHttp;
use crate::{Chapter, ChapterRef, Error, PublicationRef, Result, TextChapter, VariantRef, Work};

const ORIGIN: &str = "https://kakuyomu.jp";

/// Public original Japanese works. Kakuyomu Next publications are excluded.
/// The caller owns the runtime and cancels an operation by dropping its future.
pub struct Kakuyomu {
    http: SourceHttp,
    origin: Url,
}

impl Kakuyomu {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Kakuyomu origin");
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
        let mut url = self.origin.join("/search").expect("fixed search path");
        url.query_pairs_mut().append_pair("q", query);
        let document = self.http.html(url).await?;
        response::search(&document, query)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        self.publication(publication)
            .await
            .map(|listing| listing.work)
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        self.publication(&selected.publication)
            .await
            .map(|listing| listing.chapters)
    }

    pub async fn chapter_text(&self, selected: &ChapterRef) -> Result<TextChapter> {
        selected.validate()?;
        let publication = &selected.variant.publication;
        let listing = self.publication(publication).await?;
        if !listing
            .chapters
            .iter()
            .any(|chapter| chapter.reference == *selected)
        {
            return Err(Error::VariantUnavailable);
        }
        let (_, episode) = selected.id.split_once('/').expect("validated chapter ID");
        let path = format!("/works/{}/episodes/{episode}", publication.id);
        let document = self.page(&path).await?;
        response::chapter(&document, selected, &path)
    }

    async fn publication(&self, publication: &PublicationRef) -> Result<response::Listing> {
        let id = publication.kakuyomu_id()?;
        let path = format!("/works/{id}");
        let document = self.page(&path).await?;
        response::publication(&document, publication)
    }

    async fn page(&self, path: &str) -> Result<String> {
        let requested = self.origin.join(path).expect("validated Kakuyomu path");
        let (url, document) = match self.http.html_page(requested.clone()).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        if url != requested {
            return Err(Error::VariantUnavailable);
        }
        Ok(document)
    }
}
