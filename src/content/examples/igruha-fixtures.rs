//! Generated HTTPS catalog and torrent metadata. No game payload is served.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};

#[allow(dead_code)]
#[path = "../src/torrent.rs"]
mod torrent;

#[allow(dead_code)]
#[path = "../src/limetorrents.rs"]
mod limetorrents;

#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[allow(dead_code)]
mod webview {
    use url::Url;

    #[derive(Debug, thiserror::Error)]
    #[error("WebView is unused in this generated fixture")]
    pub struct WebViewError;

    pub struct WebViewSession {
        pub url: Url,
        pub user_agent: String,
        pub cookies: Vec<WebViewCookie>,
    }

    pub struct WebViewCookie {
        pub name: String,
        pub value: String,
        pub domain: String,
        pub path: String,
        pub expires: Option<f64>,
        pub secure: bool,
        pub http_only: bool,
        pub host_only: bool,
    }
}

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod igruha {
    include!("../src/igruha.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> Igruha {
        Igruha { http, origin }
    }
}

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::start().await?;
    let provider = igruha::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );

    let found = provider
        .search(
            "Generated Legal",
            Category::Games,
            None,
            browser::BrowserChoice::Auto,
        )
        .await?;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].reference.id, "42-generated-legal");
    assert_eq!(found[0].title, "Generated Legal (Свободная игра)");
    assert!(found[0].needs_inspection);

    let work = provider
        .inspect(&found[0].reference, None, browser::BrowserChoice::Auto)
        .await?;
    assert_eq!(work.variants.len(), 2);
    assert!(work.variants[0].title.as_deref().unwrap().contains("v1.2"));
    assert!(work.variants[1].title.as_deref().unwrap().contains("v2.0"));
    assert_ne!(work.variants[0].info_hash, work.variants[1].info_hash);

    let second = &work.variants[1].reference;
    let resolved = provider
        .resolve(second, None, browser::BrowserChoice::Auto)
        .await?;
    let ResolvedContent::TorrentFile {
        reference,
        metadata,
    } = resolved
    else {
        panic!("Igruha did not return exact torrent metadata");
    };
    assert_eq!(reference, *second);
    assert_eq!(metadata.bytes(), second_torrent());

    fixture.changed.store(true, Ordering::SeqCst);
    assert!(matches!(
        provider
            .resolve(second, None, browser::BrowserChoice::Auto)
            .await,
        Err(Error::VariantUnavailable)
    ));
    fixture.changed.store(false, Ordering::SeqCst);

    assert!(
        provider
            .search("empty", Category::Games, None, browser::BrowserChoice::Auto)
            .await?
            .is_empty()
    );
    for query in ["mismatch", "foreign", "duplicate", "challenge"] {
        assert!(
            provider
                .search(query, Category::Games, None, browser::BrowserChoice::Auto)
                .await
                .is_err(),
            "accepted {query}"
        );
    }
    for id in ["43-stale", "44-missing", "../private", "42-Mixed", "0-game"] {
        let reference = PublicationRef {
            source: Source::Igruha,
            id: id.into(),
        };
        assert!(
            provider
                .inspect(&reference, None, browser::BrowserChoice::Auto)
                .await
                .is_err(),
            "accepted {id}"
        );
    }
    let wrong_landing = PublicationRef {
        source: Source::Igruha,
        id: "42-wrong-landing".into(),
    };
    assert!(matches!(
        provider
            .inspect(&wrong_landing, None, browser::BrowserChoice::Auto)
            .await,
        Err(Error::VariantUnavailable)
    ));

    let slow = tokio::spawn(async move {
        provider
            .search("slow", Category::Games, None, browser::BrowserChoice::Auto)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.slow_started.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    slow.abort();
    assert!(slow.await.unwrap_err().is_cancelled());

    fixture.stop().await;
    println!(
        "PASS igruha_fixtures generated_https search inspect second_exact_torrent stale empty fault cancel game_payload=none"
    );
    Ok(())
}

fn first_torrent() -> &'static [u8] {
    b"d8:announce32:https://fixture.example/announce4:infod6:lengthi4e4:name9:legal.txt12:piece lengthi4e6:pieces20:\xa9J\x8f\xe5\xcc\xb1\x9b\xa6\x1cL\x08s\xd3\x91\xe9\x87\x98/\xbb\xd3ee"
}

fn second_torrent() -> &'static [u8] {
    b"d8:announce32:https://fixture.example/announce4:infod6:lengthi4e4:name9:other.txt12:piece lengthi4e6:pieces20:\xa9J\x8f\xe5\xcc\xb1\x9b\xa6\x1cL\x08s\xd3\x91\xe9\x87\x98/\xbb\xd3ee"
}

