// SPDX-License-Identifier: GPL-3.0-or-later
#[path = "nhentai/response.rs"]
mod response;

use reqwest::header::{HeaderMap, HeaderValue, REFERER};
use url::Url;

use crate::http::SourceHttp;
use crate::{
    Chapter, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result, Source,
    VariantRef, Work,
};

/// Public gallery API. Source settings require an explicit opt-in to this adult catalogue.
/// The caller owns the runtime and cancellation of each borrowed operation.
pub struct Nhentai {
    http: SourceHttp,
    origin: Url,
}

impl Nhentai {
    pub fn new() -> Result<Self> {
        let origin = Url::parse("https://nhentai.net/").expect("fixed nhentai origin");
        let mut headers = HeaderMap::new();
        headers.insert(REFERER, HeaderValue::from_static("https://nhentai.net/"));
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
        let mut url = self.origin.join("/api/v2/search").unwrap();
        url.query_pairs_mut()
            .append_pair("query", query)
            .append_pair("page", "1");
        let (search, cdn) = tokio::try_join!(self.http.json::<response::Search>(url), self.cdn(),)?;
        response::search(search, &cdn, &self.origin)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        let (gallery, cdn) = tokio::try_join!(self.gallery(publication), self.cdn())?;
        gallery.work(&cdn, &self.origin)
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        let gallery = self.gallery(&selected.publication).await?;
        Ok(vec![gallery.chapter(selected)])
    }

    pub async fn chapter_pages(&self, selected: &ChapterRef) -> Result<ChapterPages> {
        selected.validate()?;
        let (gallery, cdn) =
            tokio::try_join!(self.gallery(&selected.variant.publication), self.cdn(),)?;
        gallery.pages(selected, &cdn)
    }

    pub async fn image_bytes(&self, image: &ChapterImage) -> Result<Vec<u8>> {
        response::validate_image(&image.url)?;
        let host = image.url.host_str().ok_or(Error::UnsafeAddress)?;
        self.http.image(image.url.clone(), &[host]).await
    }

    async fn gallery(&self, publication: &PublicationRef) -> Result<response::Gallery> {
        publication.validate()?;
        if publication.source != Source::Nhentai {
            return Err(Error::InvalidReference);
        }
        let url = self
            .origin
            .join(&format!("/api/v2/galleries/{}", publication.id))
            .unwrap();
        let gallery: response::Gallery = self.http.json(url).await?;
        gallery.validate(publication)?;
        Ok(gallery)
    }

    async fn cdn(&self) -> Result<response::Cdn> {
        let cdn: response::Cdn = self
            .http
            .json(self.origin.join("/api/v2/cdn").unwrap())
            .await?;
        cdn.validate()?;
        Ok(cdn)
    }
}
