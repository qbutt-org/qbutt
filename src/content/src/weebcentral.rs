// SPDX-License-Identifier: GPL-3.0-or-later
#[path = "weebcentral/response.rs"]
mod response;

use reqwest::header::HeaderMap;
use url::Url;

use crate::http::SourceHttp;
use crate::{
    Chapter, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result, Source,
    VariantRef, Work,
};

const ORIGIN: &str = "https://weebcentral.com";
const IMAGE_HOSTS: &[&str] = &["hot.planeptune.us", "temp.compsci88.com"];
const MAX_CHAPTER_LIST_BYTES: usize = 4 * 1024 * 1024;

/// Public English manga. The caller owns the runtime and request cancellation.
pub struct WeebCentral {
    http: SourceHttp,
    origin: Url,
}

impl WeebCentral {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Weeb Central origin");
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
        let mut url = self.origin.join("/search/data").expect("fixed search path");
        url.query_pairs_mut()
            .append_pair("author", "")
            .append_pair("text", query)
            .append_pair("sort", "Best Match")
            .append_pair("order", "Descending")
            .append_pair("official", "Any")
            .append_pair("anime", "Any")
            .append_pair("adult", "Any")
            .append_pair("display_mode", "Full Display");
        response::search_page(&self.page(url).await?, &self.origin)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        publication.validate()?;
        if publication.source != Source::WeebCentral {
            return Err(Error::InvalidReference);
        }
        let url = self
            .origin
            .join(&format!("/series/{}", publication.id))
            .unwrap();
        response::work_page(&self.page(url).await?, &self.origin, publication)
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        self.inspect(&selected.publication).await?;
        let url = self
            .origin
            .join(&format!(
                "/series/{}/full-chapter-list",
                selected.publication.id
            ))
            .unwrap();
        let (final_url, page) = self
            .http
            .html_page_with_limit(url.clone(), MAX_CHAPTER_LIST_BYTES)
            .await?;
        if final_url != url {
            return Err(Error::VariantUnavailable);
        }
        response::chapter_list(&page, &self.origin, selected)
    }

    pub async fn chapter_pages(&self, selected: &ChapterRef) -> Result<ChapterPages> {
        selected.validate()?;
        self.inspect(&selected.variant.publication).await?;
        let reader = self
            .origin
            .join(&format!("/chapters/{}", selected.id))
            .unwrap();
        response::reader_identity(&self.page(reader).await?, &self.origin, selected)?;
        let mut url = self
            .origin
            .join(&format!("/chapters/{}/images", selected.id))
            .unwrap();
        url.query_pairs_mut()
            .append_pair("is_prev", "False")
            .append_pair("reading_style", "long_strip");
        response::image_list(&self.page(url).await?, selected)
    }

    pub async fn image_bytes(&self, image: &ChapterImage) -> Result<Vec<u8>> {
        response::validate_image(&image.url)?;
        self.http.image(image.url.clone(), IMAGE_HOSTS).await
    }

    async fn page(&self, url: Url) -> Result<String> {
        let (final_url, page) = self.http.html_page(url.clone()).await?;
        if final_url != url {
            return Err(Error::VariantUnavailable);
        }
        Ok(page)
    }
}
