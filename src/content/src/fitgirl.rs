use std::time::Duration;

use url::Url;

use crate::http::SourceHttp;
use crate::{Category, Error, PublicationRef, ResolvedContent, Result, VariantRef, Work};

#[path = "fitgirl/privatebin.rs"]
mod privatebin;
#[path = "fitgirl/response.rs"]
pub(crate) mod response;

const ORIGIN: &str = "https://fitgirl-repacks.site";

/// FitGirl's official public catalog and exact torrent metadata attachments.
pub struct FitGirl {
    http: SourceHttp,
    origin: Url,
    torrent_pastes: Vec<SourceHttp>,
}

impl FitGirl {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed FitGirl origin");
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "X-Requested-With",
            reqwest::header::HeaderValue::from_static("JSONHttpRequest"),
        );
        let torrent_pastes = privatebin::HOSTS
            .iter()
            .map(|host| {
                let origin = Url::parse(&format!("https://{host}"))
                    .expect("fixed public torrent paste origin");
                SourceHttp::new(origin, headers.clone())
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            http: SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?,
            origin,
            torrent_pastes,
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
        let release = self.release(&selected.publication).await?;
        let variant = release
            .work
            .variants
            .iter()
            .find(|variant| variant.reference == *selected)
            .ok_or(Error::VariantUnavailable)?;
        let crate::VariantSelector::TorrentHash { info_hash } = &selected.selector else {
            return Err(Error::InvalidReference);
        };
        let magnet = release
            .magnets
            .get(info_hash)
            .ok_or(Error::VariantUnavailable)?;

        if let Some(paste) = release.torrent_files.get(info_hash) {
            let host = privatebin::HOSTS
                .iter()
                .position(|host| Some(*host) == paste.host_str())
                .expect("validated public torrent paste host");
            let attachment = tokio::time::timeout(
                Duration::from_secs(10),
                privatebin::download(&self.torrent_pastes[host], paste),
            )
            .await
            .unwrap_or(Err(Error::Timeout));
            match attachment {
                Ok(metadata) => {
                    if crate::torrent::info_hash(metadata.bytes())? != *info_hash {
                        return Err(Error::InvalidResponse("FitGirl torrent hash changed"));
                    }
                    return Ok(ResolvedContent::TorrentFile {
                        reference: selected.clone(),
                        metadata,
                    });
                }
                // The already published magnet identifies the same payload.
                // An unavailable attachment must not disable that working path.
                Err(
                    Error::Timeout
                    | Error::Network(_)
                    | Error::Challenge
                    | Error::VariantUnavailable,
                )
                | Err(Error::HttpStatus(403 | 404 | 410 | 429 | 500..=599)) => {}
                Err(error) => return Err(error),
            }
        }

        Ok(ResolvedContent::Torrent {
            reference: selected.clone(),
            title: variant.title.clone().unwrap_or(release.work.title),
            magnet: magnet.clone(),
        })
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
