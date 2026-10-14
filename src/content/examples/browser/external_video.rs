use qbutt_content::browser::{BrowserChoice, BrowserError, BrowserHandle};
use qbutt_content::{
    ExternalVideoError, PublicationRef, Source, VariantRef, VariantSelector, resolve_external_video,
};
use serde_json::json;
use tokio::net::windows::named_pipe::NamedPipeClient;
use url::Url;

use super::fixture::{FixtureResult, read, write};

pub(super) async fn dispatch(handle: &BrowserHandle, edge: &mut NamedPipeClient) -> FixtureResult {
    let source: Url = "https://yani.tv/catalog/item/fixture-author-video".parse()?;
    let reference = VariantRef {
        publication: PublicationRef {
            source: Source::YummyAnime,
            id: "1248".into(),
        },
        selector: VariantSelector::YummyAnimeVideo {
            id: 454670,
            episode: "1".into(),
            dubbing: "Озвучка AniLibria".into(),
            player: "Плеер Alloha".into(),
            player_id: 2,
        },
    };
    let player: Url = "https://alloha.yani.tv/?token_movie=fixture-movie&translation=10&season=1&episode=1&token=fixture-access&hidden=hidden".parse()?;
    let expected = json!({"episode":"1","season":"1","translationId":"10","player":"alloha"});
    for changed in [
        None,
        Some("episode"),
        Some("season"),
        Some("translationId"),
        Some("player"),
    ] {
        let pending = tokio::spawn({
            let (handle, reference, player, source) = (
                handle.clone(),
                reference.clone(),
                player.clone(),
                source.clone(),
            );
            async move {
                resolve_external_video(&handle, BrowserChoice::Edge, &reference, player, source)
                    .await
            }
        });
        let wire = read(edge).await?;
        assert_eq!(wire["method"], "resolve");
        assert_eq!(wire["url"], player.as_str());
        assert_eq!(wire["referrer"], source.as_str());
        assert_eq!(wire["selection"], expected);
        let mut selection = expected.clone();
        if let Some(field) = changed {
            selection[field] = json!("changed");
        }
        write(
            edge,
            &json!({"id":wire["id"],"result":{
                "kind":"video","resolver":"direct","url":"https://media.example/author.m3u8",
                "selection":selection,"headers":{"Referer":player.as_str()},
            }}),
        )
        .await?;
        let result = pending.await?;
        if changed.is_some() {
            assert!(matches!(
                result,
                Err(ExternalVideoError::Browser(BrowserError::SelectionChanged))
            ));
        } else {
            assert!(result.is_ok());
        }
    }
    for address in [
        player.as_str().to_owned() + "&translation=10",
        player.as_str().replace("episode=1", "episode=2"),
        player.as_str().replace("translation=10", "translation=010"),
        player.as_str().replace("&token=fixture-access", ""),
        player.as_str().replace("alloha.yani.tv", "foreign.example"),
    ] {
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            resolve_external_video(
                handle,
                BrowserChoice::Edge,
                &reference,
                address.parse()?,
                source.clone(),
            ),
        )
        .await?;
        assert!(matches!(result, Err(ExternalVideoError::Content(_))));
    }

    let mut vk = reference.clone();
    let VariantSelector::YummyAnimeVideo { player, .. } = &mut vk.selector else {
        unreachable!()
    };
    *player = "Плеер VK".into();
    for identity in [
        None,
        Some("https://vkvideo.ru/video-1_3"),
        Some("https://vkvideo.ru/video-1_2"),
    ] {
        let pending = tokio::spawn({
            let (handle, reference, source) = (handle.clone(), vk.clone(), source.clone());
            async move {
                resolve_external_video(
                    &handle,
                    BrowserChoice::Edge,
                    &reference,
                    "https://ru.yummyani.me/iframeVK.html?id=-1_2"
                        .parse()
                        .unwrap(),
                    source,
                )
                .await
            }
        });
        let wire = read(edge).await?;
        assert_eq!(wire["url"], "https://vkvideo.ru/video-1_2");
        let mut media = json!({"kind":"video","resolver":"direct",
            "url":"https://media.example/author.mp4","selection":wire["selection"]});
        if let Some(identity) = identity {
            media["sourceUrl"] = json!(identity);
        }
        write(edge, &json!({"id":wire["id"],"result":media})).await?;
        let result = pending.await?;
        assert_eq!(
            result.is_ok(),
            identity == Some("https://vkvideo.ru/video-1_2")
        );
    }
    println!(
        "external_video alloha_exact_selection=ok malformed_selection=rejected vk_source_identity=retained"
    );
    Ok(())
}
