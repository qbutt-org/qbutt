//! Generated HTTPS integration through the production YummyAnime and Kodik flow.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
pub use qbutt_content::{Error, Result};

#[allow(dead_code)]
mod guarded_http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: url::Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

mod http {
    use std::sync::OnceLock;

    use reqwest::header::HeaderMap;
    use serde::de::DeserializeOwned;
    use url::Url;

    pub(crate) use super::guarded_http::{public_media_url, validate_https_url};
    use super::{Result, guarded_http};

    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    pub(crate) struct SourceHttp(guarded_http::SourceHttp);

    impl SourceHttp {
        pub(crate) fn new(origin: Url, _headers: HeaderMap) -> Result<Self> {
            assert_eq!(origin.as_str(), "https://kodikplayer.com/");
            Ok(Self(guarded_http::fixture(
                CLIENT.get().unwrap().clone(),
                origin,
            )))
        }

        pub(crate) async fn json<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
            self.0.json(url).await
        }

        pub(crate) async fn html_page(&self, url: Url) -> Result<(Url, String)> {
            self.0.html_page(url).await
        }

        pub(crate) async fn form_json<T: DeserializeOwned>(
            &self,
            url: Url,
            form: &str,
        ) -> Result<T> {
            self.0.form_json(url, form).await
        }

        pub(crate) async fn playlist(&self, url: Url) -> Result<(Url, String)> {
            self.0.playlist(url).await
        }
    }

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        CLIENT.set(client.clone()).unwrap();
        SourceHttp(guarded_http::fixture(client, origin))
    }
}

#[allow(dead_code)]
mod yummyanime {
    include!("../src/yummyanime.rs");

    pub(super) fn fixture(http: SourceHttp, api_origin: url::Url) -> YummyAnime {
        YummyAnime {
            http,
            origin: url::Url::parse(ORIGIN).unwrap(),
            api_origin,
        }
    }
}

#[path = "kodik/fixture.rs"]
mod fixture;

use std::sync::atomic::Ordering;
use std::time::Duration;

use fixture::{Case, Fixture};

type CheckResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> CheckResult<()> {
    let fixture = Fixture::start().await?;
    let catalog = yummyanime::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let result = exercise(&catalog, &fixture).await;
    fixture.stop.send(()).unwrap();
    fixture.server.await?;
    result?;
    println!(
        "PASS kodik-fixtures exact_serial_episode_movie_voice_subtitles=ok raw_selector=rejected maximum=honest encoding=checked form_redirects=guarded hls_final_url=guarded bounds=checked cancellation=closed timeout=closed"
    );
    Ok(())
}

