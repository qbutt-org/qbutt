#[path = "syosetu/response.rs"]
mod response;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::header::HeaderMap;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use url::Url;

use crate::http::SourceHttp;
use crate::{
    Chapter, ChapterRef, Error, PublicationRef, Result, TextChapter, TextImageAsset, VariantRef,
    Work,
};

const SEARCH_ORIGIN: &str = "https://yomou.syosetu.com";
const CACHE_AGE: Duration = Duration::from_secs(5 * 60);
const CACHE_BOOKS: usize = 8;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHAPTER_IMAGE_BYTES: usize = 24 * 1024 * 1024;
const IMAGE_CDN: &str = "img1.mitemin.net";

/// Public Japanese works. Dropping an operation cancels its bounded HTTP work.
pub struct Syosetu {
    search_http: SourceHttp,
    reader_http: SourceHttp,
    requests: Semaphore,
    books: Mutex<VecDeque<(PublicationRef, Instant, Arc<response::Book>)>>,
}

impl Syosetu {
    pub fn new() -> Result<Self> {
        let search = Url::parse(SEARCH_ORIGIN).expect("fixed Syosetu search origin");
        let reader = Url::parse(response::READER_ORIGIN).expect("fixed Syosetu reader origin");
        Ok(Self {
            search_http: SourceHttp::new(search, HeaderMap::new())?,
            reader_http: SourceHttp::new(reader, HeaderMap::new())?,
            requests: Semaphore::new(2),
            books: Mutex::new(VecDeque::new()),
        })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let _permit = self.requests.acquire().await.map_err(|_| Error::Timeout)?;
        let mut url = Url::parse(SEARCH_ORIGIN)
            .expect("fixed Syosetu search origin")
            .join("/search.php")
            .expect("fixed Syosetu search path");
        url.query_pairs_mut().append_pair("word", query);
        response::search(&self.search_http.html(url).await?)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.book(publication).await?.work.clone())
    }

    pub async fn chapters(&self, selected: &VariantRef) -> Result<Vec<Chapter>> {
        selected.validate()?;
        Ok(self.book(&selected.publication).await?.chapters.clone())
    }

    pub async fn chapter_text(&self, selected: &ChapterRef) -> Result<TextChapter> {
        selected.validate()?;
        selected.variant.publication.syosetu_id()?;
        let _permit = self.requests.acquire().await.map_err(|_| Error::Timeout)?;
        tokio::time::timeout(Duration::from_secs(90), async {
            let url = response::chapter_url(selected);
            let body = self.page(url).await?;
            let chapter = response::chapter(&body, selected)?;
            let mut images = HashMap::new();
            let mut total_bytes = 0;
            for url in &chapter.images {
                let host = url.host_str().expect("validated Mitemin image host");
                let bytes = self
                    .reader_http
                    .image(url.clone(), &[host, IMAGE_CDN])
                    .await?;
                total_bytes += bytes.len();
                if bytes.len() > MAX_IMAGE_BYTES || total_bytes > MAX_CHAPTER_IMAGE_BYTES {
                    return Err(Error::ResponseTooLarge);
                }

                let media_type = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                    "image/png"
                } else if bytes.starts_with(b"\xff\xd8\xff") {
                    "image/jpeg"
                } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
                    "image/gif"
                } else {
                    return Err(Error::InvalidResponse("unsupported Syosetu chapter image"));
                };
                images.insert(
                    url.clone(),
                    TextImageAsset {
                        id: format!("{:x}", Sha256::digest(&bytes)),
                        media_type: media_type.into(),
                        bytes: bytes.into(),
                    },
                );
            }
            chapter.into_text(images)
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    async fn book(&self, publication: &PublicationRef) -> Result<Arc<response::Book>> {
        publication.syosetu_id()?;
        if let Some(book) = self
            .books
            .lock()
            .expect("Syosetu book cache lock")
            .iter()
            .find(|(reference, saved, _)| reference == publication && saved.elapsed() < CACHE_AGE)
            .map(|(_, _, book)| Arc::clone(book))
        {
            return Ok(book);
        }

        let _permit = self.requests.acquire().await.map_err(|_| Error::Timeout)?;
        let url = response::publication_url(publication);
        let first = response::book_page(&self.page(url.clone()).await?, publication, 1, None)?;
        let page_count = first.total_pages;
        let mut pages = vec![first];
        for page in 2..=page_count {
            let mut url = url.clone();
            url.query_pairs_mut().append_pair("p", &page.to_string());
            tokio::time::sleep(Duration::from_millis(150)).await;
            pages.push(response::book_page(
                &self.page(url).await?,
                publication,
                page,
                Some(page_count),
            )?);
        }
        let book = Arc::new(response::join_pages(pages)?);
        let mut cache = self.books.lock().expect("Syosetu book cache lock");
        cache.retain(|(reference, saved, _)| {
            reference != publication && saved.elapsed() < CACHE_AGE
        });
        if cache.len() == CACHE_BOOKS {
            cache.pop_front();
        }
        cache.push_back((publication.clone(), Instant::now(), Arc::clone(&book)));
        Ok(book)
    }

    async fn page(&self, requested: Url) -> Result<String> {
        let (url, body) = match self.reader_http.html_page(requested.clone()).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        if url != requested {
            return Err(Error::VariantUnavailable);
        }
        Ok(body)
    }
}
