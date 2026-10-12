//! Generated HTTPS catalog, player and cancellation checks through the native provider.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
pub use qbutt_content::{Error, Result};

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[allow(dead_code)]
mod guarded_http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: url::Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

mod http {
    use std::net::SocketAddr;
    use std::sync::OnceLock;
    use std::time::Duration;

    use reqwest::header::HeaderMap;
    use url::Url;

    #[cfg(windows)]
    pub(crate) use super::guarded_http::validate_media_url;
    pub(crate) use super::guarded_http::{SourceDocument, SourceSession, validate_https_url};
    use super::{Result, guarded_http};

    static CONNECTION: OnceLock<(SocketAddr, Vec<u8>)> = OnceLock::new();

    pub(crate) struct SourceHttp(guarded_http::SourceHttp);

    pub(super) fn connect(address: SocketAddr, certificate: Vec<u8>) {
        CONNECTION.set((address, certificate)).unwrap();
    }

    impl SourceHttp {
        pub(crate) fn new(origin: Url, headers: HeaderMap) -> Result<Self> {
            assert!(matches!(
                origin.host_str(),
                Some("my.animespirit.ru" | "kodikplayer.com")
            ));
            let (address, certificate) = CONNECTION.get().expect("HTTPS fixture started");
            let mut client = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(25))
                .user_agent(concat!("qbutt-content/", env!("CARGO_PKG_VERSION")))
                .default_headers(headers)
                .add_root_certificate(reqwest::Certificate::from_der(certificate).unwrap());
            for name in [
                "my.animespirit.ru",
                "kodikplayer.com",
                "cdn.fixture.example",
            ] {
                client = client.resolve(name, *address);
            }
            Ok(Self(guarded_http::fixture(client.build().unwrap(), origin)))
        }

        pub(crate) async fn session_document(
            &self,
            url: Url,
            session: &mut SourceSession,
        ) -> Result<SourceDocument> {
            self.0.session_document(url, session).await
        }

        pub(crate) async fn html_page(&self, url: Url) -> Result<(Url, String)> {
            self.0.html_page(url).await
        }

        pub(crate) async fn form_json<T: serde::de::DeserializeOwned>(
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
}

#[allow(dead_code)]
#[path = "../src/kodik.rs"]
mod kodik;

#[allow(dead_code)]
#[path = "../src/animespirit.rs"]
mod animespirit;

#[path = "../../../tests/content-providers/animespirit/fixture.rs"]
mod fixture;

use std::sync::atomic::Ordering;
use std::time::Duration;

use fixture::Fixture;

type CheckResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> CheckResult<()> {
    let fixture = Fixture::start().await?;
    http::connect(fixture.address, fixture.certificate.clone());
    let source = animespirit::AnimeSpirit::new()?;
    let result = exercise(&source, &fixture).await;
    fixture.stop.send(()).unwrap();
    fixture.server.await?;
    result?;
    println!(
        "PASS animespirit-fixtures cp1251_search exact_episode_voice stale_selector malformed_player unavailable_match cancellation"
    );
    Ok(())
}

async fn exercise(source: &animespirit::AnimeSpirit, fixture: &Fixture) -> CheckResult<()> {
    let found = source.search("Демо").await?;
    assert_eq!(found.len(), 1);
    let publication = &found[0];
    assert_eq!(publication.reference.source, Source::AnimeSpirit);
    assert_eq!(publication.reference.id, "42");
    assert_eq!(publication.title, "Демо архив");
    assert_eq!(publication.url.path(), "/anime/42-generated.html");
    assert!(publication.needs_inspection);

    let inspected = source.inspect(&publication.reference).await?;
    assert_eq!(inspected.reference, publication.reference);
    assert_eq!(inspected.variants.len(), 2);
    assert!(!inspected.needs_inspection);
    let selected = inspected
        .variants
        .iter()
        .find(|variant| variant.episode.as_deref() == Some("2"))
        .unwrap();
    assert_eq!(
        selected.translation.as_deref(),
        Some("Субтитры Демо студии")
    );
    assert_eq!(selected.languages, ["ja", "ru"]);
    let VariantSelector::AnimeSpiritKodik {
        episode_id,
        episode_hash,
        kodik_translation,
        subtitled,
        ..
    } = &selected.reference.selector
    else {
        panic!("AnimeSpirit returned another selector");
    };
    assert_eq!(episode_id, "4202");
    assert_eq!(episode_hash, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert_eq!(kodik_translation, "Fixture Studio");
    assert!(*subtitled);

    let ResolvedContent::DirectVideo {
        reference,
        url,
        quality,
        ..
    } = source.resolve(&selected.reference).await?
    else {
        panic!("AnimeSpirit did not return direct video");
    };
    assert_eq!(reference, selected.reference);
    assert_eq!(quality.height(), 720);
    assert_eq!(url.as_str(), "https://cdn.fixture.example/42/final.m3u8");
    assert_eq!(fixture.forms.load(Ordering::SeqCst), 1);

    let mut stale = selected.reference.clone();
    let VariantSelector::AnimeSpiritKodik { episode_id, .. } = &mut stale.selector else {
        unreachable!()
    };
    *episode_id = "9999".into();
    assert!(matches!(
        source.resolve(&stale).await,
        Err(Error::VariantUnavailable)
    ));

    let mut unmatched = selected.reference.clone();
    let VariantSelector::AnimeSpiritKodik { voice, .. } = &mut unmatched.selector else {
        unreachable!()
    };
    *voice = "Другая студия".into();
    assert!(matches!(
        source.resolve(&unmatched).await,
        Err(Error::VariantUnavailable)
    ));

    let malformed = PublicationRef {
        source: Source::AnimeSpirit,
        id: "43".into(),
    };
    assert!(matches!(
        source.inspect(&malformed).await,
        Err(Error::InvalidResponse(_))
    ));
    assert_eq!(fixture.forms.load(Ordering::SeqCst), 1);

    let held = PublicationRef {
        source: Source::AnimeSpirit,
        id: "44".into(),
    };
    let mut pending = Box::pin(source.inspect(&held));
    tokio::select! {
        _ = fixture.started.notified() => {},
        result = &mut pending => panic!("cancel completed before held player: {}", result.err().unwrap()),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("cancel did not reach player"),
    }
    drop(pending);
    tokio::time::timeout(Duration::from_secs(3), fixture.closed.notified()).await?;
    Ok(())
}
