mod response;

use reqwest::header::HeaderMap;
use url::Url;

use crate::http::SourceHttp;
use crate::kodik::{self, KodikVideo};
use crate::{Error, PublicationRef, ResolvedContent, Result, VariantRef, VariantSelector, Work};

const ORIGIN: &str = "https://anitokyo.tv";

/// AniTokyo's public catalog and its exact episode/voice selection.
pub struct AniTokyo {
    http: SourceHttp,
    origin: Url,
}

impl AniTokyo {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed AniTokyo origin");
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

        let mut url = self.origin.join("index.php").expect("fixed search path");
        url.query_pairs_mut()
            .append_pair("do", "search")
            .append_pair("subaction", "search")
            .append_pair("story", query);
        let (final_url, html) = self.http.html_page(url.clone()).await?;
        if final_url != url {
            return Err(Error::InvalidResponse("AniTokyo search changed location"));
        }
        response::search(&html, &self.origin, query)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        Ok(self.publication(publication).await?.work)
    }

    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        let VariantSelector::AniTokyoVideo { .. } = &selected.selector else {
            return Err(Error::InvalidReference);
        };

        let publication = self.publication(&selected.publication).await?;
        let video = publication
            .videos
            .iter()
            .find(|video| video.selector == selected.selector)
            .ok_or(Error::VariantUnavailable)?;

        let html = self.http.html(video.url.clone()).await?;
        let player = response::player(&html, &self.origin)?;
        if player.host_str() != Some("kodikplayer.com") {
            return Err(Error::UnsupportedAcquisition(
                "AniTokyo player is not supported",
            ));
        }

        let (url, quality) = kodik::resolve(
            KodikVideo {
                episode: &video.episode,
                dubbing: &video.kodik_voice,
                subtitled: video.subtitled,
                video: None,
            },
            &player,
        )
        .await?;
        Ok(ResolvedContent::DirectVideo {
            reference: selected.clone(),
            title: publication.work.title,
            url,
            quality,
        })
    }

    async fn publication(&self, reference: &PublicationRef) -> Result<response::Publication> {
        let id = reference.anitokyo_id()?;
        let mut url = self
            .origin
            .join("index.php")
            .expect("fixed publication path");
        url.query_pairs_mut().append_pair("newsid", &id.to_string());
        let (final_url, html) = self.http.html_page(url).await?;
        response::publication(&html, &self.origin, &final_url, reference)
    }
}
