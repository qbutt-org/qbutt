use serde::Deserialize;
use url::Url;

use crate::http::{SourceHttp, public_media_url};
use crate::{Error, Result, VideoQuality};

use super::response::{Anime, Video};

const ORIGIN: &str = "https://plapi.cdnvideohub.com";
const API_PATH: &str = "/api/v1/player/sv";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Playlist {
    is_serial: bool,
    items: Vec<PlaylistItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistItem {
    vk_id: String,
    episode: Option<u32>,
    voice_studio: Option<String>,
    voice_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerVideo {
    united_video_id: u64,
    sources: Sources,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sources {
    mpeg_full_hd_url: Option<String>,
    mpeg_high_url: Option<String>,
    mpeg_medium_url: Option<String>,
    mpeg_low_url: Option<String>,
    mpeg_lowest_url: Option<String>,
    mpeg_tiny_url: Option<String>,
}

pub(super) async fn resolve(
    anime: &Anime,
    selected: &Video,
    player: &Url,
) -> Result<(Url, VideoQuality)> {
    if player.host_str() != Some("ru.yummyani.me")
        || player.port().is_some()
        || player.path() != "/iframeCVH.html"
    {
        return Err(Error::InvalidResponse("invalid CVH player address"));
    }
    let title = parameter(player, "anime_id")?;
    let episode = parameter(player, "episode")?;
    let voice = parameter(player, "dubbing_code")?;
    let dubbing = parameter(player, "dubbing")?;
    let mal_id = anime
        .remote_ids
        .as_ref()
        .and_then(|ids| ids.myanimelist_id)
        .filter(|id| *id != 0)
        .ok_or(Error::VariantUnavailable)?;
    if title != mal_id.to_string()
        || episode != selected.number
        || episode
            .parse::<u32>()
            .ok()
            .filter(|value| *value != 0)
            .is_none()
        || dubbing != selected.data.dubbing
        || voice.trim().is_empty()
        || voice.len() > 512
        || voice.chars().any(char::is_control)
    {
        return Err(Error::VariantUnavailable);
    }

    let origin = Url::parse(ORIGIN).expect("fixed CVH origin");
    let http = SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?;
    let mut endpoint = origin
        .join(&format!("{API_PATH}/playlist"))
        .expect("fixed CVH path");
    // YummyAnime supplies priority-voice, not only-voice. The current player
    // fetches this catalog without vstu and selects a matching voice locally.
    endpoint
        .query_pairs_mut()
        .append_pair("pub", "745")
        .append_pair("aggr", "mali")
        .append_pair("id", &title)
        .append_pair("ep", &episode);
    let playlist: Playlist = http.json(endpoint).await?;
    if playlist.items.len() > 5000 {
        return Err(Error::InvalidResponse("too many CVH playlist items"));
    }
    let voice = voice.trim().to_lowercase();
    let mut matching = playlist.items.iter().filter(|item| {
        let matches_episode = if playlist.is_serial {
            item.episode
                .is_some_and(|value| value.to_string() == episode)
        } else {
            episode == "1" && item.episode.is_none()
        };
        let studio = item
            .voice_studio
            .as_deref()
            .filter(|studio| !studio.trim().is_empty())
            .unwrap_or(&item.voice_type);
        matches_episode
            && studio.trim().to_lowercase() == voice
            && (item.voice_type == "Субтитры") == selected.is_subtitled()
    });
    let chosen = matching.next().ok_or(Error::VariantUnavailable)?;
    if matching.next().is_some()
        || chosen.vk_id.is_empty()
        || chosen.vk_id.len() > 20
        || !chosen.vk_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Error::VariantUnavailable);
    }
    let endpoint = origin
        .join(&format!("{API_PATH}/video/{}", chosen.vk_id))
        .expect("numeric CVH video path");
    let video: PlayerVideo = http.json(endpoint).await?;
    if video.united_video_id.to_string() != chosen.vk_id {
        return Err(Error::VariantUnavailable);
    }
    let (address, quality) = [
        (video.sources.mpeg_full_hd_url, VideoQuality::P1080),
        (video.sources.mpeg_high_url, VideoQuality::P720),
        (video.sources.mpeg_medium_url, VideoQuality::P480),
        (video.sources.mpeg_low_url, VideoQuality::P360),
        (video.sources.mpeg_lowest_url, VideoQuality::P240),
        (video.sources.mpeg_tiny_url, VideoQuality::P144),
    ]
    .into_iter()
    .find_map(|(address, quality)| {
        address
            .filter(|address| !address.is_empty())
            .map(|address| (address, quality))
    })
    .ok_or(Error::VariantUnavailable)?;
    let url = public_media_url(&address).await?;
    crate::http::validate_https_url(&url)?;
    Ok((url, quality))
}

fn parameter(player: &Url, name: &str) -> Result<String> {
    let mut values = player
        .query_pairs()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned());
    let value = values
        .next()
        .ok_or(Error::InvalidResponse("missing CVH selection parameter"))?;
    if values.next().is_some() {
        return Err(Error::InvalidResponse("duplicate CVH selection parameter"));
    }
    Ok(value)
}
