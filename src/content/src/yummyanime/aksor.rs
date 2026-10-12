use serde::Deserialize;
use url::Url;

use crate::http::{SourceHttp, public_media_url};
use crate::{Error, Result, VideoQuality};

use super::response::{Anime, Video};

const ORIGIN: &str = "https://player.aksor.tv";

#[derive(Deserialize)]
struct PlayerVideo {
    id: String,
    anime_id: u64,
    episode: String,
    studio_name: String,
    is_subtitle: bool,
    qualities: Qualities,
}

#[derive(Deserialize)]
struct Qualities {
    q1080: Option<String>,
    q720: Option<String>,
    q480: Option<String>,
    q360: Option<String>,
}

pub(super) async fn resolve(
    anime: &Anime,
    selected: &Video,
    player: &Url,
) -> Result<(Url, VideoQuality)> {
    let origin = Url::parse(ORIGIN).expect("fixed Aksor origin");
    let hash = player
        .path()
        .strip_prefix("/video/")
        .ok_or(Error::InvalidResponse("invalid Aksor player path"))?;
    if player.origin() != origin.origin()
        || hash.len() != 32
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || player.query().is_some()
    {
        return Err(Error::InvalidResponse("invalid Aksor player identity"));
    }
    let http = SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?;
    let endpoint = origin
        .join(&format!("/api/video/{hash}"))
        .expect("validated Aksor path");
    let video: PlayerVideo = http.json(endpoint).await?;
    let studio = selected
        .data
        .dubbing
        .strip_prefix("Озвучка ")
        .or_else(|| selected.data.dubbing.strip_prefix("Субтитры "))
        .unwrap_or(if selected.is_subtitled() {
            "subtitles"
        } else {
            &selected.data.dubbing
        });
    if video.id != hash
        || video.anime_id != anime.anime_id
        || video.episode != selected.number
        || video.is_subtitle != selected.is_subtitled()
        || normalized(&video.studio_name) != normalized(studio)
    {
        return Err(Error::VariantUnavailable);
    }
    let (address, quality) = [
        (video.qualities.q1080, VideoQuality::P1080),
        (video.qualities.q720, VideoQuality::P720),
        (video.qualities.q480, VideoQuality::P480),
        (video.qualities.q360, VideoQuality::P360),
    ]
    .into_iter()
    .find_map(|(address, quality)| {
        address
            .filter(|address| !address.is_empty())
            .map(|address| (address, quality))
    })
    .ok_or(Error::VariantUnavailable)?;
    let url = public_media_url(&address).await?;
    if url.scheme() != "https" || !url.path().ends_with(".mpd") {
        return Err(Error::InvalidResponse("unsupported Aksor stream address"));
    }
    Ok((url, quality))
}

fn normalized(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
