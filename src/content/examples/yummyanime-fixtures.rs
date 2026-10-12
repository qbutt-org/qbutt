//! Generated catalog responses travel through the production provider and HTTP
//! parser. The fixture alone trusts its own local TLS certificate.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
pub use qbutt_content::{Error, Result};

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod yummyanime {
    include!("../src/yummyanime.rs");

    pub(super) fn fixture(http: SourceHttp, api_origin: Url) -> YummyAnime {
        YummyAnime {
            http,
            origin: Url::parse(ORIGIN).unwrap(),
            api_origin,
        }
    }
}

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qbutt_content::{Library, LibraryView, VariantFilters, VariantPreferences};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

type CheckResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> CheckResult<()> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
        return reopen(Path::new(
            &arguments.next().ok_or("missing fixture profile")?,
        ));
    }

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
        "PASS yummyanime-fixtures search_inspect=ok invalid_changed_duplicate_missing=rejected challenge=failed empty=success redirects_size=guarded cancellation=connection_closed sqlite_reopen=exact"
    );
    Ok(())
}

async fn exercise(catalog: &yummyanime::YummyAnime, fixture: &Fixture) -> CheckResult<()> {
    let search = catalog.search("legal").await?;
    assert_eq!(search.len(), 1);
    assert!(search[0].needs_inspection && search[0].variants.is_empty());
    let work = catalog.inspect(&search[0].reference).await?;
    assert_eq!(work.reference, search[0].reference);
    assert_eq!(work.variants.len(), 2);
    assert!(!work.needs_inspection);
    assert_eq!(
        work.variants[0].translation.as_deref(),
        Some("Fixture studio")
    );
    assert_eq!(work.variants[1].languages, ["ja", "ru"]);
    assert!(catalog.search("empty").await?.is_empty());
    assert!(matches!(
        catalog.search("challenge").await,
        Err(Error::Json(_))
    ));
    assert!(matches!(
        catalog.search("blocked").await,
        Err(Error::HttpStatus(403))
    ));
    assert!(matches!(
        catalog.search("redirect").await,
        Err(Error::UnsafeAddress)
    ));
    assert!(matches!(
        catalog.search("oversized").await,
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        catalog.search("too-many").await,
        Err(Error::InvalidResponse(_))
    ));

    for id in [43, 44, 45] {
        let publication = PublicationRef {
            source: Source::YummyAnime,
            id: id.to_string(),
        };
        assert!(matches!(
            catalog.inspect(&publication).await,
            Err(Error::InvalidResponse(_))
        ));
    }
    for field in 0..5 {
        let mut changed = work.variants[0].reference.clone();
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
            0 => *id = 999,
            1 => *episode = "2".into(),
            2 => *dubbing = "Changed studio".into(),
            3 => *player = "Changed player".into(),
            _ => *player_id = 999,
        }
        assert!(matches!(
            catalog.resolve(&changed).await,
            Err(Error::VariantUnavailable)
        ));
    }
    // The generated video's local address must never leave the provider.
    assert!(matches!(
        catalog.resolve(&work.variants[0].reference).await,
        Err(Error::UnsafeAddress)
    ));

    let mut pending = Box::pin(catalog.search("cancel"));
    tokio::select! {
        _ = fixture.cancel_started.notified() => {},
        result = &mut pending => panic!("request finished before cancellation: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("fixture did not receive cancellation request"),
    }
    drop(pending);
    tokio::time::timeout(Duration::from_secs(3), fixture.cancel_closed.notified()).await?;

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!(
        "qbutt-yummyanime-fixtures-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&profile)?;
    let saved = save_and_reopen(&profile, &work);
    std::fs::remove_dir_all(&profile)?;
    saved
}

fn save_and_reopen(profile: &Path, work: &Work) -> CheckResult<()> {
    let work: qbutt_content::Work = serde_json::from_value(serde_json::to_value(work)?)?;
    let mut library = Library::open(profile)?;
    library.set_favorite(&work, true)?;
    library.save_preferences(
        &work,
        VariantPreferences {
            selected: Some(work.variants[1].reference.clone()),
            filters: VariantFilters {
                episode: work.variants[1].episode.clone(),
                translation: work.variants[1].translation.clone(),
                player: work.variants[1].player.clone(),
                source: Some(qbutt_content::Source::YummyAnime),
                ..VariantFilters::default()
            },
        },
    )?;
    assert!(library.entries(LibraryView::History)?.is_empty());
    let confirmed = library.get(&work.reference)?.unwrap();
    let mut invalid = work.clone();
    invalid.variants[0].transport = qbutt_content::Transport::Torrent;
    assert!(library.set_favorite(&invalid, false).is_err());
    assert_eq!(library.get(&work.reference)?.unwrap(), confirmed);
    drop(library);
    let status = std::process::Command::new(std::env::current_exe()?)
        .arg("--reopen")
        .arg(profile)
        .status()?;
    assert!(
        status.success(),
        "second process could not restore exact YummyAnime state"
    );
    Ok(())
}

fn reopen(profile: &Path) -> CheckResult<()> {
    let library = Library::open(profile)?;
    let entries = library.entries(LibraryView::Favorites)?;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.work.variants.len(), 2);
    assert_eq!(
        entry.preferences.selected.as_ref(),
        Some(&entry.work.variants[1].reference)
    );
    assert_eq!(
        entry.preferences.filters.translation.as_deref(),
        Some("Субтитры Fixture subtitles")
    );
    assert!(entry.last_opened.is_none());
    let qbutt_content::VariantSelector::YummyAnimeVideo {
        id,
        episode,
        dubbing,
        player,
        player_id,
    } = &entry.work.variants[1].reference.selector
    else {
        unreachable!()
    };
    assert_eq!(
        (
            *id,
            episode.as_str(),
            dubbing.as_str(),
            player.as_str(),
            *player_id
        ),
        (102, "1", "Субтитры Fixture subtitles", "Плеер Fixture", 10)
    );
    Ok(())
}

