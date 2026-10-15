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
const TITLE: &str = "Generated & Legal <edition> 2026";

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
    assert_eq!(found[0].title, TITLE);
    assert!(found[0].needs_inspection && found[0].variants.is_empty());

    let work = source.inspect(&found[0].reference).await?;
    assert_eq!(work.reference, found[0].reference);
    assert_eq!(work.title, TITLE);
    assert_eq!(work.category, Category::Movies);
    assert_eq!(work.variants.len(), 1);
    let variant = &work.variants[0];
    assert_eq!(variant.info_hash.as_deref(), Some(HASH));
    assert_eq!(variant.size, Some(1_310_720));
    assert_eq!(variant.seeders, Some(4));

    let ResolvedContent::Torrent {
        reference,
        title,
        magnet,
    } = source.resolve(&variant.reference).await?
    else {
        panic!("unexpected transport");
    };
    assert_eq!(reference, variant.reference);
    assert_eq!(title, TITLE);
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
    http_retries(&fixture).await?;

    fixture.stop().await;
    println!(
        "PASS limetorrents_fixtures generated_https=ok exact_hash=ok stale=rejected foreign=rejected malformed=rejected empty=ok cancel=connection_closed external_payload=none"
    );
    Ok(())
}

async fn http_retries(fixture: &Fixture) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let http = http::fixture(fixture.client.clone(), fixture.origin.clone());

    let hits = fixture.hits.load(Ordering::SeqCst);
    let (_, body) = http
        .html_page(fixture.origin.join("/retry/transient")?)
        .await?;
    assert_eq!(body, "generated legal response");
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 2);

    let hits = fixture.hits.load(Ordering::SeqCst);
    assert!(matches!(
        http.html_page(fixture.origin.join("/retry/persistent")?)
            .await,
        Err(Error::HttpStatus(503))
    ));
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 2);

    let hits = fixture.hits.load(Ordering::SeqCst);
    let mut session = http::SourceSession::anonymous(fixture.origin.clone())?;
    let raw = http
        .session_document(fixture.origin.join("/retry/persistent")?, &mut session)
        .await?;
    assert_eq!(raw.status, 503);
    assert_eq!(raw.body, b"generated legal response");
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 2);

    for path in ["/retry/long-backoff", "/retry/date-backoff"] {
        let hits = fixture.hits.load(Ordering::SeqCst);
        assert!(matches!(
            http.html_page(fixture.origin.join(path)?).await,
            Err(Error::HttpStatus(503))
        ));
        assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 1);
    }

    for status in [403, 429] {
        let hits = fixture.hits.load(Ordering::SeqCst);
        assert!(matches!(
            http.html_page(fixture.origin.join(&format!("/retry/status/{status}"))?)
                .await,
            Err(Error::HttpStatus(actual)) if actual == status
        ));
        assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 1);
    }

    let hits = fixture.hits.load(Ordering::SeqCst);
    let raw = http
        .session_form_document(
            fixture.origin.join("/retry/post")?,
            "legal=fixture",
            &mut session,
        )
        .await?;
    assert_eq!(raw.status, 503);
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 1);

    fixture.transient.store(true, Ordering::SeqCst);
    let hits = fixture.hits.load(Ordering::SeqCst);
    let (arrived, _) = http
        .html_page(fixture.origin.join("/retry/redirect/0")?)
        .await?;
    assert_eq!(arrived.path(), "/retry/redirect/5");
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 7);

    fixture.transient.store(true, Ordering::SeqCst);
    let hits = fixture.hits.load(Ordering::SeqCst);
    let raw = http
        .session_document(
            fixture.origin.join("/retry/redirect/0?persistent=1")?,
            &mut session,
        )
        .await?;
    assert_eq!(raw.status, 503);
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 7);

    let hits = fixture.hits.load(Ordering::SeqCst);
    let mut pending = Box::pin(http.html_page(fixture.origin.join("/retry/cancel")?));
    tokio::select! {
        started = tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.hits.load(Ordering::SeqCst) == hits {
                tokio::task::yield_now().await;
            }
        }) => started?,
        _ = &mut pending => panic!("retry cancellation completed before its first request"),
    }
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        _ = &mut pending => panic!("retry cancellation did not wait for Retry-After"),
    }
    drop(pending);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits + 1);
    println!(
        "PASS source_http_retry transient503=requests2 persistent503=requests2 raw503=preserved post503=request1 backoff=honored redirects=preserved cancel_delay=request1 statuses403429=unchanged"
    );
    Ok(())
}

fn search_page(query: &str) -> String {
    let row = format!(
        r##"<tr bgcolor="#ffffff">
            <td><div class="tt-name"><a href="/{ID}">Generated &amp;amp; Legal &lt;edition&gt; &amp;#50;026</a></div></td>
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
            <h1>Generated &amp; Legal &lt;edition&gt; 2026</h1>
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
    transient: Arc<AtomicBool>,
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
        let transient = Arc::new(AtomicBool::new(true));
        let hits = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let payload = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let changed = Arc::clone(&changed);
            let transient = Arc::clone(&transient);
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
                    let transient = Arc::clone(&transient);
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
                        if path.starts_with("/retry/") {
                            let mut headers = String::new();
                            let status = match path {
                                "/retry/transient" => {
                                    if transient.swap(false, Ordering::SeqCst) { 503 } else { 200 }
                                }
                                "/retry/long-backoff" => {
                                    headers = "Retry-After: 30\r\n".into();
                                    503
                                }
                                "/retry/date-backoff" => {
                                    headers = "Retry-After: Thu, 15 Oct 2026 15:00:00 GMT\r\n".into();
                                    503
                                }
                                "/retry/cancel" => {
                                    headers = "Retry-After: 2\r\n".into();
                                    503
                                }
                                "/retry/status/403" => 403,
                                "/retry/status/429" => 429,
                                p if p.starts_with("/retry/redirect/") => {
                                    let hop: usize = p.rsplit('/').next().unwrap().parse().unwrap();
                                    if hop == 0 && transient.swap(false, Ordering::SeqCst) {
                                        503
                                    } else if hop < 5 {
                                        let mut next = url.clone();
                                        next.set_path(&format!("/retry/redirect/{}", hop + 1));
                                        headers = format!("Location: {next}\r\n");
                                        302
                                    } else if url.query() == Some("persistent=1") {
                                        503
                                    } else {
                                        200
                                    }
                                }
                                _ => 503,
                            };
                            let body = b"generated legal response";
                            let header = format!(
                                "HTTP/1.1 {status} Fixture\r\n{headers}Content-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(header.as_bytes()).await;
                            let _ = stream.write_all(body).await;
                            let _ = stream.shutdown().await;
                            return;
                        }
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
            transient,
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
