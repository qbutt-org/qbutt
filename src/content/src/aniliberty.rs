mod response;

use std::collections::HashSet;

use url::Url;

use crate::http::{SourceHttp, public_media_url};
use crate::{Error, PublicationRef, ResolvedContent, Result, VariantRef, VariantSelector, Work};

use response::Release;

const ORIGIN: &str = "https://anilibria.top";

/// AniLiberty's public catalog. Construction creates no process or task.
pub struct AniLiberty {
    http: SourceHttp,
    origin: Url,
}

impl AniLiberty {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed AniLiberty origin");
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
        url.set_path("/api/v1/app/search/releases");
        url.query_pairs_mut().append_pair("query", query);
        let releases: Vec<Release> = self.http.json(url).await?;
        if releases.len() > 500 {
            return Err(Error::InvalidResponse("too many search results"));
        }
        let mut seen = HashSet::new();
        releases
            .into_iter()
            .filter(|release| seen.insert(release.id))
            .map(|release| release.into_work(&self.origin, true))
            .collect()
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        self.release(publication)
            .await?
            .into_work(&self.origin, false)
    }

    /// Refetch the publication and resolve this exact selector. Missing variants
    /// fail rather than falling back to a different episode, quality or torrent.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        let release = self.release(&selected.publication).await?;
        match &selected.selector {
            VariantSelector::Torrent { id } => {
                let torrent = release
                    .torrents
                    .iter()
                    .flatten()
                    .find(|torrent| torrent.id == *id)
                    .ok_or(Error::VariantUnavailable)?;
                let magnet = torrent.magnet()?;
                Ok(ResolvedContent::Torrent {
                    reference: selected.clone(),
                    title: release.name.main,
                    magnet,
                })
            }
            VariantSelector::Episode { id, quality } => {
                let episode = release
                    .episodes
                    .iter()
                    .flatten()
                    .find(|episode| episode.id == *id)
                    .ok_or(Error::VariantUnavailable)?;
                let stream = episode.stream(*quality).ok_or(Error::VariantUnavailable)?;
                let url = public_media_url(stream).await?;
                Ok(ResolvedContent::DirectVideo {
                    reference: selected.clone(),
                    title: release.name.main,
                    url,
                    quality: *quality,
                })
            }
            VariantSelector::Rutube { episode_id } => {
                let id = release
                    .episodes
                    .iter()
                    .flatten()
                    .find(|episode| episode.id == *episode_id)
                    .and_then(|episode| episode.rutube_id.as_deref())
                    .filter(|id| {
                        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    })
                    .ok_or(Error::VariantUnavailable)?;
                let url = public_media_url(&format!("https://rutube.ru/video/{id}/")).await?;
                Ok(ResolvedContent::ExternalVideo {
                    reference: selected.clone(),
                    title: release.name.main,
                    url,
                })
            }
            VariantSelector::MangaTranslation { .. }
            | VariantSelector::LibTranslation { .. }
            | VariantSelector::TorrentHash { .. } => Err(Error::InvalidReference),
        }
    }

    async fn release(&self, publication: &PublicationRef) -> Result<Release> {
        let id = publication.aniliberty_id()?;

        let mut url = self.origin.clone();
        url.set_path(&format!("/api/v1/anime/releases/{id}"));
        let release: Release = self.http.json(url).await?;
        if release.id != id {
            return Err(Error::InvalidResponse("publication identity changed"));
        }
        release.validate()?;
        Ok(release)
    }
}
