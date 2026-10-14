use url::Url;

use crate::browser::{
    BrowserChoice, BrowserCommand, BrowserError, BrowserHandle, BrowserReply, BrowserResolver,
    ResolveKind, Selection,
};
use crate::{Error, PlaybackMedia, VariantRef, VariantSelector};

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
    source_url: Url,
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
    let (command, identity) = match player.as_str() {
        "Плеер VK" => {
            let identity = vk_identity(&url)?;
            // The wrapper identifies one VK video; direct URLs keep access parameters.
            let browser_url = if url.host_str() == Some("ru.yummyani.me") {
                identity.clone()
            } else {
                url
            };
            let selection = Selection {
                episode: Some(episode.clone()),
                translation_id: Some(dubbing.clone()),
                player: Some(player.clone()),
                ..Selection::default()
            };
            (
                BrowserCommand::Resolve {
                    url: browser_url,
                    selection,
                    kind: ResolveKind::Video,
                    referrer: None,
                },
                Some(identity),
            )
        }
        "Плеер Alloha" => {
            let selection = alloha_selection(&url, episode)?;
            if source_url.origin().ascii_serialization() != "https://yani.tv"
                || !source_url.path().starts_with("/catalog/item/")
                || source_url.query().is_some()
                || source_url.fragment().is_some()
            {
                return Err(Error::InvalidResponse("invalid Alloha source page").into());
            }
            (
                BrowserCommand::Resolve {
                    url,
                    selection,
                    kind: ResolveKind::Video,
                    referrer: Some(source_url),
                },
                None,
            )
        }
        _ => {
            return Err(
                Error::UnsupportedAcquisition("This external player is not supported").into(),
            );
        }
    };
    let reply = browser.request(choice, command).await?;
    let BrowserReply::Resolved(media) = reply else {
        return Err(BrowserError::InvalidResponse.into());
    };
    if !matches!(media.resolver, BrowserResolver::Direct)
        || identity.is_some_and(|identity| media.source_url.as_ref() != Some(&identity))
    {
        return Err(BrowserError::SelectionChanged.into());
    }
    Ok((*media).into())
}

fn alloha_selection(url: &Url, episode: &str) -> Result<Selection, Error> {
    crate::http::validate_https_url(url)?;
    if url.host_str() != Some("alloha.yani.tv")
        || url.port().is_some()
        || url.path() != "/"
        || url.fragment().is_some()
    {
        return Err(Error::InvalidResponse("invalid Alloha player address"));
    }
    // The fresh iframe binds its movie token to numeric translation/season/episode.
    // Dubbing labels are catalog metadata, not Alloha translation IDs.
    for name in ["token_movie", "token"] {
        alloha_parameter(url, name)?;
    }
    let translation = alloha_parameter(url, "translation")?;
    let season = alloha_parameter(url, "season")?;
    let selected_episode = alloha_parameter(url, "episode")?;
    if selected_episode != episode
        || [&translation, &season, &selected_episode]
            .into_iter()
            .any(|value| !crate::model::canonical_decimal(value))
    {
        return Err(Error::VariantUnavailable);
    }
    Ok(Selection {
        episode: Some(selected_episode),
        season: Some(season),
        translation_id: Some(translation),
        player: Some("alloha".into()),
        ..Selection::default()
    })
}

fn alloha_parameter(url: &Url, name: &str) -> Result<String, Error> {
    let mut values = url
        .query_pairs()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned());
    let value = values
        .next()
        .filter(|value| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        })
        .ok_or(Error::InvalidResponse("missing Alloha selection parameter"))?;
    if values.next().is_some() {
        return Err(Error::InvalidResponse(
            "duplicate Alloha selection parameter",
        ));
    }
    Ok(value)
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
    let wrapper = host == "ru.yummyani.me" && url.path() == "/iframeVK.html";
    if url.port().is_some()
        || (!wrapper
            && !matches!(
                host,
                "vk.com"
                    | "vk.ru"
                    | "vkvideo.com"
                    | "vkvideo.ru"
                    | "vkontakte.com"
                    | "vkontakte.ru"
            ))
    {
        return Err(Error::InvalidResponse("invalid VK player address"));
    }
    let (owner, video) = if wrapper {
        let mut id = None;
        for (key, value) in url.query_pairs() {
            if key == "id" && id.replace(value.into_owned()).is_some() {
                return Err(Error::InvalidResponse("ambiguous VK video identity"));
            }
        }
        let id = id.ok_or(Error::InvalidResponse("missing VK video identity"))?;
        let id = id.trim();
        let id = id.strip_prefix("video").unwrap_or(id);
        let (owner, video) = id
            .split_once('_')
            .ok_or(Error::InvalidResponse("missing VK video identity"))?;
        (owner.to_owned(), video.to_owned())
    } else if url.path() == "/video_ext.php" {
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
