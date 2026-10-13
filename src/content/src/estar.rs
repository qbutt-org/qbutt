#[path = "estar/response.rs"]
mod response;

use std::collections::HashMap;

use reqwest::header::HeaderMap;
use sha2::{Digest, Sha256};
use url::Url;

use crate::http::SourceHttp;
use crate::{
    Chapter, ChapterRef, Error, PublicationRef, Result, TextChapter, TextImageAsset, VariantRef,
    Work,
};

const ORIGIN: &str = "https://estar.jp";
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHAPTER_IMAGE_BYTES: usize = 24 * 1024 * 1024;

/// Public Japanese novels on エブリスタ. The caller owns runtime and cancellation.
pub struct Estar {
    http: SourceHttp,
    origin: Url,
}

impl Estar {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Estar origin");
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

        let mut url = self.origin.join("/novels").expect("fixed search path");
        url.query_pairs_mut()
            .append_pair("awarded", "0")
            .append_pair("characterLengthOption", "undefined")
            .append_pair("ex_paused", "0")
            .append_pair("ex_r18", "0")
            .append_pair("ex_ticket", "1")
            .append_pair("finished", "0")
            .append_pair("in_selection", "0")
            .append_pair("is_book_p", "0")
            .append_pair("keyword", query)
            .append_pair("sort", "score_desc")
            .append_pair("tag_only", "0")
            .append_pair("writing", "0");
        response::search(&self.page(url).await?, query)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.book(publication).await?.work)
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        let book = self.book(&selected.publication).await?;
        Ok(self
            .listing(&book)
            .await?
            .into_iter()
            .map(|entry| entry.chapter)
            .collect())
    }

    pub async fn chapter_text(&self, selected: &ChapterRef) -> Result<TextChapter> {
        selected.validate()?;
        let book = self.book(&selected.variant.publication).await?;
        let entry = self
            .listing(&book)
            .await?
            .into_iter()
            .find(|entry| entry.chapter.reference == *selected)
            .ok_or(Error::VariantUnavailable)?;

        let mut blocks = Vec::new();
        let mut assets = HashMap::new();
        let mut image_bytes = 0;
        for page in entry.start..entry.end {
            let mut url = self
                .origin
                .join(&format!("/novels/{}/viewer", book.work.reference.id))
                .expect("canonical Estar viewer path");
            url.query_pairs_mut().append_pair("page", &page.to_string());
            let document = self.page(url).await?;
            let body = response::viewer(&document, &book.work.reference, &entry.chapter, page)?;

            let mut images = HashMap::new();
            for source in &body.images {
                if images.contains_key(source) {
                    continue;
                }
                let url = self
                    .origin
                    .join(source)
                    .map_err(|_| Error::InvalidResponse("invalid Estar chapter image URL"))?;
                if url.scheme() != "https" || url.host_str() != Some("img.estar.jp") {
                    return Err(Error::InvalidResponse(
                        "unsupported Estar chapter image host",
                    ));
                }
                let bytes = self.http.image(url, &["img.estar.jp"]).await?;
                image_bytes += bytes.len();
                if bytes.len() > MAX_IMAGE_BYTES || image_bytes > MAX_CHAPTER_IMAGE_BYTES {
                    return Err(Error::ResponseTooLarge);
                }
                let media_type = image_media_type(&bytes)
                    .ok_or(Error::InvalidResponse("invalid Estar chapter image"))?;
                let id = format!("{:x}", Sha256::digest(&bytes));
                images.insert(
                    source.clone(),
                    TextImageAsset {
                        id,
                        media_type: media_type.into(),
                        bytes: bytes.into(),
                    },
                );
            }

            let mut resolve_image = |source: &str| -> Result<Option<TextImageAsset>> {
                images
                    .get(source)
                    .cloned()
                    .map(Some)
                    .ok_or(Error::InvalidResponse("unlisted Estar chapter image"))
            };
            let chapter = TextChapter::from_local_html(
                selected.clone(),
                &body.html,
                &mut resolve_image,
                None,
                None,
            )?;
            blocks.extend(chapter.blocks);
            for asset in chapter.assets {
                assets.entry(asset.id.clone()).or_insert(asset);
            }
        }

        TextChapter::from_blocks_with_assets(
            selected.clone(),
            blocks,
            assets.into_values().collect(),
        )
    }

    async fn book(&self, publication: &PublicationRef) -> Result<response::Book> {
        let id = publication.estar_id()?;
        let url = self
            .origin
            .join(&format!("/novels/{id}"))
            .expect("canonical Estar work path");
        response::book(&self.page(url).await?, publication)
    }

    async fn listing(&self, book: &response::Book) -> Result<Vec<response::Entry>> {
        let mut pages = Vec::new();
        let total = book.episode_count.div_ceil(30);
        for number in 1..=total {
            let mut url = self
                .origin
                .join(&format!("/novels/{}/episodes", book.work.reference.id))
                .expect("canonical Estar episode path");
            url.query_pairs_mut()
                .append_pair("page", &number.to_string());
            pages.push(response::episodes(
                &self.page(url).await?,
                &book.work.reference,
                number,
            )?);
        }
        response::listing(book, pages)
    }

    async fn page(&self, requested: Url) -> Result<String> {
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

fn image_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Some("image/webp")
    } else {
        None
    }
}
