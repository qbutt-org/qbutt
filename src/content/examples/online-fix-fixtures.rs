//! Generated legal HTTPS pages and torrent metadata exercise the native source.

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

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[path = "../src/torrent.rs"]
#[allow(dead_code)]
mod torrent;

#[allow(dead_code)]
mod online_fix {
    include!("../src/online_fix.rs");

    pub(super) fn fixture(
        site: SourceHttp,
        uploads: SourceHttp,
        origin: Url,
        uploads_origin: Url,
    ) -> OnlineFix {
        OnlineFix {
            site,
            uploads,
            origin,
            uploads_origin,
        }
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
    let source = online_fix::fixture(
        http::fixture(fixture.client.clone(), fixture.site.clone()),
        http::fixture(fixture.client.clone(), fixture.uploads.clone()),
        fixture.site.clone(),
        fixture.uploads.clone(),
    );

    let results = source.search("Generated Game", Category::Games).await?;
    assert_eq!(results.len(), 1);
    assert!(results[0].needs_inspection && results[0].variants.is_empty());
    let work = source.inspect(&results[0].reference).await?;
    assert_eq!(work.title, "Generated Game");
    assert_eq!(work.variants.len(), 1);
    assert_eq!(work.variants[0].transport, Transport::Torrent);
    assert!(
        work.variants[0]
            .title
            .as_deref()
            .unwrap()
            .starts_with("Full game:")
    );
    let selected = work.variants[0].reference.clone();
    assert_eq!(
        selected.selector,
        VariantSelector::TorrentHash {
            info_hash: "7b8f2a3307bc1d3f386312a4cbfb49bd58fc4ad2".into(),
        }
    );
    let ResolvedContent::TorrentFile {
        reference,
        metadata,
    } = source.resolve(&selected).await?
    else {
        panic!("expected exact torrent metadata");
    };
    assert_eq!(reference, selected);
    assert_eq!(metadata.bytes(), torrent_bytes(false));
    reject_invalid_metadata();

    fixture.changed.store(true, Ordering::SeqCst);
    let updated = source.inspect(&work.reference).await?;
    assert_ne!(updated.variants[0].reference, selected);
    assert!(matches!(
        source.resolve(&selected).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.changed.store(false, Ordering::SeqCst);
    assert!(
        source
            .search("Unlikely Result", Category::Games)
            .await?
            .is_empty()
    );
    assert!(source.search("AI", Category::Games).await?.is_empty());
    assert!(
        tokio::time::timeout(
            Duration::from_millis(25),
            source.search("Slow Search", Category::Games),
        )
        .await
        .is_err()
    );
    assert_eq!(
        source
            .search("Generated Game", Category::Games)
            .await?
            .len(),
        1
    );
    assert!(
        source
            .search("Generated Game", Category::Movies)
            .await?
            .is_empty()
    );
    assert!(matches!(
        source
            .inspect(&PublicationRef {
                source: Source::OnlineFix,
                id: "games/horror/999-missing.html".into(),
            })
            .await,
        Err(Error::VariantUnavailable)
    ));
    assert_eq!(fixture.archive_requests.load(Ordering::SeqCst), 0);

    println!(
        "PASS onlinefix search inspect exact_torrent stale_selection empty missing cancel no_archive"
    );
    fixture.stop().await;
    Ok(())
}

fn torrent_bytes(changed: bool) -> Vec<u8> {
    let name = if changed {
        "Generated Game 2"
    } else {
        "Generated Game"
    };
    let mut info = format!(
        "d6:lengthi42e4:name{}:{name}12:piece lengthi16384e6:pieces20:",
        name.len()
    )
    .into_bytes();
    info.extend_from_slice(b"01234567890123456789e");
    let announce = "https://tracker.example/announce";
    let mut torrent = format!("d8:announce{}:{announce}4:info", announce.len()).into_bytes();
    torrent.extend_from_slice(&info);
    torrent.push(b'e');
    torrent
}

fn reject_invalid_metadata() {
    let original = String::from_utf8(torrent_bytes(false)).expect("ASCII metadata fixture");
    let invalid = [
        original.replace("6:lengthi42e", "6:lengthi-e"),
        original.replace("6:lengthi42e", "6:lengthi-0e"),
        original.replace("6:lengthi42e", "6:lengthi01e"),
        original.replace("4:name14:", "4:name014:"),
        original.replace("4:info", "8:announce0:4:info"),
        original.replace("6:pieces20:01234567890123456789", "6:piecesli1ee"),
    ];
    for bytes in invalid {
        assert!(torrent::info_hash(bytes.as_bytes()).is_err());
    }
}

struct Fixture {
    site: Url,
    uploads: Url,
    client: reqwest::Client,
    changed: Arc<AtomicBool>,
    archive_requests: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let certificate = rcgen::generate_simple_self_signed(vec![
            "site.example".into(),
            "uploads.example".into(),
        ])?;
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
            )?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let site = Url::parse(&format!("https://site.example:{}/", address.port()))?;
        let uploads = Url::parse(&format!("https://uploads.example:{}/", address.port()))?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(reqwest::Certificate::from_der(certificate.cert.der())?)
            .resolve("site.example", address)
            .resolve("uploads.example", address)
            .build()?;
        let changed = Arc::new(AtomicBool::new(false));
        let archive_requests = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let site = site.clone();
            let uploads = uploads.clone();
            let changed = Arc::clone(&changed);
            let archive_requests = Arc::clone(&archive_requests);
            async move {
                loop {
                    let socket = tokio::select! {
                        _ = &mut stopped => break,
                        socket = listener.accept() => match socket {
                            Ok((socket, _)) => socket,
                            Err(_) => break,
                        },
                    };
                    let acceptor = acceptor.clone();
                    let site = site.clone();
                    let uploads = uploads.clone();
                    let changed = Arc::clone(&changed);
                    let archive_requests = Arc::clone(&archive_requests);
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
                        let request = String::from_utf8_lossy(&request);
                        let path = request.split_whitespace().nth(1).unwrap_or("");
                        if path.contains("Slow") {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                        let (status, mime, body) = route(
                            path,
                            &site,
                            &uploads,
                            changed.load(Ordering::SeqCst),
                            &archive_requests,
                        );
                        let header = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(&body).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });
        Ok(Self {
            site,
            uploads,
            client,
            changed,
            archive_requests,
            stop,
            server,
        })
    }

    async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.expect("fixture server stopped");
    }
}

fn route(
    path: &str,
    site: &Url,
    uploads: &Url,
    changed: bool,
    archives: &AtomicUsize,
) -> (&'static str, &'static str, Vec<u8>) {
    let url = site.join(path).expect("fixture route");
    let query = url
        .query_pairs()
        .find(|(name, _)| name == "story")
        .map(|(_, value)| value.into_owned());
    if url.path() == "/index.php" {
        assert!(
            url.query_pairs()
                .any(|(name, value)| name == "titleonly" && value == "3"),
            "Online-Fix search must request matching titles"
        );
        let query = query.unwrap_or_default();
        let results = if query == "Unlikely Result" {
            "К сожалению, поиск по сайту не дал никаких результатов.".to_owned()
        } else {
            format!(
                "<div class=\"news news-search\"><a href=\"{site}games/horror/18094-generated-game.html\"><h2 class=\"title\">Generated Game по сети</h2></a></div>"
            )
        };
        return (
            "200 OK",
            "text/html; charset=windows-1251",
            cp1251(&format!(
                "<div id=\"dle-content\"><input id=\"searchinput\" name=\"story\" value=\"{query}\">{results}</div>"
            )),
        );
    }
    if url.path() == "/games/horror/999-missing.html" {
        return (
            "404 Not Found",
            "text/html; charset=windows-1251",
            Vec::new(),
        );
    }
    if url.path() == "/games/horror/18094-generated-game.html" {
        let body = format!(
            "<link rel=\"canonical\" href=\"{site}games/horror/18094-generated-game.html\"><div class=\"full-story\"><h1 id=\"news-title\">Generated Game по сети</h1><div itemprop=\"articleBody\"><a href=\"{uploads}torrents/Generated%20Game/\">Скачать Torrent</a><a href=\"{uploads}uploads/Generated%20Game/Fix%20Repair/\">Скачать фикс с сервера</a></div></div>"
        );
        return ("200 OK", "text/html; charset=windows-1251", cp1251(&body));
    }
    if url.path() == "/torrents/Generated%20Game/" {
        return ("200 OK", "text/html; charset=utf-8", b"<h1>Index of /torrents/Generated Game/</h1><pre><a href=\"Generated.Game.v1.torrent\">Generated.Game.v1.torrent</a></pre>".to_vec());
    }
    if url.path() == "/torrents/Generated%20Game/Generated.Game.v1.torrent" {
        return ("200 OK", "application/octet-stream", torrent_bytes(changed));
    }
    archives.fetch_add(1, Ordering::SeqCst);
    ("404 Not Found", "text/plain", Vec::new())
}

fn cp1251(text: &str) -> Vec<u8> {
    let (bytes, _, invalid) = encoding_rs::WINDOWS_1251.encode(text);
    assert!(!invalid);
    bytes.into_owned()
}
