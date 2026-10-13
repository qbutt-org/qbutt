//! Generated HTTPS articles and tiny legal torrents exercise the real provider.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
#[cfg(windows)]
pub use qbutt_content::webview;

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
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
mod freetp {
    include!("../src/freetp.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> FreeTP {
        FreeTP { http, origin }
    }
}

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

fn metadata(name: &str) -> Vec<u8> {
    let info = format!(
        "d6:lengthi1e4:name{}:{}12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae",
        name.len(),
        name
    );
    format!("d4:info{info}e").into_bytes()
}

fn page(
    path: &str,
    query: &[(String, String)],
    changed: bool,
) -> (&'static str, &'static str, Vec<u8>) {
    let params = query
        .iter()
        .cloned()
        .collect::<std::collections::BTreeMap<_, _>>();
    if path == "/index.php" && params.contains_key("newsid") {
        return ("302 Found", "text/html; charset=cp1251", Vec::new());
    }
    if path == "/index.php" && params.contains_key("story") {
        let story = params.get("story").unwrap();
        if story == "challenge" {
            return (
                "200 OK",
                "text/html; charset=cp1251",
                b"<h1>Checking your browser</h1>".to_vec(),
            );
        }
        let result = if story == "generated" {
            r#"<div class="base"><div class="heading"><a href="/po-seti/42-generated-game.html">Generated Legal Game играть по сети</a></div></div>"#
        } else {
            ""
        };
        let body = format!(
            r#"<div id="dle-content"><h1 class="heading3">Поиск по сайту</h1><form id="fullsearch"><input name="story" value="{story}"><input name="search_start" value="1"></form>{result}</div>"#
        );
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(&body);
        return ("200 OK", "text/html; charset=cp1251", bytes.into_owned());
    }
    if path == "/po-seti/42-generated-game.html" {
        let body = r#"<div id="dle-content"><div class="base fullstory"><h1>Generated Legal Game играть по сети</h1><div id="news-id-42"><p>Legal game and setup instructions.</p><p>Скачать игру</p><p><a href="/getfile-10">Скачать Generated_Game_v2.torrent</a><a href="/getfile-12"> </a></p><p>Скачать патч</p><p><a href="/getfile-13">Generated_Patch_v2.torrent</a></p><p>Скачать онлайн фикс</p><p><a href="/getfile-11">Online Fix.exe</a></p></div></div></div>"#;
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(body);
        return ("200 OK", "text/html; charset=cp1251", bytes.into_owned());
    }
    if let Some(id) = path.strip_prefix("/getfile-") {
        let name = match id {
            "10" => "Generated_Game_v2.torrent",
            "11" => "Online_Fix.exe",
            "12" => "Unrelated_Game.torrent",
            "13" => "Generated_Patch_v2.torrent",
            _ => return ("404 Not Found", "text/html; charset=cp1251", Vec::new()),
        };
        let body = format!("<title>{name} скачать бесплатно</title>");
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(&body);
        return ("200 OK", "text/html; charset=cp1251", bytes.into_owned());
    }
    if path == "/engine/download.php" {
        let bytes = match params.get("id").map(String::as_str) {
            Some("10") => metadata(if changed {
                "Changed Game"
            } else {
                "Generated Game"
            }),
            Some("13") => metadata("Generated Patch"),
            _ => return ("404 Not Found", "text/html; charset=cp1251", Vec::new()),
        };
        return ("200 OK", "application/force-download", bytes);
    }
    ("404 Not Found", "text/html; charset=cp1251", Vec::new())
}

