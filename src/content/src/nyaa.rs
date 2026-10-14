use url::Url;

use crate::http::SourceHttp;
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "nyaa/description.rs"]
mod description;
#[path = "nyaa/response.rs"]
pub(crate) mod response;

const ORIGIN: &str = "https://nyaa.si";

/// The public catalog shares the application's bounded source HTTP owner.
pub struct Nyaa {
    http: SourceHttp,
    origin: Url,
}

impl Nyaa {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Nyaa origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?,
            origin,
        })
    }

    pub async fn search(&self, query: &str, category: Category) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let route = match category {
            Category::Anime => "1_0",
            Category::Books => "3_0",
            Category::Software => "6_1",
            Category::Games => "6_2",
            Category::Movies | Category::Tv | Category::Video | Category::Manga => {
                return Ok(Vec::new());
            }
        };
        let mut url = self.origin.clone();
        url.query_pairs_mut().extend_pairs([
            ("f", "0"),
            ("c", route),
            ("q", query),
            ("s", "id"),
            ("o", "desc"),
        ]);
        let body = self.http.html(url).await?;
        response::search(&body, &self.origin, query, route, category)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.release(publication).await?.work)
    }

    /// A fresh inspection must still match the selected publication and hash.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        self.release(&selected.publication).await?.resolve(selected)
    }

    async fn release(&self, publication: &PublicationRef) -> Result<response::Release> {
        let id = publication.nyaa_id()?;
        let url = self
            .origin
            .join(&format!("/view/{id}"))
            .expect("canonical publication path");
        let body = match self.http.html(url).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::release(&body, &self.origin, publication)
    }
}
