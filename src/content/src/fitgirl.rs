use url::Url;

use crate::http::SourceHttp;
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "fitgirl/response.rs"]
pub(crate) mod response;

const ORIGIN: &str = "https://fitgirl-repacks.site";

/// FitGirl's official public catalog. Direct and torrent-file mirrors require
/// separate acquisition support; this provider resolves published magnets only.
pub struct FitGirl {
    http: SourceHttp,
    origin: Url,
}

impl FitGirl {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed FitGirl origin");
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
        if category != Category::Games {
            return Ok(Vec::new());
        }

        let mut url = self.origin.clone();
        url.query_pairs_mut().append_pair("s", query);
        let (url, body) = self.http.html_page(url).await?;
        response::search(&body, &self.origin, &url, query)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.release(publication).await?.work)
    }

    /// Re-fetch the exact publication and selected hash, including old repacks.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        self.release(&selected.publication).await?.resolve(selected)
    }

    async fn release(&self, publication: &PublicationRef) -> Result<response::Release> {
        let slug = publication.fitgirl_slug()?;
        let url = self
            .origin
            .join(&format!("/{slug}/"))
            .expect("validated publication slug");
        let (url, body) = match self.http.html_page(url).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        response::release(&body, &self.origin, &url, publication)
    }
}
