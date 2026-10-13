//! Generated HTTPS pages exercise the production provider without torrent payloads.

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
#[path = "../src/torrent.rs"]
mod torrent;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod limetorrents {
    include!("../src/limetorrents.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> LimeTorrents {
        LimeTorrents { http, origin }
    }
}

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_HASH: &str = "abcdef0123456789abcdef0123456789abcdef01";
const ID: &str = "Generated-Legal-torrent-42.html";

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::start().await?;
    let source = limetorrents::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );

    let found = source.search("generated & legal", Category::Movies).await?;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].reference.id, ID);
    assert!(found[0].needs_inspection && found[0].variants.is_empty());

    let work = source.inspect(&found[0].reference).await?;
    assert_eq!(work.reference, found[0].reference);
    assert_eq!(work.category, Category::Movies);
    assert_eq!(work.variants.len(), 1);
    let variant = &work.variants[0];
    assert_eq!(variant.info_hash.as_deref(), Some(HASH));
    assert_eq!(variant.size, Some(1_310_720));
    assert_eq!(variant.seeders, Some(4));

    let ResolvedContent::Torrent {
        reference, magnet, ..
    } = source.resolve(&variant.reference).await?
    else {
        panic!("unexpected transport");
    };
    assert_eq!(reference, variant.reference);
    assert!(
        magnet
            .query_pairs()
            .any(|(key, value)| { key == "xt" && value == format!("urn:btih:{HASH}") })
    );

    fixture.changed.store(true, Ordering::SeqCst);
    assert!(matches!(
        source.resolve(&variant.reference).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.changed.store(false, Ordering::SeqCst);

    for id in ["Missing-torrent-404.html", "Gone-torrent-410.html"] {
        let missing = PublicationRef {
            source: Source::LimeTorrents,
            id: id.into(),
        };
        assert!(matches!(
            source.inspect(&missing).await,
            Err(Error::VariantUnavailable)
        ));
    }
    let invalid = PublicationRef {
        source: Source::LimeTorrents,
        id: "BadHash-torrent-44.html".into(),
    };
    assert!(matches!(
        source.inspect(&invalid).await,
        Err(Error::InvalidResponse(_))
    ));
    let advertised_only = PublicationRef {
        source: Source::LimeTorrents,
        id: "ExternalOnly-torrent-45.html".into(),
    };
    assert!(matches!(
        source.inspect(&advertised_only).await,
        Err(Error::VariantUnavailable)
    ));

    assert!(source.search("empty", Category::Movies).await?.is_empty());
    assert!(
        source
            .search("generated", Category::Games)
            .await?
            .is_empty()
    );
    for query in ["duplicate", "foreign", "challenge", "wrong-query"] {
        let result = source.search(query, Category::Movies).await;
        assert!(
            matches!(
                &result,
                Err(Error::InvalidResponse(_) | Error::InvalidReference)
            ),
            "fault {query}: {result:?}"
        );
    }
    let hits = fixture.hits.load(Ordering::SeqCst);
    for query in ["", "bad\nquery", &"x".repeat(501)] {
        assert!(matches!(
            source.search(query, Category::Movies).await,
            Err(Error::InvalidQuery)
        ));
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits);
    assert!(
        source
            .search("generated", Category::Manga)
            .await?
            .is_empty()
    );
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits);

    let mut pending = Box::pin(source.search("cancel", Category::Movies));
    tokio::select! {
        started = tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.cancelled.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        }) => started?,
        _ = &mut pending => panic!("cancel fixture completed before request"),
    }
    drop(pending);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert_eq!(fixture.payload.load(Ordering::SeqCst), 0);

    fixture.stop().await;
    println!(
        "PASS limetorrents_fixtures generated_https=ok exact_hash=ok stale=rejected foreign=rejected malformed=rejected empty=ok cancel=connection_closed external_payload=none"
    );
    Ok(())
}

