//! Generated HTTPS pages and tiny legal torrents exercise the production source.
//! `--live` fetches public catalog and torrent metadata, never game payloads.

use qbutt_content::{
    Category, Error, Library, LibraryView, PublicationRef, ResolvedContent, Result, SharedWork,
    Source, TheLastGame, TorrentMetadata, Transport, Variant, VariantPreferences, VariantRef,
    VariantSelector, Work, WorkGroup,
};

#[cfg(windows)]
use qbutt_content::webview;

#[cfg(windows)]
#[path = "../src/browser/mod.rs"]
#[allow(dead_code)]
mod browser;

#[path = "../src/torrent.rs"]
#[allow(dead_code)]
mod torrent;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod thelastgame {
    include!("../src/thelastgame.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> TheLastGame {
        TheLastGame { http, origin }
    }
}

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

#[path = "thelastgame/pages.rs"]
mod pages;

type CheckResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> CheckResult {
    match std::env::args().nth(1).as_deref() {
        Some("--live") => return live().await,
        Some("--reopen") => {
            return reopen(Path::new(
                &std::env::args().nth(2).ok_or("missing profile")?,
            ));
        }
        _ => {}
    }

    let fixture = Fixture::start().await?;
    let source = thelastgame::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let results = source.search("generated & legal", Category::Games).await?;
    assert_eq!(results.len(), 2);
    assert!(results[0].needs_inspection && results[0].variants.is_empty());
    assert_eq!(
        source
            .search("generated  & legal", Category::Games)
            .await?
            .len(),
        2
    );
    let single = source.search("one", Category::Games).await?;
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].reference, results[0].reference);
    assert!(single[0].needs_inspection && single[0].variants.is_empty());

    let work = source.inspect(&results[0].reference).await?;
    assert_eq!(work.title, "Generated Legal Game");
    assert_eq!(work.variants.len(), 2);
    assert_eq!(work.variants[1].transport, Transport::Torrent);
    assert_eq!(work.variants[1].title.as_deref(), Some("Legal Game v2"));
    assert!(
        work.description
            .as_deref()
            .unwrap()
            .contains("v2 GOG Полная")
    );
    assert!(
        work.variants
            .iter()
            .all(|variant| variant.languages.is_empty())
    );
    let selected = work.variants[1].reference.clone();
    let ResolvedContent::TorrentFile {
        reference,
        metadata: acquired,
    } = source.resolve(&selected).await?
    else {
        panic!("expected exact torrent metadata");
    };
    assert_eq!(reference, selected);
    assert_eq!(acquired.bytes(), metadata("Legal Game 2"));
    assert_eq!(
        work.variants[1].info_hash.as_deref(),
        Some(torrent::info_hash(acquired.bytes())?.as_str())
    );

    library(&work, &selected)?;
    for fault in ["changed", "gone", "substituted"] {
        *fixture.torrent_fault.lock().unwrap() = fault;
        assert!(matches!(
            source.resolve(&selected).await,
            Err(Error::VariantUnavailable)
        ));
    }
    for (fault, expected) in [
        ("foreign", "unsafe"),
        ("wrong-mime", "invalid"),
        ("malformed", "invalid"),
        ("oversized", "large"),
        ("challenge", "challenge"),
        ("server-error", "http"),
    ] {
        *fixture.torrent_fault.lock().unwrap() = fault;
        let error = source.resolve(&selected).await.err().ok_or(fault)?;
        assert!(
            matches!(
                (&error, expected),
                (Error::UnsafeAddress, "unsafe")
                    | (Error::InvalidResponse(_), "invalid")
                    | (Error::ResponseTooLarge, "large")
                    | (Error::Challenge, "challenge")
                    | (Error::HttpStatus(500), "http")
            ),
            "torrent {fault}: {error:?}"
        );
    }
    *fixture.torrent_fault.lock().unwrap() = "";
    assert!(source.resolve(&selected).await.is_ok());

    assert!(source.search("empty", Category::Games).await?.is_empty());
    for (query, expected) in [
        ("wrong-query", "invalid"),
        ("duplicate", "invalid"),
        ("foreign-link", "reference"),
        ("empty-with-results", "invalid"),
        ("malformed", "invalid"),
        ("wrong-mime", "invalid"),
        ("challenge", "challenge"),
        ("oversized", "large"),
        ("foreign-redirect", "unsafe"),
        ("http-redirect", "unsafe"),
        ("private-redirect", "unsafe"),
        ("redirect-loop", "redirect"),
    ] {
        let error = source.search(query, Category::Games).await.unwrap_err();
        assert!(
            matches!(
                (&error, expected),
                (Error::InvalidResponse(_), "invalid")
                    | (Error::InvalidReference, "reference")
                    | (Error::Challenge, "challenge")
                    | (Error::ResponseTooLarge, "large")
                    | (Error::UnsafeAddress, "unsafe")
                    | (Error::InvalidRedirect, "redirect")
            ),
            "search {query}: {error:?}"
        );
    }
    for (slug, expected) in [
        ("missing", "unavailable"),
        ("gone", "unavailable"),
        ("identity", "unavailable"),
        ("redirect", "unavailable"),
        ("challenge", "challenge"),
        ("no-torrent", "unsupported"),
        ("too-many", "invalid"),
        ("duplicate-canonical", "invalid"),
    ] {
        let error = source.inspect(&publication(slug)).await.unwrap_err();
        assert!(
            matches!(
                (&error, expected),
                (Error::VariantUnavailable, "unavailable")
                    | (Error::Challenge, "challenge")
                    | (Error::UnsupportedAcquisition(_), "unsupported")
                    | (Error::InvalidResponse(_), "invalid")
            ),
            "article {slug}: {error:?}"
        );
    }

    let hits = fixture.hits.load(Ordering::SeqCst);
    for query in ["", "bad\nquery", &"x".repeat(501)] {
        assert!(matches!(
            source.search(query, Category::Games).await,
            Err(Error::InvalidQuery)
        ));
    }
    assert!(
        source
            .search("generated", Category::Movies)
            .await?
            .is_empty()
    );
    for slug in [
        "",
        "../game",
        "Game",
        "game/other",
        "-game",
        "game-",
        "g%61me",
    ] {
        assert!(matches!(
            source.inspect(&publication(slug)).await,
            Err(Error::InvalidReference)
        ));
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits);

    let mut pending = Box::pin(source.search("cancel", Category::Games));
    tokio::select! {
        started = tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.cancelled.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        }) => started?,
        _ = &mut pending => panic!("cancel completed before fixture request"),
    }
    drop(pending);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert_eq!(
        source
            .search("generated & legal", Category::Games)
            .await?
            .len(),
        2
    );
    assert_eq!(fixture.payload.load(Ordering::SeqCst), 0);
    fixture.close().await;
    println!(
        "PASS thelastgame_fixtures HTTPS=ok exact_second_hash=ok stale=rejected faults=rejected SQLite_reopen=ok share=ok cancel=connection_closed payload=none"
    );
    Ok(())
}

