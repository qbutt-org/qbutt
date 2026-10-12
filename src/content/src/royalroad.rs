use url::Url;

use crate::http::SourceHttp;
use crate::{Chapter, ChapterRef, Error, PublicationRef, Result, TextChapter, VariantRef, Work};

#[path = "royalroad/response.rs"]
pub(crate) mod response;

const ORIGIN: &str = "https://www.royalroad.com";

/// Royal Road's public English fiction. The caller owns runtime and cancellation.
pub struct RoyalRoad {
    http: SourceHttp,
    origin: Url,
}

impl RoyalRoad {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Royal Road origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?,
            origin,
        })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let mut url = self
            .origin
            .join("/fictions/search")
            .expect("fixed search path");
        url.query_pairs_mut().append_pair("title", query);
        response::search(&self.http.html(url).await?, &self.origin)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.book(publication).await?.work)
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        Ok(self
            .book(&selected.publication)
            .await?
            .chapters
            .into_iter()
            .map(|entry| entry.chapter)
            .collect())
    }

    pub async fn chapter_text(&self, selected: &ChapterRef) -> Result<TextChapter> {
        selected.validate()?;
        // Fresh public membership prevents stale references from opening a
        // subscription chapter or a chapter reassigned to another fiction.
        let book = self.book(&selected.variant.publication).await?;
        let entry = book
            .chapters
            .into_iter()
            .find(|entry| entry.chapter.reference == *selected)
            .ok_or(Error::VariantUnavailable)?;
        let body = match self.http.html(entry.url).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::chapter(&body, &self.origin, selected)
    }

    async fn book(&self, publication: &PublicationRef) -> Result<response::Book> {
        let id = publication.royalroad_id()?;
        let url = self
            .origin
            .join(&format!("/fiction/{id}"))
            .expect("canonical decimal fiction path");
        let body = match self.http.html(url).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::book(&body, &self.origin, publication)
    }
}
