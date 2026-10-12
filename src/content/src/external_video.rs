use url::Url;

use crate::browser::{
    BrowserChoice, BrowserCommand, BrowserError, BrowserHandle, BrowserReply, BrowserResolver,
    ResolveKind, Selection,
};
use crate::model::playback::PlaybackSubtitle;
use crate::{Error, PlaybackAudioTrack, PlaybackMedia, VariantRef, VariantSelector};

#[derive(Debug, thiserror::Error)]
pub enum ExternalVideoError {
    #[error(transparent)]
    Content(#[from] Error),
    #[error(transparent)]
    Browser(#[from] BrowserError),
}

/// Resolve a freshly inspected external player through the existing browser
/// session. The caller keeps the selected reference and owns cancellation.
pub async fn resolve_external_video(
    browser: &BrowserHandle,
    choice: BrowserChoice,
    reference: &VariantRef,
    url: Url,
) -> Result<PlaybackMedia, ExternalVideoError> {
    reference.validate()?;
    let VariantSelector::YummyAnimeVideo {
        episode,
        dubbing,
        player,
        ..
    } = &reference.selector
    else {
        return Err(Error::InvalidReference.into());
    };
    if player != "VK" {
        return Err(Error::UnsupportedAcquisition("This external player is not supported").into());
    }
    let identity = vk_identity(&url)?;
    let selection = Selection {
        episode: Some(episode.clone()),
        translation_id: Some(dubbing.clone()),
        player: Some(player.clone()),
        ..Selection::default()
    };
    let reply = browser
        .request(
            choice,
            BrowserCommand::Resolve {
                url,
                selection,
                kind: ResolveKind::Video,
                referrer: Some(Url::parse("https://yani.tv/").expect("fixed source origin")),
            },
        )
        .await?;
    let BrowserReply::Resolved(media) = reply else {
        return Err(BrowserError::InvalidResponse.into());
    };
    if !matches!(media.resolver, BrowserResolver::Direct)
        || media.source_url.as_ref() != Some(&identity)
    {
        return Err(BrowserError::SelectionChanged.into());
    }
    Ok(PlaybackMedia {
        url: media.url,
        headers: media.headers,
        audio_url: media.audio_url,
        audio_track: media.audio_track.map(|track| PlaybackAudioTrack {
            title: track.title,
            kind: track.kind,
            language: track.language,
            codec: track.codec,
            channels: track.channels,
        }),
        subtitles: media
            .subtitles
            .into_iter()
            .map(|subtitle| PlaybackSubtitle {
                title: subtitle.title,
                url: subtitle.url,
                language: subtitle.language,
            })
            .collect(),
        quality: media.quality,
    })
}

fn vk_identity(url: &Url) -> Result<Url, Error> {
    crate::http::validate_https_url(url)?;
    let host = url.host_str().unwrap_or_default();
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .or_else(|| host.strip_prefix("new."))
        .or_else(|| host.strip_prefix("vksport."))
        .unwrap_or(host);
    if url.port().is_some()
        || !matches!(
            host,
            "vk.com" | "vk.ru" | "vkvideo.com" | "vkvideo.ru" | "vkontakte.com" | "vkontakte.ru"
        )
    {
        return Err(Error::InvalidResponse("invalid VK player address"));
    }
    let (owner, video) = if url.path() == "/video_ext.php" {
        let mut owner = None;
        let mut video = None;
        for (key, value) in url.query_pairs() {
            let field = match key.as_ref() {
                "oid" => &mut owner,
                "id" => &mut video,
                _ => continue,
            };
            if field.replace(value.into_owned()).is_some() {
                return Err(Error::InvalidResponse("ambiguous VK video identity"));
            }
        }
        (
            owner.ok_or(Error::InvalidResponse("missing VK owner identity"))?,
            video.ok_or(Error::InvalidResponse("missing VK video identity"))?,
        )
    } else {
        let overlay = url
            .query_pairs()
            .find_map(|(key, value)| (key == "z").then(|| value.into_owned()));
        let path = overlay
            .as_deref()
            .unwrap_or_else(|| url.path().trim_start_matches('/'));
        let id = path
            .strip_prefix("video")
            .and_then(|path| path.split('/').next())
            .ok_or(Error::InvalidResponse("missing VK video identity"))?;
        let (owner, video) = id
            .split_once('_')
            .ok_or(Error::InvalidResponse("missing VK video identity"))?;
        (owner.to_owned(), video.to_owned())
    };
    let unsigned_owner = owner.strip_prefix('-').unwrap_or(&owner);
    if !crate::model::canonical_decimal(unsigned_owner) || !crate::model::canonical_decimal(&video)
    {
        return Err(Error::InvalidResponse("invalid VK video identity"));
    }
    Url::parse(&format!("https://vkvideo.ru/video{owner}_{video}"))
        .map_err(|_| Error::InvalidResponse("invalid VK video identity"))
}