fn search_page(query: &str) -> String {
    let row = format!(
        r##"<tr bgcolor="#ffffff">
            <td><div class="tt-name"><a href="/{ID}">Generated Legal</a></div></td>
            <td>Today - in Movies</td><td>1.25 MB</td>
            <td>4</td><td>0</td><td>healthy</td>
        </tr>"##
    );
    let body = match query {
        "empty" => "<h2>No results found</h2>".to_owned(),
        "duplicate" => format!(
            "<table class=table2><tr><th class=thleft><span>Torrent Name</span></th></tr>{row}{row}</table>"
        ),
        "foreign" => format!(
            "<table class=table2><tr><th class=thleft><span>Torrent Name</span></th></tr>{}</table>",
            row.replace(
                &format!("/{ID}"),
                "https://ads.example/Foreign-torrent-42.html"
            )
        ),
        _ => format!(
            "<table class=table2><tr><th class=thleft><span>Torrent Name</span></th></tr>{row}</table>"
        ),
    };
    let label = if query == "wrong query" {
        "another"
    } else {
        query
    };
    format!(r#"<div id="content"><span class="querytext">{label}</span>{body}</div>"#)
}

fn detail_page(hash: &str, magnet_hash: Option<&str>) -> String {
    let magnet = magnet_hash.map_or(String::new(), |value| {
        format!(r#"<a href="magnet:?xt=urn:btih:{value}&amp;dn=Generated+Legal">Magnet</a>"#)
    });
    format!(
        r#"<div id="content">
            <h1>Generated Legal</h1>
            <span class="greenish">Seeders : 4</span>
            <div class="torrentinfo"><table>
                <tr><td>Torrent Hash :</td><td>{hash}</td></tr>
                <tr><td>Torrent Added :</td><td><a href="/browse-torrents/Movies">Movies</a></td></tr>
                <tr><td>Torrent Size :</td><td>1.25 MB</td></tr>
            </table></div>
            <div class="downloadarea">
                {magnet}
                <a href="https://ads.example/payload.torrent">Sponsored download</a>
            </div>
        </div>"#
    )
}

struct Fixture {
    origin: Url,
    client: reqwest::Client,
    changed: Arc<AtomicBool>,
    hits: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    payload: Arc<AtomicUsize>,
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
        let hits = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let payload = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let changed = Arc::clone(&changed);
            let hits = Arc::clone(&hits);
            let cancelled = Arc::clone(&cancelled);
            let payload = Arc::clone(&payload);
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
                    let hits = Arc::clone(&hits);
                    let cancelled = Arc::clone(&cancelled);
                    let payload = Arc::clone(&payload);
                    clients.spawn(async move {
                        let Ok(mut stream) = acceptor.accept(socket).await else { return };
                        let mut request = Vec::new();
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                            let mut byte = [0];
                            if stream.read_exact(&mut byte).await.is_err() { return }
                            request.push(byte[0]);
                        }
                        hits.fetch_add(1, Ordering::SeqCst);
                        let target = String::from_utf8_lossy(&request)
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("")
                            .to_owned();
                        let Ok(url) = origin.join(&target) else { return };
                        let path = url.path();
                        let query = path.split('/').nth(3).unwrap_or("");
                        let query = percent_encoding::percent_decode_str(query)
                            .decode_utf8_lossy()
                            .into_owned();

                        if query == "cancel" {
                            cancelled.store(1, Ordering::SeqCst);
                            let mut byte = [0];
                            let _ = stream.read(&mut byte).await;
                            cancelled.store(2, Ordering::SeqCst);
                            return;
                        }
                        if path.ends_with(".torrent") {
                            payload.fetch_add(1, Ordering::SeqCst);
                        }
                        let (status, body) = match path {
                            "/Missing-torrent-404.html" => ("404 Not Found", String::new()),
                            "/Gone-torrent-410.html" => ("410 Gone", String::new()),
                            "/BadHash-torrent-44.html" => ("200 OK", detail_page(HASH, Some(OTHER_HASH))),
                            "/ExternalOnly-torrent-45.html" => ("200 OK", detail_page(HASH, None)),
                            p if p == format!("/{ID}") => {
                                let hash = if changed.load(Ordering::SeqCst) { OTHER_HASH } else { HASH };
                                ("200 OK", detail_page(hash, Some(hash)))
                            }
                            _ if query == "challenge" => ("200 OK", "<h1>Checking your browser</h1>".into()),
                            _ => ("200 OK", search_page(&query)),
                        };
                        let bytes = body.into_bytes();
                        let header = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
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
            hits,
            cancelled,
            payload,
            stop,
            server,
        })
    }

    async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.expect("TLS fixture stopped");
    }
}