struct Fixture {
    origin: Url,
    client: reqwest::Client,
    changed: Arc<AtomicBool>,
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
            .timeout(Duration::from_secs(3))
            .build()?;
        let changed = Arc::new(AtomicBool::new(false));
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let changed = Arc::clone(&changed);
            async move {
                let mut clients = JoinSet::new();
                loop {
                    let socket = tokio::select! {
                        _ = &mut stopped => break,
                        socket = listener.accept() => match socket {
                            Ok((socket, _)) => socket,
                            Err(_) => break,
                        },
                    };
                    let acceptor = acceptor.clone();
                    let origin = origin.clone();
                    let changed = Arc::clone(&changed);
                    clients.spawn(async move {
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
                        let request = String::from_utf8_lossy(&request);
                        let target = request
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("")
                            .to_owned();
                        let Ok(url) = origin.join(&target) else {
                            return;
                        };
                        let params = url.query_pairs()
                            .map(|(name, value)| (name.into_owned(), value.into_owned()))
                            .collect::<Vec<_>>();
                        if params.iter().any(|(name, value)| name == "story" && value == "hold") {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                        let expected_referer = params
                            .iter()
                            .find(|(name, _)| name == "id")
                            .map(|(_, id)| format!("Referer: {origin}getfile-{id}"));
                        let referred = expected_referer.as_ref().is_some_and(|expected| {
                            request.lines().any(|line| line.trim().eq_ignore_ascii_case(expected))
                        });
                        let (status, mime, bytes) = if url.path() == "/engine/download.php" && !referred {
                            ("403 Forbidden", "text/html; charset=cp1251", Vec::new())
                        } else {
                            page(url.path(), &params, changed.load(Ordering::SeqCst))
                        };
                        let location = if url.path() == "/index.php"
                            && params.iter().any(|(name, _)| name == "newsid")
                        {
                            "Location: /po-seti/42-generated-game.html\r\n"
                        } else {
                            ""
                        };
                        let header = format!(
                            "HTTP/1.1 {status}\r\n{location}Content-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        );
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(&bytes).await;
                        let _ = stream.shutdown().await;
                    });
                }
                clients.abort_all();
                while clients.join_next().await.is_some() {}
            }
        });
        Ok(Self {
            origin,
            client,
            changed,
            stop,
            server,
        })
    }

    async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.expect("TLS fixture stopped");
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::start().await?;
    let source = Arc::new(freetp::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    ));
    let found = tokio::spawn({
        let source = Arc::clone(&source);
        async move { source.search("generated", Category::Games).await }
    })
    .await??;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].reference.id, "42");
    assert!(found[0].needs_inspection && found[0].variants.is_empty());
    let publication = found[0].reference.clone();
    let work = tokio::spawn({
        let source = Arc::clone(&source);
        async move { source.inspect(&publication).await }
    })
    .await??;
    assert_eq!(work.variants.len(), 2);
    assert!(
        work.variants[0]
            .title
            .as_deref()
            .unwrap()
            .starts_with("Игра:")
    );
    assert!(
        work.variants[1]
            .title
            .as_deref()
            .unwrap()
            .starts_with("Патч:")
    );
    for variant in &work.variants {
        let selected = variant.reference.clone();
        let ResolvedContent::TorrentFile {
            reference,
            metadata,
        } = tokio::spawn({
            let source = Arc::clone(&source);
            async move { source.resolve(&selected).await }
        })
        .await??
        else {
            panic!("expected torrent metadata");
        };
        assert_eq!(reference, variant.reference);
        assert_eq!(
            torrent::info_hash(metadata.bytes())?,
            variant.info_hash.as_ref().unwrap().as_str()
        );
    }
    fixture.changed.store(true, Ordering::SeqCst);
    assert!(matches!(
        source.resolve(&work.variants[0].reference).await,
        Err(Error::VariantUnavailable)
    ));
    assert!(source.resolve(&work.variants[1].reference).await.is_ok());
    assert!(source.search("absent", Category::Games).await?.is_empty());
    assert!(source.search("challenge", Category::Games).await.is_err());
    assert!(
        tokio::time::timeout(
            Duration::from_millis(30),
            source.search("hold", Category::Games)
        )
        .await
        .is_err()
    );
    fixture.stop().await;
    println!(
        "PASS freetp_fixture search inspect game patch exact_resolve changed_hash empty challenge cancel"
    );
    Ok(())
}
