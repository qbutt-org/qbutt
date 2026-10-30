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
    if path == "/index.php" && params.contains_key("story") {
        let story = params.get("story").unwrap().replace(':', "");
        if story == "forbidden" {
            return (
                "403 Forbidden",
                "text/html; charset=utf-8",
                b"error".to_vec(),
            );
        }
        if story == "server-error" {
            return (
                "503 Service Unavailable",
                "text/html; charset=utf-8",
                b"error".to_vec(),
            );
        }
        if story == "challenge" {
            return (
                "200 OK",
                "text/html; charset=utf-8",
                b"<h1>Checking your browser</h1>".to_vec(),
            );
        }
        let result = if matches!(story.as_str(), "generated" | "generatedlegal" | "КЛЕТЬ") {
            r#"<article class="shortstory">
    <h2 class="game-card__title">
        <a href="/po-seti/42-generated-game.html">Generated Legal Game играть по сети и интернету Онлайн</a>
    </h2>
</article>"#
        } else {
            ""
        };
        let shown = if story == "mismatch" {
            "unrelated"
        } else {
            story.as_str()
        };
        let notice = if story == "limited" {
            "Вы сможете воспользоваться поиском через 10 секунд"
        } else if result.is_empty() && story != "malformed-empty" {
            "К сожалению, поиск по сайту не дал никаких результатов. Попробуйте изменить или сократить Ваш запрос."
        } else {
            ""
        };
        let started = if story == "unstarted" {
            "1"
        } else {
            params.get("search_start").map(String::as_str).unwrap_or("")
        };
        let body = format!(
            r#"<section id="content">
    <div class="ftp-info">{notice}</div>
    <form id="fullsearch">
        <input name="story" value="{shown}">
        <input name="search_start" value="{started}">
        <article class="searchpage"><h1 class="title">Поиск по сайту</h1></article>
    </form>
    {result}
</section>"#
        );
        return ("200 OK", "text/html; charset=utf-8", body.into_bytes());
    }
    if path == "/index.php" && params.contains_key("newsid") {
        let body = r#"<link rel="canonical" href="/po-seti/42-generated-game.html">
<section id="content">
    <article class="fullstory">
        <h2 class="game-page__title">Generated Legal Game играть по сети и интернету Онлайн</h2>
        <div class="game-page__content">
            <p>Legal game and setup instructions.</p>
            <p>Скачать игру</p>
            <p><a href="/getfile-10">Скачать Generated_Game_v2.torrent</a><a href="/getfile-12"> </a></p>
            <p>Скачать патч</p>
            <p><a href="/getfile-13">Generated_Patch_v2.torrent</a></p>
            <p>Скачать онлайн фикс</p>
            <p><a href="/getfile-11">Online Fix.exe</a></p>
        </div>
    </article>
</section>"#;
        return (
            "200 OK",
            "text/html; charset=utf-8",
            body.as_bytes().to_vec(),
        );
    }
    if let Some(id) = path.strip_prefix("/getfile-") {
        let name = match id {
            "10" => "Generated_Game_v2.torrent",
            "11" => "Online_Fix.exe",
            "12" => "Unrelated_Game.torrent",
            "13" => "Generated_Patch_v2.torrent",
            _ => return ("404 Not Found", "text/html; charset=utf-8", Vec::new()),
        };
        let body = format!(r#"<h1 class="ftp-dl-title">Скачать файл <b>{name}</b></h1>"#);
        return ("200 OK", "text/html; charset=utf-8", body.into_bytes());
    }
    if path == "/index.php" && params.get("do").is_some_and(|value| value == "download") {
        let bytes = match params.get("id").map(String::as_str) {
            Some("10") => metadata(if changed {
                "Changed Game"
            } else {
                "Generated Game"
            }),
            Some("13") => metadata("Generated Patch"),
            _ => return ("404 Not Found", "text/html; charset=utf-8", Vec::new()),
        };
        return ("200 OK", "application/x-bittorrent", bytes);
    }
    ("404 Not Found", "text/html; charset=utf-8", Vec::new())
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
                        let (status, mime, bytes) = if url.path() == "/index.php"
                            && params.iter().any(|(name, value)| name == "do" && value == "download")
                            && !referred
                        {
                            ("403 Forbidden", "text/html; charset=utf-8", Vec::new())
                        } else {
                            page(url.path(), &params, changed.load(Ordering::SeqCst))
                        };
                        let header = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
    assert_eq!(found[0].aliases, ["Generated Legal Game"]);
    assert!(found[0].needs_inspection && found[0].variants.is_empty());
    let publication = found[0].reference.clone();
    let work = tokio::spawn({
        let source = Arc::clone(&source);
        async move { source.inspect(&publication).await }
    })
    .await??;
    assert_eq!(work.variants.len(), 2);
    assert_eq!(work.aliases, found[0].aliases);
    assert_eq!(work.title, found[0].title);
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
    for query in ["absent", "Re:Zero", "Acon Acoustica 8"] {
        assert!(source.search(query, Category::Games).await?.is_empty());
    }
    let normalized = source.search("generated:legal", Category::Games).await?;
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].reference, found[0].reference);
    let cyrillic = source.search("КЛЕТЬ", Category::Games).await?;
    assert_eq!(cyrillic.len(), 1);
    assert_eq!(cyrillic[0].reference, found[0].reference);
    for query in [":", ": :"] {
        assert!(matches!(
            source.search(query, Category::Games).await,
            Err(Error::InvalidQuery)
        ));
    }
    assert!(matches!(
        source.search("mismatch", Category::Games).await,
        Err(Error::InvalidResponse("FreeTP search was not confirmed"))
    ));
    assert!(matches!(
        source.search("unstarted", Category::Games).await,
        Err(Error::InvalidResponse("FreeTP search was not confirmed"))
    ));
    assert!(matches!(
        source.search("malformed-empty", Category::Games).await,
        Err(Error::InvalidResponse("missing FreeTP search results"))
    ));
    assert!(matches!(
        source
            .inspect(&PublicationRef {
                source: Source::FreeTP,
                id: "43".into()
            })
            .await,
        Err(Error::VariantUnavailable)
    ));
    assert!(matches!(
        source.search("forbidden", Category::Games).await,
        Err(Error::Challenge)
    ));
    assert!(matches!(
        source.search("server-error", Category::Games).await,
        Err(Error::HttpStatus(503))
    ));
    assert!(matches!(
        source.search("limited", Category::Games).await,
        Err(Error::InvalidResponse(
            "FreeTP search is temporarily limited"
        ))
    ));
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
        "PASS freetp_fixture search utf8_cyrillic colon_normalized positive empty_acoustica_rezero mismatch_rejected unstarted_rejected malformed_empty_rejected http_challenge_preserved inspect canonical_identity game patch exact_resolve changed_hash limited cancel"
    );
    Ok(())
}
