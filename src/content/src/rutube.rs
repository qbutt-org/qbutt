use std::collections::HashSet;

use url::Url;

use crate::http::SourceHttp;
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "rutube/response.rs"]
mod response;

const ORIGIN: &str = "https://rutube.ru";

/// Anonymous, free publications. Network work belongs to the caller's runtime.
pub struct Rutube {
    http: SourceHttp,
    origin: Url,
}

impl Rutube {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Rutube origin");
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
        if !matches!(category, Category::Movies | Category::Tv | Category::Anime) {
            return Ok(Vec::new());
        }
        if let Some(publication) = response::publication_query(query)? {
            let work = self.inspect(&publication).await?;
            return Ok(if work.category == category {
                vec![work]
            } else {
                Vec::new()
            });
        }

        let mut url = self
            .origin
            .join("/api/search/video/")
            .expect("fixed API path");
        url.query_pairs_mut()
            .append_pair("query", query)
            .append_pair("page", "1");
        let response: response::Search = self.http.json(url).await?;
        if response.current_page != 1 || response.results.len() > 100 {
            return Err(Error::InvalidResponse("invalid Rutube search page"));
        }
        let mut seen = HashSet::new();
        let mut works = Vec::new();
        for video in response.results {
            video.validate()?;
            if !seen.insert(video.id.clone()) {
                return Err(Error::InvalidResponse("duplicate Rutube publication"));
            }
            if video.available()
                && video.is_locked == Some(false)
                && video.category() == Some(category)
                && response::matches_query(&video.title, query)
            {
                works.push(video.into_work(&self.origin, true)?);
            }
        }
        Ok(works)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        let video = self.video(publication).await?;
        let (_, quality) = self.stream(publication).await?;
        let mut work = video.into_work(&self.origin, false)?;
        work.variants[0].quality = Some(format!("{}p", quality.height()));
        Ok(work)
    }

    /// Refetch the selected video and its access policy. Signed stream URLs
    /// exist only in this fresh descriptor, never in the publication or library.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        let video = self.video(&selected.publication).await?;
        let (url, quality) = self.stream(&selected.publication).await?;
        Ok(ResolvedContent::DirectVideo {
            reference: selected.clone(),
            title: video.title,
            url,
            quality,
        })
    }

    async fn video(&self, publication: &PublicationRef) -> Result<response::Video> {
        let id = publication.rutube_id()?;
        let url = self
            .origin
            .join(&format!("/api/video/{id}/"))
            .expect("canonical video path");
        let video: response::Video = match self.http.json(url).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        video.validate()?;
        if video.id != id {
            return Err(Error::InvalidResponse(
                "Rutube publication identity changed",
            ));
        }
        if !video.available() || video.category().is_none() {
            return Err(Error::VariantUnavailable);
        }
        Ok(video)
    }

    async fn stream(&self, publication: &PublicationRef) -> Result<(Url, crate::VideoQuality)> {
        let id = publication.rutube_id()?;
        let url = self
            .origin
            .join(&format!("/api/play/options/{id}/"))
            .expect("canonical player path");
        let options: response::Options = match self.http.json(url).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        let address = options.stream(id)?;
        let (url, manifest) = self.http.playlist(address).await?;
        let (url, quality) = response::select_stream(&manifest, &url)?;
        let (url, manifest) = self.http.playlist(url).await?;
        response::validate_media_playlist(&manifest)?;
        Ok((url, quality))
    }
}