struct Fixture {
    origin: Url,
    client: reqwest::Client,
    changed: Arc<AtomicBool>,
    slow_started: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> std::result::Result<Self, Box<dyn std::error::Error>> {
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
            .timeout(Duration::from_secs(5))
            .build()?;
        let changed = Arc::new(AtomicBool::new(false));
        let slow_started = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let changed = Arc::clone(&changed);
            let slow_started = Arc::clone(&slow_started);
            async move {
                loop {
                    let socket = tokio::select! {
                        _ = &mut stopped => break,
                        result = listener.accept() => match result {
                            Ok((socket, _)) => socket,
                            Err(_) => break,
                        },
                    };
                    let acceptor = acceptor.clone();
                    let origin = origin.clone();
                    let changed = Arc::clone(&changed);
                    let slow_started = Arc::clone(&slow_started);
                    tokio::spawn(async move {
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
                        let target = String::from_utf8_lossy(&request)
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("")
                            .to_owned();
                        let Ok(url) = origin.join(&target) else {
                            return;
                        };
                        let (status, mime, bytes) =
                            response(&origin, &url, changed.load(Ordering::SeqCst));
                        if url
                            .query_pairs()
                            .any(|(name, value)| name == "story" && value == "slow")
                        {
                            slow_started.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                        let head = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        );
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream.write_all(&bytes).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });
        Ok(Self {
            origin,
            client,
            changed,
            slow_started,
            stop,
            server,
        })
    }

    async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.expect("TLS fixture stopped");
    }
}

fn response(origin: &Url, url: &Url, changed: bool) -> (&'static str, &'static str, Vec<u8>) {
    let query = url
        .query_pairs()
        .find(|(name, _)| name == "story")
        .map(|(_, value)| value.into_owned());
    if url.path() == "/index.php" && query.is_some() {
        let query = query.unwrap();
        if query == "challenge" {
            return ("403 Forbidden", "text/html", b"DDoS-Guard".to_vec());
        }
        let shown = if query == "mismatch" { "other" } else { &query };
        let link = if query == "foreign" {
            "https://clone.example/42-generated-legal.html".into()
        } else {
            format!("{origin}42-generated-legal.html")
        };
        let row = format!(
            "<div class='game-card'><div class='game-card__cover'><img class='game-cover' src='/uploads/posts/fixture.webp'></div><div class='game-card__name'><a href='{link}'>Generated Legal (Свободная игра)</a></div></div>"
        );
        let rows = match query.as_str() {
            "empty" => String::new(),
            "duplicate" => row.repeat(2),
            _ => row,
        };
        let body = format!(
            "<html><div id='dle-content'><div class='searchpage'><form id='fullsearch'><input id='searchinput' name='story' value='{shown}'></form></div>{rows}</div></html>"
        );
        return html(body);
    }
    if url.path().ends_with(".html") {
        if url.path().contains("missing") {
            return ("404 Not Found", "text/html", Vec::new());
        }
        let slug = if url.path().contains("stale") {
            "42-generated-legal"
        } else {
            url.path().trim_start_matches('/').trim_end_matches(".html")
        };
        let releases = if slug == "42-wrong-landing" {
            format!(
                "<div class='is-centered'><span class='size-line'>Размер: 4 MB v1.2 | Legal</span></div><div class='dl-bar'><a class='torrent' href='{origin}?do=download&amp;id=93'>Скачать торрент</a></div>"
            )
        } else {
            format!(
                "<div class='is-centered'><span class='size-line'>Размер: 4 MB v1.2 | Legal</span></div><div class='dl-bar'><a class='torrent' href='{origin}?do=download&amp;id=91'>Скачать торрент</a></div><div class='is-centered'><span class='size-line'>Размер: 4 MB v2.0 | Legal</span></div><div class='dl-bar'><a class='torrent' href='{origin}?do=download&amp;id=92'>Скачать торрент</a></div>"
            )
        };
        let body = format!(
            "<html><head><link rel='canonical' href='{origin}{slug}.html'></head><body><div id='dle-content'><div class='page-title'><h1>Generated Legal (Свободная игра)</h1></div><div id='entry-poster-bg'><img class='entry-image' src='/uploads/posts/fixture.webp'></div><div class='updategame'>Изменения: v2.0</div><div class='blockinfo'>{releases}</div></div></body></html>"
        );
        return html(body);
    }
    if url.path() == "/"
        && url
            .query_pairs()
            .any(|(name, value)| name == "do" && value == "download")
    {
        let id = url
            .query_pairs()
            .find(|(name, _)| name == "id")
            .map(|(_, value)| value.into_owned());
        let article = if id.as_deref() == Some("93") {
            "43-other"
        } else {
            "42-generated-legal"
        };
        let body = format!(
            "<html><div id='dle-content'><div class='dl-back'><a href='{origin}{article}.html'>Вернуться к игре</a></div><a id='btn-download' href='{origin}engine/download.php?id={}'>Скачать торрент</a></div></html>",
            id.as_deref().unwrap_or("0")
        );
        return html(body);
    }
    if url.path() == "/engine/download.php" {
        let id = url
            .query_pairs()
            .find(|(name, _)| name == "id")
            .map(|(_, value)| value.into_owned());
        let bytes = match (id.as_deref(), changed) {
            (Some("91"), _) => first_torrent(),
            (Some("92"), false) => second_torrent(),
            (Some("92"), true) => first_torrent(),
            _ => return ("404 Not Found", "text/html", Vec::new()),
        };
        return ("200 OK", "application/x-bittorrent", bytes.to_vec());
    }
    ("404 Not Found", "text/html", Vec::new())
}

fn html(body: String) -> (&'static str, &'static str, Vec<u8>) {
    let (bytes, _, invalid) = encoding_rs::WINDOWS_1251.encode(&body);
    assert!(
        !invalid,
        "fixture HTML must be representable in Windows-1251"
    );
    (
        "200 OK",
        "text/html; charset=windows-1251",
        bytes.into_owned(),
    )
}