fn anime(id: u64) -> Value {
    let mut anime = json!({
        "anime_id": id,
        "anime_url": "generated-legal-animation",
        "title": "Generated legal animation",
        "year": 2026,
        "other_titles": ["Generated title"],
        "videos": [
            {
                "video_id": 101,
                "number": "1",
                "data": {
                    "player": "Плеер Fixture",
                    "dubbing": "Озвучка Fixture studio",
                    "player_id": 10
                },
                "iframe_url": "https://127.0.0.1/private"
            },
            {
                "video_id": 102,
                "number": "1",
                "data": {
                    "player": "Плеер Fixture",
                    "dubbing": "Субтитры Fixture subtitles",
                    "player_id": 10
                },
                "iframe_url": "https://example.org/exact-video"
            }
        ]
    });
    match id {
        43 => anime["anime_id"] = json!(42),
        44 => {
            anime.as_object_mut().unwrap().remove("videos");
        }
        45 => anime["videos"][1]["video_id"] = json!(101),
        _ => {}
    }
    anime
}

struct Fixture {
    origin: Url,
    client: reqwest::Client,
    cancel_started: Arc<Notify>,
    cancel_closed: Arc<Notify>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> CheckResult<Self> {
        let certified = rcgen::generate_simple_self_signed(vec!["fixture.example".into()])?;
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certified.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()).into(),
            )?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let origin = Url::parse(&format!("https://fixture.example:{}", address.port()))?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .resolve("fixture.example", address)
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .build()?;
        let (stop, mut shutdown) = oneshot::channel();
        let cancel_started = Arc::new(Notify::new());
        let cancel_closed = Arc::new(Notify::new());
        let started = cancel_started.clone();
        let closed = cancel_closed.clone();
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown => break,
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); },
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let acceptor = acceptor.clone();
                        let started = started.clone();
                        let closed = closed.clone();
                        connections.spawn(async move {
                            let stream = acceptor.accept(socket).await.unwrap();
                            serve(stream, started, closed).await;
                        });
                    }
                }
            }
            connections.shutdown().await;
        });
        Ok(Self {
            origin,
            client,
            cancel_started,
            cancel_closed,
            stop,
            server,
        })
    }
}

async fn serve(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    started: Arc<Notify>,
    closed: Arc<Notify>,
) {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
        let mut byte = [0];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        request.push(byte[0]);
    }
    let request = String::from_utf8(request).unwrap();
    let target = request.split_whitespace().nth(1).unwrap();
    let url = Url::parse(&format!("https://fixture.example{target}")).unwrap();
    let query = url
        .query_pairs()
        .find(|(key, _)| key == "q")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    if query == "cancel" {
        started.notify_one();
        let mut byte = [0];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            closed.notify_one();
        }
        return;
    }
    let body = if let Some(id) = url.path().strip_prefix("/anime/") {
        assert_eq!(url.query(), Some("need_videos=true"));
        json!({"response": anime(id.parse().unwrap())})
    } else {
        assert!(
            url.query_pairs()
                .any(|(key, value)| key == "limit" && value == "40")
        );
        let count = match query.as_str() {
            "empty" => 0,
            "too-many" => 41,
            _ => 1,
        };
        json!({"response": (0..count).map(|_| anime(42)).collect::<Vec<_>>()})
    };
    let mut status = "200 OK";
    let mut extra = String::new();
    let mut body = body.to_string();
    match query.as_str() {
        "challenge" => body = "<html><title>Checking your browser</title></html>".into(),
        "blocked" => status = "403 Forbidden",
        "redirect" => {
            status = "302 Found";
            extra = "Location: https://foreign.example/anime\r\n".into();
        }
        "oversized" => extra = "Content-Length: 25165825\r\n".into(),
        _ => {}
    }
    if query != "oversized" {
        extra.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Connection: close\r\n\r\n"
    );
    if stream.write_all(header.as_bytes()).await.is_ok() {
        let _ = stream.write_all(body.as_bytes()).await;
    }
    let _ = stream.shutdown().await;
}
