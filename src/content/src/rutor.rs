use std::collections::HashSet;

use url::Url;

use crate::http::SourceHttp;
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "rutor/response.rs"]
pub(crate) mod response;

const ORIGIN: &str = "https://rutor.info";

/// RuTor's public torrent catalog, using the application's existing runtime.
pub struct RuTor {
    http: SourceHttp,
    origin: Url,
}

impl RuTor {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed RuTor origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone())?,
            origin,
        })
    }

    /// Each site route must confirm its category before its rows are labelled.
    /// The caller owns fanout, cancellation and publication of partial results.
    pub async fn search(&self, query: &str, category: Category) -> Result<Vec<Work>> {
        let query = query.trim();
        // The site applies its minimum to UTF-8 bytes, not Unicode characters.
        if query.len() < 3 || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let routes: &[u8] = match category {
            Category::Movies => &[1, 5, 12],
            Category::Tv => &[4, 16, 6],
            Category::Anime => &[10],
            Category::Games => &[8],
            Category::Software => &[9],
            Category::Books => &[11],
            Category::Manga => return Ok(Vec::new()),
        };

        let mut works = Vec::new();
        let mut seen = HashSet::new();
        for route in routes {
            let mut url = self.origin.clone();
            url.path_segments_mut()
                .expect("fixed HTTPS origin")
                .clear()
                .extend(["search", "0", &route.to_string(), "100", "0", query, ""]);
            let body = self.http.html(url).await?;
            for work in response::search(&body, &self.origin, *route, category)? {
                if seen.insert(work.reference.clone()) {
                    works.push(work);
                }
            }
        }
        Ok(works)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.release(publication).await?.work)
    }

    /// Re-fetch the exact publication. A replaced hash is never substituted.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        self.release(&selected.publication).await?.resolve(selected)
    }

    async fn release(&self, publication: &PublicationRef) -> Result<response::Release> {
        let id = publication.rutor_id()?;
        let url = self
            .origin
            .join(&format!("/torrent/{id}"))
            .expect("canonical decimal publication path");
        let body = match self.http.html(url).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::release(&body, &self.origin, publication)
    }
}
