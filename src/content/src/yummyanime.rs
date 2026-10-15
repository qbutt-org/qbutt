#[path = "yummyanime/aksor.rs"]
mod aksor;
#[path = "yummyanime/cvh.rs"]
mod cvh;
#[path = "yummyanime/response.rs"]
mod response;

use std::collections::HashSet;
use std::time::Duration;

use tokio::time::{Instant, timeout_at};
use url::Url;

use crate::http::{SourceHttp, public_media_url};
use crate::{Error, PublicationRef, ResolvedContent, Result, VariantRef, VariantSelector, Work};

use crate::kodik::{self, KodikVideo};
use response::{Anime, Envelope};

const ORIGIN: &str = "https://yani.tv";
const API_ORIGIN: &str = "https://api.yani.tv";
const SEARCH_ALIAS_LIMIT: usize = 4;
const SEARCH_ALIAS_BUDGET: Duration = Duration::from_secs(3);

/// Anonymous public catalog access. Account-specific content is not requested.
pub struct YummyAnime {
    http: SourceHttp,
    origin: Url,
    api_origin: Url,
}

impl YummyAnime {
    pub fn new() -> Result<Self> {
        let api_origin = Url::parse(API_ORIGIN).expect("fixed YummyAnime API origin");
        Ok(Self {
            http: SourceHttp::new(api_origin.clone(), reqwest::header::HeaderMap::new())?,
            origin: Url::parse(ORIGIN).expect("fixed YummyAnime origin"),
            api_origin,
        })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Work>> {
        self.search_with_updates(query, |_| {}).await
    }

    /// Publish search rows before bounded, optional catalogue alias requests.
    /// Metadata failure preserves successful rows; variants still need inspection.
    /// The caller owns this future, its cancellation and publication generation.
    pub async fn search_with_updates(
        &self,
        query: &str,
        mut publish: impl FnMut(&[Work]),
    ) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let mut url = self.api_origin.join("anime").expect("fixed API path");
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("limit", "40");
        let response: Envelope<Vec<Anime>> = self.http.json(url).await?;
        if response.response.len() > 40 {
            return Err(Error::InvalidResponse("too many search results"));
        }
        let mut seen = HashSet::new();
        let mut works: Vec<Work> = response
            .response
            .into_iter()
            .filter(|anime| seen.insert(anime.anime_id))
            .map(|anime| anime.into_work(&self.origin, true))
            .collect::<Result<_>>()?;
        publish(&works);

        let deadline = Instant::now() + SEARCH_ALIAS_BUDGET;
        for work in works
            .iter_mut()
            .filter(|work| work.aliases.is_empty() && crate::query::matches(query, work))
            .take(SEARCH_ALIAS_LIMIT)
        {
            if Instant::now() >= deadline {
                break;
            }
            let metadata = timeout_at(deadline, self.anime(&work.reference, false)).await;
            let Ok(metadata) = metadata else {
                break;
            };
            let Ok(metadata) = metadata.and_then(|anime| anime.into_work(&self.origin, true))
            else {
                continue;
            };
            if metadata.aliases.is_empty()
                || metadata.title != work.title
                || matches!((work.year, metadata.year), (Some(a), Some(b)) if a != b)
                || work.identifiers.iter().any(|(name, value)| {
                    metadata
                        .identifiers
                        .get(name)
                        .is_some_and(|other| other != value)
                })
            {
                continue;
            }
            work.aliases = metadata.aliases;
            publish(std::slice::from_ref(work));
        }
        Ok(works)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        self.anime(publication, true)
            .await?
            .into_work(&self.origin, false)
    }

    /// Refetch the exact video and its raw episode, dubbing and player identity.
    /// Acquisition addresses remain private to this fresh resolution.
    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        let VariantSelector::YummyAnimeVideo { id, .. } = &selected.selector else {
            return Err(Error::InvalidReference);
        };
        let anime = self.anime(&selected.publication, true).await?;
        anime.validate(true)?;
        let video = anime
            .videos
            .as_ref()
            .expect("validated video list")
            .iter()
            .find(|video| video.video_id == *id)
            .filter(|video| video.selector() == selected.selector)
            .ok_or(Error::VariantUnavailable)?;
        let player = self
            .origin
            .join(&video.iframe_url)
            .map_err(|_| Error::UnsafeAddress)?;
        crate::http::validate_https_url(&player)?;

        let (url, quality) = match video.player() {
            "Aksor" => aksor::resolve(&anime, video, &player).await?,
            "CVH" => cvh::resolve(&anime, video, &player).await?,
            "Kodik" => {
                let dubbing = video
                    .data
                    .dubbing
                    .strip_prefix("Озвучка ")
                    .or_else(|| video.data.dubbing.strip_prefix("Субтитры "))
                    .unwrap_or(&video.data.dubbing);
                kodik::resolve(
                    KodikVideo {
                        episode: &video.number,
                        dubbing,
                        subtitled: video.is_subtitled(),
                        video: None,
                    },
                    &player,
                )
                .await?
            }
            _ => {
                return Ok(ResolvedContent::ExternalVideo {
                    reference: selected.clone(),
                    source_url: anime.publication_url(&self.origin),
                    title: anime.title,
                    url: public_media_url(player.as_str()).await?,
                });
            }
        };
        Ok(ResolvedContent::DirectVideo {
            reference: selected.clone(),
            title: anime.title,
            url,
            quality,
        })
    }

    async fn anime(&self, publication: &PublicationRef, need_videos: bool) -> Result<Anime> {
        let id = publication.yummyanime_id()?;
        let mut url = self
            .api_origin
            .join(&format!("anime/{id}"))
            .expect("numeric API path");
        url.query_pairs_mut()
            .append_pair("need_videos", if need_videos { "true" } else { "false" });
        let response: Envelope<Anime> = self.http.json(url).await?;
        if response.response.anime_id != id {
            return Err(Error::InvalidResponse("publication identity changed"));
        }
        Ok(response.response)
    }
}