async fn exercise(catalog: &yummyanime::YummyAnime, fixture: &Fixture) -> CheckResult<()> {
    for case in [
        Case::Season,
        Case::Serial,
        Case::Episode,
        Case::Movie,
        Case::Alias,
        Case::Subtitles,
        Case::Encoded,
        Case::Form307,
        Case::PageRedirect,
    ] {
        let selected = selection(catalog, case).await?;
        let resolved = catalog.resolve(&selected).await?;
        let ResolvedContent::DirectVideo {
            reference,
            url,
            quality,
            ..
        } = resolved
        else {
            panic!("{case:?} did not resolve directly");
        };
        assert_eq!(reference, selected, "{case:?}");
        assert_eq!(quality.height(), 720, "{case:?}");
        assert_eq!(url.host_str(), Some("cdn.fixture.example"));
        assert!(url.path().ends_with("/final.m3u8"));
    }

    for case in [
        Case::OtherSeasonRedirect,
        Case::OtherEpisodeRedirect,
        Case::OtherVoice,
        Case::SimilarAlias,
        Case::AliasOtherTranslation,
        Case::OtherSubtitleType,
        Case::WrongEpisode,
        Case::WrongId,
        Case::WrongHash,
        Case::OtherTranslation,
        Case::DuplicateEpisode,
        Case::UnselectedEpisode,
        Case::ConflictingTitle,
        Case::WrongVideoType,
        Case::ChangedMovieId,
        Case::DuplicateQuery,
        Case::UnsupportedQuality,
        Case::PaddedQuality,
        Case::UnknownQuality,
        Case::AmbiguousStream,
        Case::MalformedEncoding,
        Case::UnsafeAddress,
        Case::UnsafeRedirect,
        Case::ForeignFormRedirect,
        Case::Form303,
        Case::PageTooLarge,
        Case::FormTooLarge,
        Case::PlaylistTooLarge,
        Case::PlaylistStreamTooLarge,
        Case::InvalidPlaylist,
        Case::RedirectLoop,
    ] {
        let selected = selection(catalog, case).await?;
        let result = catalog.resolve(&selected).await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("{case:?} accepted invalid content"),
        };
        assert!(
            !error.to_string().contains("fixture.example"),
            "private address leaked"
        );
        let expected = match case {
            Case::OtherSeasonRedirect
            | Case::OtherEpisodeRedirect
            | Case::OtherVoice
            | Case::SimilarAlias
            | Case::AliasOtherTranslation
            | Case::OtherSubtitleType
            | Case::WrongEpisode
            | Case::WrongId
            | Case::WrongHash
            | Case::OtherTranslation
            | Case::DuplicateEpisode
            | Case::UnselectedEpisode
            | Case::ChangedMovieId
            | Case::DuplicateQuery => matches!(error, Error::VariantUnavailable),
            Case::ConflictingTitle
            | Case::WrongVideoType
            | Case::UnsupportedQuality
            | Case::PaddedQuality
            | Case::UnknownQuality
            | Case::AmbiguousStream
            | Case::MalformedEncoding
            | Case::InvalidPlaylist => matches!(error, Error::InvalidResponse(_)),
            Case::UnsafeAddress | Case::UnsafeRedirect | Case::ForeignFormRedirect => {
                matches!(error, Error::UnsafeAddress)
            }
            Case::Form303 | Case::RedirectLoop => matches!(error, Error::InvalidRedirect),
            Case::PageTooLarge
            | Case::FormTooLarge
            | Case::PlaylistTooLarge
            | Case::PlaylistStreamTooLarge => matches!(error, Error::ResponseTooLarge),
            _ => unreachable!(),
        };
        assert!(expected, "{case:?} failed at the wrong boundary: {error}");
    }

    let selected = selection(catalog, Case::Serial).await?;
    let before = fixture.forms.load(Ordering::SeqCst);
    for field in 0..5 {
        let mut changed = selected.clone();
        let VariantSelector::YummyAnimeVideo {
            id,
            episode,
            dubbing,
            player,
            player_id,
        } = &mut changed.selector
        else {
            unreachable!()
        };
        match field {
            0 => *id = u64::MAX,
            1 => *episode = "9".into(),
            2 => *dubbing = "Озвучка Other studio".into(),
            3 => *player = "Плеер Other".into(),
            _ => *player_id = 99,
        }
        assert!(matches!(
            catalog.resolve(&changed).await,
            Err(Error::VariantUnavailable)
        ));
    }
    assert_eq!(fixture.forms.load(Ordering::SeqCst), before);

    let selected = selection(catalog, Case::Cancel).await?;
    let mut pending = Box::pin(catalog.resolve(&selected));
    tokio::select! {
        _ = fixture.started.notified() => {},
        result = &mut pending => panic!("cancel completed before held response: {}", result.err().unwrap()),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("cancel did not reach HLS"),
    }
    drop(pending);
    tokio::time::timeout(Duration::from_secs(3), fixture.closed.notified()).await?;

    let selected = selection(catalog, Case::Timeout).await?;
    assert!(matches!(
        catalog.resolve(&selected).await,
        Err(Error::Timeout)
    ));
    tokio::time::timeout(Duration::from_secs(3), fixture.closed.notified()).await?;

    let guarded = guarded_http::fixture(fixture.client.clone(), fixture.origin.clone());
    assert!(matches!(
        guarded
            .form_json::<serde_json::Value>(
                fixture.origin.join("ftor")?,
                &"x".repeat(16 * 1024 + 1)
            )
            .await,
        Err(Error::ResponseTooLarge)
    ));
    Ok(())
}

async fn selection(catalog: &yummyanime::YummyAnime, case: Case) -> CheckResult<VariantRef> {
    let work = catalog
        .inspect(&PublicationRef {
            source: Source::YummyAnime,
            id: (case as u64).to_string(),
        })
        .await?;
    assert_eq!(work.variants.len(), 1);
    Ok(work.variants[0].reference.clone())
}
