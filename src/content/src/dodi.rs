use url::Url;

use crate::http::{SourceHttp, SourceSession};
use crate::{Error, PublicationRef, ResolvedContent, Result, VariantRef, VariantSelector, Work};

#[path = "dodi/host.rs"]
mod host;

#[path = "dodi/response.rs"]
pub(crate) mod response;

const ORIGIN: &str = "https://dodi-repacks.site";

/// DODI's publication catalog. Host landing pages are not torrent acquisitions.
pub struct Dodi {
    http: SourceHttp,
    origin: Url,
}

impl Dodi {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed DODI origin");
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
        let mut url = self.origin.clone();
        url.query_pairs_mut().append_pair("s", query);
        let body = self.http.html(url).await?;
        response::search(&body, &self.origin, query)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        let (_, slug) = publication.dodi_parts()?;
        let url = self
            .origin
            .join(&format!("/{slug}/"))
            .expect("validated publication slug");
        let (arrived, body) = match self.http.html_page(url.clone()).await {
            Err(Error::HttpStatus(404 | 410)) => return Err(Error::VariantUnavailable),
            result => result?,
        };
        if arrived != url {
            return Err(Error::VariantUnavailable);
        }
        response::release(&body, &self.origin, publication)
    }

    /// Re-fetch the publication before requesting its exact torrent mirror.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        tokio::time::timeout(std::time::Duration::from_secs(45), async {
            let work = self.inspect(&selected.publication).await?;
            if !work
                .variants
                .iter()
                .any(|variant| variant.reference == *selected)
            {
                return Err(Error::VariantUnavailable);
            }
            let VariantSelector::DodiTorrent { id } = &selected.selector else {
                return Err(Error::InvalidReference);
            };
            let origin = Url::parse("https://file-me.top").expect("fixed torrent host origin");
            let url = origin
                .join(&format!("/{id}.html"))
                .expect("validated torrent host ID");
            let http = SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?;
            let mut session = SourceSession::anonymous(origin)?;
            let metadata = host::download(&http, url, &mut session, id).await?;
            Ok(ResolvedContent::TorrentFile {
                reference: selected.clone(),
                metadata,
            })
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}
