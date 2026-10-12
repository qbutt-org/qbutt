//! Generated legal HTTPS search, article and torrent traffic through ByRut.
//! The listener never serves game payload or leaves the loopback interface.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
pub use qbutt_content::{Error, Result};

#[path = "../src/torrent.rs"]
mod torrent;

#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod byrut {
    include!("../src/byrut.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> ByRut {
        ByRut { http, origin }
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
    let provider = byrut::fixture(
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
    assert!(found[0].needs_inspection);
    assert_eq!(found[0].description.as_deref(), Some("v 1.2"));
    let work = provider
        .inspect(&found[0].reference, None, browser::BrowserChoice::Auto)
        .await?;
    assert_eq!(work.variants.len(), 1);
    assert!(
        work.variants[0]
            .title
            .as_ref()
            .unwrap()
            .contains("Полная версия")
    );
    assert!(work.variants[0].title.as_ref().unwrap().contains("v 1.2"));
    let selected = &work.variants[0].reference;
    let resolved = provider
        .resolve(selected, None, browser::BrowserChoice::Auto)
        .await?;
    let ResolvedContent::TorrentFile {
        reference,
        metadata,
    } = resolved
    else {
        panic!("ByRut did not provide torrent metadata");
    };
    assert_eq!(reference, *selected);
    assert_eq!(metadata.bytes(), legal_torrent());

    fixture.changed.store(true, Ordering::SeqCst);
    let downloads = fixture.downloads.load(Ordering::SeqCst);
    assert!(matches!(
        provider
            .resolve(selected, None, browser::BrowserChoice::Auto)
            .await,
        Err(Error::VariantUnavailable)
    ));
    assert_eq!(fixture.downloads.load(Ordering::SeqCst), downloads + 1);
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
    for id in ["43-identity", "44-challenge", "45-missing"] {
        let reference = PublicationRef {
            source: Source::ByRut,
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
    for id in ["../private", "42-Mixed", "42-", "0-game", "42-game?query"] {
        let reference = PublicationRef {
            source: Source::ByRut,
            id: id.into(),
        };
        assert!(matches!(
            provider
                .inspect(&reference, None, browser::BrowserChoice::Auto)
                .await,
            Err(Error::InvalidReference)
        ));
    }
    let mut wrong = selected.clone();
    wrong.selector = VariantSelector::TorrentHash {
        info_hash: "0123456789abcdef0123456789abcdef01234567".into(),
    };
    assert!(matches!(
        provider
            .resolve(&wrong, None, browser::BrowserChoice::Auto)
            .await,
        Err(Error::VariantUnavailable)
    ));
    assert!(
        provider
            .search(
                "Generated Legal",
                Category::Books,
                None,
                browser::BrowserChoice::Auto
            )
            .await?
            .is_empty()
    );
    fixture.stop().await;
    println!(
        "PASS byrut_fixtures generated_https search inspect exact_torrent replaced_metadata_rejected faults_rejected game_payload=none"
    );
    Ok(())
}

fn legal_torrent() -> &'static [u8] {
    b"d8:announce32:https://fixture.example/announce4:infod6:lengthi4e4:name9:legal.txt12:piece lengthi4e6:pieces20:\xa9J\x8f\xe5\xcc\xb1\x9b\xa6\x1cL\x08s\xd3\x91\xe9\x87\x98/\xbb\xd3ee"
}

fn changed_torrent() -> &'static [u8] {
    b"d8:announce32:https://fixture.example/announce4:infod6:lengthi4e4:name9:other.txt12:piece lengthi4e6:pieces20:\xa9J\x8f\xe5\xcc\xb1\x9b\xa6\x1cL\x08s\xd3\x91\xe9\x87\x98/\xbb\xd3ee"
}

struct Fixture {
    origin: Url,
    client: reqwest::Client,
    changed: Arc<AtomicBool>,
    downloads: Arc<AtomicUsize>,
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
        let downloads = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let changed = Arc::clone(&changed);
            let downloads = Arc::clone(&downloads);
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
                    let downloads = Arc::clone(&downloads);
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
                            .any(|(name, value)| name == "do" && value == "download")
                        {
                            downloads.fetch_add(1, Ordering::SeqCst);
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
            downloads,
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
            return ("403 Forbidden", "text/html", b"error".to_vec());
        }
        let shown = if query == "mismatch" { "other" } else { &query };
        let link = if query == "foreign" {
            "https://clone.example/42-generated-legal.html".into()
        } else {
            format!("{origin}42-generated-legal.html")
        };
        let row = format!(
            "<a class=\"search_res\" href=\"{link}\"><span class=\"search_res_title\">Generated Legal</span><div class=\"search_res_sub\"><span class=\"js-ver\">v 1.2</span></div></a>"
        );
        let result = match query.as_str() {
            "empty" => {
                "<div class=\"berrors_text\">По вашему запросу ничего не найдено</div>".into()
            }
            "duplicate" => row.repeat(2),
            _ => row,
        };
        let body = format!(
            "<html><div id=\"dle-content\"><form id=\"fullsearch\"><input name=\"story\" value=\"{shown}\"></form>{result}</div></html>"
        );
        return ("200 OK", "text/html; charset=utf-8", body.into_bytes());
    }
    if url.path().ends_with(".html") {
        if url.path().contains("missing") {
            return ("404 Not Found", "text/html", Vec::new());
        }
        if url.path().contains("challenge") {
            return ("403 Forbidden", "text/html", b"error".to_vec());
        }
        let slug = if url.path().contains("identity") {
            "42-generated-legal"
        } else {
            url.path().trim_start_matches('/').trim_end_matches(".html")
        };
        let id = 91;
        let body = format!(
            "<html><head><link rel=\"canonical\" href=\"{origin}{slug}.html\"></head><body><article class=\"main_content\" data-news-id=\"42\"><div class=\"game_details\"><h1>Generated Legal</h1></div><div class=\"main-release-block\"><div class=\"info-basictor__row\"><div class=\"info-basictor__item\"><strong class=\"info-basictor__value\">Open fixture</strong></div><div class=\"info-basictor__item info-basictor__item--version\"><strong class=\"info-basictor__value\">v 1.2</strong></div></div><div class=\"info-basictor__item--status\"><strong class=\"info-basictor__value\">Полная версия</strong></div><a class=\"itemdown_games\" href=\"{origin}index.php?do=download&amp;id={id}\">Скачать .torrent</a></div></article></body></html>"
        );
        return ("200 OK", "text/html; charset=utf-8", body.into_bytes());
    }
    if url
        .query_pairs()
        .any(|(name, value)| name == "do" && value == "download")
    {
        let body = if changed {
            changed_torrent()
        } else {
            legal_torrent()
        };
        return ("200 OK", "application/x-bittorrent", body.to_vec());
    }
    ("404 Not Found", "text/html", Vec::new())
}
