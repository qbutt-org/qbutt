//! AniLiberty response types, validation and publication mapping.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use url::Url;

use crate::{
    Category, Error, PublicationRef, Result, Source, Transport, Variant, VariantRef,
    VariantSelector, VideoQuality, Work,
};

#[derive(Deserialize)]
pub(super) struct Release {
    pub(super) id: u64,
    alias: String,
    pub(super) name: ReleaseName,
    description: Option<String>,
    year: Option<u16>,
    poster: Option<Poster>,
    mal: Option<ExternalId>,
    shikimori: Option<ExternalId>,
    pub(super) episodes: Option<Vec<Episode>>,
    pub(super) torrents: Option<Vec<Torrent>>,
}

#[derive(Deserialize)]
pub(super) struct ReleaseName {
    pub(super) main: String,
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

impl Release {
    pub(super) fn validate(&self) -> Result<()> {
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

    pub(super) fn into_work(self, origin: &Url, needs_inspection: bool) -> Result<Work> {
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
        if !needs_inspection {
            for episode in self.episodes.iter().flatten() {
                episode.append_variants(&reference, &mut variants)?;
            }
            for torrent in self.torrents.into_iter().flatten() {
                variants.push(torrent.into_variant(&reference)?);
            }
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

#[derive(Deserialize)]
pub(super) struct Episode {
    pub(super) id: String,
    ordinal: f64,
    hls_480: Option<String>,
    hls_720: Option<String>,
    hls_1080: Option<String>,
    pub(super) rutube_id: Option<String>,
}

impl Episode {
    pub(super) fn stream(&self, quality: VideoQuality) -> Option<&str> {
        let stream = match quality {
            VideoQuality::P480 => &self.hls_480,
            VideoQuality::P720 => &self.hls_720,
            VideoQuality::P1080 => &self.hls_1080,
            _ => return None,
        };
        stream.as_deref().filter(|value| !value.is_empty())
    }

    fn append_variants(
        &self,
        reference: &PublicationRef,
        variants: &mut Vec<Variant>,
    ) -> Result<()> {
        for quality in [VideoQuality::P1080, VideoQuality::P720, VideoQuality::P480] {
            let Some(stream) = self.stream(quality) else {
                continue;
            };
            let address =
                Url::parse(stream).map_err(|_| Error::InvalidResponse("invalid stream address"))?;
            variants.push(Variant {
                reference: VariantRef {
                    publication: reference.clone(),
                    selector: VariantSelector::Episode {
                        id: self.id.clone(),
                        quality,
                    },
                },
                title: None,
                transport: Transport::Video,
                episode: Some(self.ordinal.to_string()),
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
        if let Some(id) = self.rutube_id.as_ref().filter(|id| !id.is_empty()) {
            PublicationRef {
                source: Source::Rutube,
                id: id.clone(),
            }
            .validate()
            .map_err(|_| Error::InvalidResponse("invalid Rutube episode identity"))?;
            variants.push(Variant {
                reference: VariantRef {
                    publication: reference.clone(),
                    selector: VariantSelector::Rutube {
                        episode_id: self.id.clone(),
                        rutube_id: id.clone(),
                    },
                },
                title: None,
                transport: Transport::Video,
                episode: Some(self.ordinal.to_string()),
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
        Ok(())
    }
}

#[derive(Deserialize)]
pub(super) struct Torrent {
    pub(super) id: u64,
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
    pub(super) fn magnet(&self) -> Result<Url> {
        let (magnet, hash) = crate::torrent::magnet(&self.magnet)?;
        if !hash.eq_ignore_ascii_case(&self.hash) {
            return Err(Error::InvalidResponse(
                "torrent magnet does not match its hash",
            ));
        }
        Ok(magnet)
    }

    fn into_variant(self, reference: &PublicationRef) -> Result<Variant> {
        self.magnet()?;
        if self.label.len() > 2048
            || self
                .description
                .as_ref()
                .is_some_and(|value| value.len() > 2048)
            || self
                .quality
                .as_ref()
                .is_some_and(|quality| quality.value.len() > 64)
        {
            return Err(Error::InvalidResponse("invalid torrent metadata"));
        }
        Ok(Variant {
            reference: VariantRef {
                publication: reference.clone(),
                selector: VariantSelector::Torrent { id: self.id },
            },
            title: Some(self.label),
            transport: Transport::Torrent,
            episode: self.description,
            quality: self.quality.map(|quality| quality.value),
            translation: Some("AniLiberty".into()),
            languages: vec!["ru".into()],
            player: None,
            size: self.size,
            seeders: self.seeders,
            info_hash: Some(self.hash.to_ascii_lowercase()),
            reported_video_ads: false,
        })
    }
}