fn publication(slug: &str) -> PublicationRef {
    PublicationRef {
        source: Source::TheLastGame,
        id: slug.to_owned(),
    }
}

fn metadata(name: &str) -> Vec<u8> {
    let info = format!(
        "d6:lengthi1e4:name{}:{}12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae",
        name.len(),
        name
    );
    format!("d4:info{info}e").into_bytes()
}

fn library(work: &Work, selected: &VariantRef) -> CheckResult {
    let profile = std::env::temp_dir().join(format!(
        "qbutt-thelastgame-library-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir(&profile)?;
    let result = (|| -> CheckResult {
        let mut library = Library::open(&profile)?;
        let group = WorkGroup::single(work.clone());
        library.set_favorite(&group, true)?;
        library.save_preferences(
            &group,
            VariantPreferences {
                selected: Some(selected.clone()),
                ..VariantPreferences::default()
            },
        )?;
        let before = library.get(&work.reference)?;
        let mut invalid = work.clone();
        invalid.variants[1].info_hash = invalid.variants[0].info_hash.clone();
        assert!(
            library
                .set_favorite(&WorkGroup::single(invalid), true)
                .is_err()
        );
        assert_eq!(library.get(&work.reference)?, before);
        assert!(library.entries(LibraryView::History)?.is_empty());
        let shared = SharedWork::new(vec![work.reference.clone()], Some(selected.clone()))?;
        assert_eq!(SharedWork::from_url(&shared.to_url()?)?, shared);
        assert_eq!(
            serde_json::to_string(&Source::TheLastGame)?,
            "\"thelastgame\""
        );
        library.set_source_enabled(Source::TheLastGame, false)?;
        drop(library);
        assert!(
            std::process::Command::new(std::env::current_exe()?)
                .arg("--reopen")
                .arg(&profile)
                .status()?
                .success()
        );
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    result
}

fn reopen(profile: &Path) -> CheckResult {
    let library = Library::open(profile)?;
    let entry = library
        .get(&publication("generated-game"))?
        .ok_or("missing favorite")?;
    let work = entry.group.anchor_work().ok_or("missing work")?;
    assert!(entry.favorite && entry.last_opened.is_none());
    assert_eq!(work.variants.len(), 2);
    assert_eq!(work.variants[1].title.as_deref(), Some("Legal Game v2"));
    assert!(
        work.description
            .as_deref()
            .unwrap()
            .contains("v2 GOG Полная")
    );
    assert_eq!(
        entry.preferences.selected.as_ref(),
        Some(&work.variants[1].reference)
    );
    assert_eq!(
        work.variants[1].info_hash.as_deref(),
        Some(torrent::info_hash(&metadata("Legal Game 2"))?.as_str())
    );
    assert!(library.entries(LibraryView::History)?.is_empty());
    assert!(
        library
            .source_settings()?
            .iter()
            .any(|setting| setting.source == Source::TheLastGame && !setting.enabled)
    );
    println!(
        "PASS thelastgame_library_reopen exact_second_variant=ok history=empty source_choice=preserved"
    );
    Ok(())
}

async fn live() -> CheckResult {
    let source = TheLastGame::new()?;
    let found = source.search("resident", Category::Games).await?;
    assert!(!found.is_empty());
    let single = source.search("stardew", Category::Games).await?;
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].reference, publication("stardew-valley"));
    let work = source.inspect(&single[0].reference).await?;
    let variant = work.variants.first().ok_or("no live torrent")?;
    let ResolvedContent::TorrentFile {
        reference,
        metadata,
    } = source.resolve(&variant.reference).await?
    else {
        return Err("live source did not return torrent metadata".into());
    };
    assert_eq!(reference, variant.reference);
    assert_eq!(
        variant.info_hash.as_deref(),
        Some(torrent::info_hash(metadata.bytes())?.as_str())
    );
    assert!(
        source
            .search("qbutt-no-result-01736", Category::Games)
            .await?
            .is_empty()
    );
    println!(
        "{}",
        serde_json::json!({
            "result": "PASS thelastgame_live", "search_rows": found.len(),
            "publication": work.reference, "title": work.title, "variant": variant.title,
            "info_hash": variant.info_hash, "torrent_metadata_bytes": metadata.bytes().len(),
            "payload_requested": false, "native_handoff_verified": false
        })
    );
    Ok(())
}

struct Fixture {
    origin: Url,
    client: reqwest::Client,
    torrent_fault: Arc<Mutex<&'static str>>,
    hits: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    payload: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> CheckResult<Self> {
        let certificate = rcgen::generate_simple_self_signed(vec!["fixture.example".into()])?;
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
            )?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let origin = Url::parse(&format!("https://fixture.example:{}/", address.port()))?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(reqwest::Certificate::from_der(certificate.cert.der())?)
            .resolve("fixture.example", address)
            .timeout(Duration::from_secs(3))
            .build()?;
        let torrent_fault = Arc::new(Mutex::new(""));
        let hits = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let payload = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let torrent_fault = Arc::clone(&torrent_fault);
            let hits = Arc::clone(&hits);
            let cancelled = Arc::clone(&cancelled);
            let payload = Arc::clone(&payload);
            async move {
                let mut requests = JoinSet::new();
                loop {
                    let socket = tokio::select! {
                        _ = &mut stopped => break,
                        Some(_) = requests.join_next(), if !requests.is_empty() => continue,
                        socket = listener.accept() => match socket {
                            Ok((socket, _)) => socket,
                            Err(_) => break,
                        },
                    };
                    let acceptor = acceptor.clone();
                    let origin = origin.clone();
                    let torrent_fault = Arc::clone(&torrent_fault);
                    let hits = Arc::clone(&hits);
                    let cancelled = Arc::clone(&cancelled);
                    let payload = Arc::clone(&payload);
                    requests.spawn(async move {
                        let Ok(mut stream) = acceptor.accept(socket).await else {
                            return;
                        };
                        let mut request = Vec::new();
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                            let mut byte = [0];
                            if stream.read_exact(&mut byte).await.is_err() {
                                return;
                            }
                            request.push(byte[0]);
                        }

                        hits.fetch_add(1, Ordering::SeqCst);
                        let target = String::from_utf8_lossy(&request)
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("")
                            .to_owned();
                        let Ok(url) = origin.join(&target) else {
                            return;
                        };
                        if url.query_pairs().any(|(key, value)| key == "s" && value == "cancel") {
                            cancelled.store(1, Ordering::SeqCst);
                            let mut byte = [0];
                            if matches!(stream.read(&mut byte).await, Ok(0) | Err(_)) {
                                cancelled.store(2, Ordering::SeqCst);
                            }
                            return;
                        }

                        if url.path().ends_with("blue.torrent") {
                            payload.fetch_add(1, Ordering::SeqCst);
                        }
                        if url.path().starts_with("/download/") {
                            let headers = String::from_utf8_lossy(&request).to_lowercase();
                            assert!(headers.contains(&format!("referer: {origin}generated-game/")));
                        }
                        let fault = *torrent_fault.lock().unwrap();
                        let (status, mime, location, bytes) = pages::response(&url, &origin, fault);
                        let header = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        );
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(&bytes).await;
                        let _ = stream.shutdown().await;
                    });
                }
                requests.abort_all();
                while requests.join_next().await.is_some() {}
            }
        });
        Ok(Self {
            origin,
            client,
            torrent_fault,
            hits,
            cancelled,
            payload,
            stop,
            server,
        })
    }

    async fn close(self) {
        let _ = self.stop.send(());
        self.server.await.expect("TLS fixture stopped");
    }
}
