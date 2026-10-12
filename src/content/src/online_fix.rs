use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use url::Url;

use crate::http::{SourceHttp, SourceSession};
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, TorrentMetadata, VariantRef, Work,
};

#[path = "online_fix/response.rs"]
mod response;

const ORIGIN: &str = "https://online-fix.me";
const UPLOADS_ORIGIN: &str = "https://uploads.online-fix.me:2053";
const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0 Safari/537.36";

/// Public game articles and their advertised torrent directory.
pub struct OnlineFix {
    site: SourceHttp,
    uploads: SourceHttp,
    origin: Url,
    uploads_origin: Url,
}

impl OnlineFix {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed Online-Fix origin");
        let uploads_origin = Url::parse(UPLOADS_ORIGIN).expect("fixed Online-Fix uploads origin");
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
        Ok(Self {
            site: SourceHttp::new(origin.clone(), headers.clone())?,
            uploads: SourceHttp::new(uploads_origin.clone(), headers)?,
            origin,
            uploads_origin,
        })
    }

    pub async fn search(&self, query: &str, category: Category) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        if category != Category::Games || query.chars().count() < 3 {
            return Ok(Vec::new());
        }

        let mut url = self.origin.join("/index.php").expect("fixed search route");
        url.query_pairs_mut().extend_pairs([
            ("do", "search"),
            ("subaction", "search"),
            ("story", query),
        ]);
        let body = self.site_document(url).await?;
        response::search(&body, &self.origin, query)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.release(publication).await?.work)
    }

    /// A regenerated torrent at the same path cannot replace an exact selection.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        self.release(&selected.publication).await?.resolve(selected)
    }

    async fn release(&self, publication: &PublicationRef) -> Result<response::Release> {
        let path = publication.online_fix_path()?;
        let url = self
            .origin
            .join(path)
            .map_err(|_| Error::InvalidReference)?;
        let body = match self.site_document(url.clone()).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        let article =
            response::article(&body, &self.origin, &self.uploads_origin, &url, publication)?;
        let mut session = SourceSession::anonymous(self.uploads_origin.clone())?;
        let mut release = response::Release::new(article.work);
        for directory_url in article.torrent_directories {
            let directory = self
                .uploads
                .session_document(directory_url.clone(), &mut session)
                .await?;
            let files = response::directory(directory, &directory_url, &self.uploads_origin)?;
            for (url, name) in files {
                let metadata = self.torrent_metadata(url, &mut session).await?;
                release.add_torrent(name, metadata)?;
            }
        }
        Ok(release)
    }

    async fn torrent_metadata(
        &self,
        url: Url,
        session: &mut SourceSession,
    ) -> Result<TorrentMetadata> {
        let response = self
            .uploads
            .session_metadata(url.clone(), session, None)
            .await?;
        if response.final_url != url || !(200..300).contains(&response.status) {
            return Err(Error::VariantUnavailable);
        }
        if !response.content_type.as_deref().is_some_and(|value| {
            matches!(
                value.split(';').next().map(str::trim),
                Some("application/octet-stream" | "application/x-bittorrent")
            )
        }) {
            return Err(Error::InvalidResponse("Online-Fix torrent is not metadata"));
        }
        TorrentMetadata::new(response.body)
    }

    async fn site_document(&self, url: Url) -> Result<String> {
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let response = self
            .site
            .session_document(url.clone(), &mut session)
            .await?;
        response::site_document(response, &url)
    }
}
