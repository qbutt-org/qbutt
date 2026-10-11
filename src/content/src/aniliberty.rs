use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use url::Url;

use crate::http::{SourceHttp, public_media_url};
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, Transport, Variant,
    VariantRef, VariantSelector, VideoQuality, Work,
};

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
            http: SourceHttp::new(origin.clone())?,
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
        }
    }

    async fn release(&self, publication: &PublicationRef) -> Result<Release> {
        if publication.source != Source::AniLiberty {
            return Err(Error::InvalidReference);
        }
        let id = publication
            .id
            .parse::<u64>()
            .map_err(|_| Error::InvalidReference)?;
        if id == 0 || publication.id != id.to_string() {
            return Err(Error::InvalidReference);
        }

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

#[derive(Deserialize)]
struct Release {
    id: u64,
    alias: String,
    name: ReleaseName,
    description: Option<String>,
    year: Option<u16>,
    poster: Option<Poster>,
    mal: Option<ExternalId>,
    shikimori: Option<ExternalId>,
    episodes: Option<Vec<Episode>>,
    torrents: Option<Vec<Torrent>>,
}

#[derive(Deserialize)]
struct ReleaseName {
    main: String,
    english: Option<String>,
    alternative: Option<String>,
}

#[derive(Deserialize)]
struct Poster {
    src: Option<String>,
}

#[derive(Deserialize)]
struct ExternalId {
    id: Option<u64>,
}

#[derive(Deserialize)]
struct Episode {
    id: String,
    ordinal: f64,
    hls_480: Option<String>,
    hls_720: Option<String>,
    hls_1080: Option<String>,
    rutube_id: Option<String>,
}

impl Episode {
    fn stream(&self, quality: VideoQuality) -> Option<&str> {
        let stream = match quality {
            VideoQuality::P480 => &self.hls_480,
            VideoQuality::P720 => &self.hls_720,
            VideoQuality::P1080 => &self.hls_1080,
        };
        stream.as_deref().filter(|value| !value.is_empty())
    }
}

#[derive(Deserialize)]
struct Torrent {
    id: u64,
    label: String,
    magnet: String,
    hash: String,
    size: Option<u64>,
    seeders: Option<u64>,
    quality: Option<TorrentQuality>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct TorrentQuality {
    value: String,
}

impl Torrent {
    fn magnet(&self) -> Result<Url> {
        let magnet = Url::parse(&self.magnet)
            .map_err(|_| Error::InvalidResponse("invalid torrent magnet"))?;
        let identity = format!("urn:btih:{}", self.hash);
        let mut identifiers = magnet.query_pairs().filter(|(name, _)| name == "xt");
        let matches_hash = identifiers
            .next()
            .is_some_and(|(_, value)| value.eq_ignore_ascii_case(&identity));
        if magnet.scheme() != "magnet"
            || magnet.host().is_some()
            || !magnet.path().is_empty()
            || self.magnet.len() > 32 * 1024
            || self.hash.len() != 40
            || !self.hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !matches_hash
            || identifiers.next().is_some()
        {
            return Err(Error::InvalidResponse(
                "torrent magnet does not match its hash",
            ));
        }
        Ok(magnet)
    }
}

impl Release {
    fn validate(&self) -> Result<()> {
        if self.id == 0 || self.name.main.trim().is_empty() || self.alias.trim().is_empty() {
            return Err(Error::InvalidResponse(
                "missing publication identity or title",
            ));
        }
        if self.name.main.len() > 2048
            || self.alias.len() > 256
            || self
                .name
                .english
                .as_ref()
                .is_some_and(|name| name.len() > 2048)
            || self
                .name
                .alternative
                .as_ref()
                .is_some_and(|name| name.len() > 2048)
            || self
                .description
                .as_ref()
                .is_some_and(|text| text.len() > 128 * 1024)
            || self
                .episodes
                .as_ref()
                .is_some_and(|episodes| episodes.len() > 5000)
            || self
                .torrents
                .as_ref()
                .is_some_and(|torrents| torrents.len() > 1000)
        {
            return Err(Error::InvalidResponse(
                "publication metadata exceeds its limits",
            ));
        }

        let mut episode_ids = HashSet::new();
        for episode in self.episodes.iter().flatten() {
            if episode.id.is_empty()
                || episode.id.len() > 128
                || !episode.ordinal.is_finite()
                || episode.ordinal < 0.0
                || !episode_ids.insert(&episode.id)
            {
                return Err(Error::InvalidResponse(
                    "invalid or duplicate episode identity",
                ));
            }
        }
        let mut torrent_ids = HashSet::new();
        for torrent in self.torrents.iter().flatten() {
            if torrent.id == 0 || !torrent_ids.insert(torrent.id) {
                return Err(Error::InvalidResponse(
                    "invalid or duplicate torrent identity",
                ));
            }
        }
        Ok(())
    }

    fn into_work(self, origin: &Url, needs_inspection: bool) -> Result<Work> {
        self.validate()?;
        let reference = PublicationRef {
            source: Source::AniLiberty,
            id: self.id.to_string(),
        };
        let mut url = origin.clone();
        url.path_segments_mut()
            .expect("fixed HTTPS origin")
            .clear()
            .extend(["anime", "releases", "release", self.alias.as_str()]);

        let mut variants = Vec::new();
        for episode in self.episodes.iter().filter(|_| !needs_inspection).flatten() {
            for quality in [VideoQuality::P1080, VideoQuality::P720, VideoQuality::P480] {
                let Some(stream) = episode.stream(quality) else {
                    continue;
                };
                let address = Url::parse(stream)
                    .map_err(|_| Error::InvalidResponse("invalid stream address"))?;
                let selector = VariantSelector::Episode {
                    id: episode.id.clone(),
                    quality,
                };
                variants.push(Variant {
                    reference: VariantRef {
                        publication: reference.clone(),
                        selector,
                    },
                    title: None,
                    transport: Transport::Video,
                    episode: Some(episode.ordinal.to_string()),
                    quality: Some(format!("{}p", quality.height())),
                    translation: Some("AniLiberty".into()),
                    languages: vec!["ru".into()],
                    player: None,
                    size: None,
                    seeders: None,
                    info_hash: None,
                    reported_video_ads: address.query_pairs().any(|(key, value)| {
                        matches!(key.as_ref(), "isWithVideoAds" | "isWithVideoAdsAlways")
                            && value == "1"
                    }),
                });
            }
            if episode.rutube_id.as_ref().is_some_and(|id| !id.is_empty()) {
                let selector = VariantSelector::Rutube {
                    episode_id: episode.id.clone(),
                };
                variants.push(Variant {
                    reference: VariantRef {
                        publication: reference.clone(),
                        selector,
                    },
                    title: None,
                    transport: Transport::Video,
                    episode: Some(episode.ordinal.to_string()),
                    quality: None,
                    translation: Some("AniLiberty".into()),
                    languages: vec!["ru".into()],
                    player: Some("Rutube".into()),
                    size: None,
                    seeders: None,
                    info_hash: None,
                    reported_video_ads: false,
                });
            }
        }

        for torrent in self
            .torrents
            .into_iter()
            .filter(|_| !needs_inspection)
            .flatten()
        {
            torrent.magnet()?;
            if torrent.label.len() > 2048
                || torrent
                    .description
                    .as_ref()
                    .is_some_and(|value| value.len() > 2048)
                || torrent
                    .quality
                    .as_ref()
                    .is_some_and(|quality| quality.value.len() > 64)
            {
                return Err(Error::InvalidResponse("invalid torrent metadata"));
            }
            let selector = VariantSelector::Torrent { id: torrent.id };
            variants.push(Variant {
                reference: VariantRef {
                    publication: reference.clone(),
                    selector,
                },
                title: Some(torrent.label),
                transport: Transport::Torrent,
                episode: torrent.description,
                quality: torrent.quality.map(|quality| quality.value),
                translation: Some("AniLiberty".into()),
                languages: vec!["ru".into()],
                player: None,
                size: torrent.size,
                seeders: torrent.seeders,
                info_hash: Some(torrent.hash.to_ascii_lowercase()),
                reported_video_ads: false,
            });
        }

        let mut identifiers = BTreeMap::new();
        if let Some(id) = self.mal.and_then(|external| external.id) {
            identifiers.insert("mal".into(), id.to_string());
        }
        if let Some(id) = self.shikimori.and_then(|external| external.id) {
            identifiers.insert("shikimori".into(), id.to_string());
        }
        let mut aliases = Vec::new();
        for alias in [self.name.english, self.name.alternative]
            .into_iter()
            .flatten()
        {
            if !alias.trim().is_empty() && alias != self.name.main && !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
        let cover = self
            .poster
            .and_then(|poster| poster.src)
            .and_then(|source| origin.join(&source).ok())
            .filter(|address| {
                address.origin() == origin.origin()
                    && address.username().is_empty()
                    && address.password().is_none()
            });

        Ok(Work {
            reference,
            title: self.name.main,
            category: Category::Anime,
            url,
            cover,
            description: self.description,
            year: self.year,
            aliases,
            identifiers,
            variants,
            needs_inspection,
        })
    }
}
