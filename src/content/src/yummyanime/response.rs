//! YummyAnime catalog responses and durable publication mapping.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use url::Url;

use crate::{
    Category, Error, PublicationRef, Result, Source, Transport, Variant, VariantRef,
    VariantSelector, Work,
};

#[derive(Deserialize)]
pub(super) struct Envelope<T> {
    pub(super) response: T,
}

#[derive(Deserialize)]
pub(super) struct Anime {
    pub(super) anime_id: u64,
    anime_url: String,
    pub(super) title: String,
    description: Option<String>,
    year: Option<u16>,
    poster: Option<Poster>,
    other_titles: Option<Vec<String>>,
    pub(super) remote_ids: Option<RemoteIds>,
    pub(super) videos: Option<Vec<Video>>,
}

#[derive(Deserialize)]
struct Poster {
    big: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RemoteIds {
    pub(super) myanimelist_id: Option<u64>,
    shikimori_id: Option<u64>,
    kp_id: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct Video {
    pub(super) video_id: u64,
    pub(super) number: String,
    pub(super) data: VideoData,
    pub(super) iframe_url: String,
}

#[derive(Deserialize)]
pub(super) struct VideoData {
    pub(super) player: String,
    pub(super) dubbing: String,
    pub(super) player_id: u64,
}

impl Anime {
    pub(super) fn validate(&self, inspected: bool) -> Result<()> {
        if self.anime_id == 0
            || !label(&self.title, 2048)
            || self.anime_url.is_empty()
            || self.anime_url.len() > 256
            || self.anime_url.split('-').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            })
        {
            return Err(Error::InvalidResponse(
                "invalid publication identity or title",
            ));
        }
        if self
            .description
            .as_ref()
            .is_some_and(|value| value.len() > 128 * 1024)
            || self.other_titles.as_ref().is_some_and(|aliases| {
                aliases.len() > 100 || aliases.iter().any(|alias| !label(alias, 2048))
            })
            || self
                .videos
                .as_ref()
                .is_some_and(|videos| videos.len() > 50_000)
        {
            return Err(Error::InvalidResponse(
                "publication metadata exceeds its limits",
            ));
        }
        if inspected && self.videos.is_none() {
            return Err(Error::InvalidResponse("missing publication videos"));
        }

        let mut ids = HashSet::new();
        for video in self.videos.iter().flatten() {
            if video.video_id == 0
                || video.data.player_id == 0
                || !label(&video.number, 128)
                || !label(&video.data.dubbing, 512)
                || !label(&video.data.player, 128)
                || video.iframe_url.is_empty()
                || video.iframe_url.len() > 8192
                || !ids.insert(video.video_id)
            {
                return Err(Error::InvalidResponse(
                    "invalid or duplicate video identity",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn publication_url(&self, origin: &Url) -> Url {
        let mut url = origin.clone();
        url.path_segments_mut()
            .expect("fixed YummyAnime origin")
            .clear()
            .extend(["catalog", "item", &self.anime_url]);
        url
    }

    pub(super) fn into_work(self, origin: &Url, needs_inspection: bool) -> Result<Work> {
        self.validate(!needs_inspection)?;
        let reference = PublicationRef {
            source: Source::YummyAnime,
            id: self.anime_id.to_string(),
        };
        let url = self.publication_url(origin);

        let mut variants = Vec::new();
        if !needs_inspection {
            for video in self.videos.iter().flatten() {
                variants.push(video.variant(&reference));
            }
        }
        let mut aliases = Vec::new();
        for alias in self.other_titles.into_iter().flatten() {
            if alias != self.title && !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
        let mut identifiers = BTreeMap::new();
        if let Some(ids) = self.remote_ids {
            for (name, id) in [
                ("mal", ids.myanimelist_id),
                ("shikimori", ids.shikimori_id),
                ("kinopoisk", ids.kp_id),
            ] {
                if let Some(id) = id.filter(|id| *id != 0) {
                    identifiers.insert(name.into(), id.to_string());
                }
            }
        }
        let cover = self
            .poster
            .and_then(|poster| poster.big)
            .map(|value| {
                let cover = origin.join(&value).map_err(|_| Error::UnsafeAddress)?;
                crate::http::validate_https_url(&cover)?;
                if cover.host_str() != Some("imgproxy.yani.tv") || cover.query().is_some() {
                    return Err(Error::UnsafeAddress);
                }
                Ok(cover)
            })
            .transpose()?;

        Ok(Work {
            reference,
            title: self.title,
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

impl Video {
    pub(super) fn selector(&self) -> VariantSelector {
        VariantSelector::YummyAnimeVideo {
            id: self.video_id,
            episode: self.number.clone(),
            dubbing: self.data.dubbing.clone(),
            player: self.data.player.clone(),
            player_id: self.data.player_id,
        }
    }

    pub(super) fn player(&self) -> &str {
        self.data
            .player
            .strip_prefix("Плеер ")
            .unwrap_or(&self.data.player)
    }

    pub(super) fn is_subtitled(&self) -> bool {
        self.data.dubbing == "Субтитры" || self.data.dubbing.starts_with("Субтитры ")
    }

    fn variant(&self, publication: &PublicationRef) -> Variant {
        Variant {
            reference: VariantRef {
                publication: publication.clone(),
                selector: self.selector(),
            },
            title: None,
            transport: Transport::Video,
            episode: Some(self.number.clone()),
            quality: None,
            translation: Some(
                self.data
                    .dubbing
                    .strip_prefix("Озвучка ")
                    .unwrap_or(&self.data.dubbing)
                    .into(),
            ),
            languages: if self.is_subtitled() {
                vec!["ja".into(), "ru".into()]
            } else {
                vec!["ru".into()]
            },
            player: Some(self.player().into()),
            size: None,
            seeders: None,
            info_hash: None,
            reported_video_ads: false,
        }
    }
}

fn label(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}
