//! Generated legal catalog traffic through the production provider and HTTP
//! parser. The private TLS client belongs only to this integration executable.

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
mod torrent;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod nyaa {
    include!("../src/nyaa.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> Nyaa {
        Nyaa { http, origin }
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
const OTHER: &str = "abcdef0123456789abcdef0123456789abcdef01";
const MARKDOWN: &str = r#"# Generated instructions

Keep **all** files, then *verify* them.

1. Run `qbutt --check`.
2. Read [guide](https://example.test/guide?edition=1&lang=en).

> Do not overwrite unrelated files.

- Preserve:
  - video.mkv
  - subtitles.ass

- [x] Download complete
- [ ] Verify files

| Track | Language |
| --- | --- |
| Video | Japanese |

```text
echo "**literal** <tag>"
```

[![Map](/download/map.png)](https://example.test/map)

<div>Plain <strong>HTML</strong> and <a href="/help">relative guide</a>.</div>

<noscript>Fallback instructions remain visible.</noscript>

<script>SHOULD_NOT_APPEAR</script>
<style>SHOULD_NOT_APPEAR</style>
<iframe src="/download/frame">SHOULD_NOT_APPEAR</iframe>

[Unsafe](javascript:alert(1)) and [local file](file:///C:/private.txt).
"#;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::start().await?;
    let source = nyaa::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let results = source.search("generated & legal", Category::Anime).await?;
    assert_eq!(results.len(), 1);
    let found = &results[0];
    assert_eq!(found.title, "[Generated] Legal & sample - 02v2 [1080p]");
    assert_eq!(found.variants[0].episode.as_deref(), Some("2"));
    assert_eq!(found.variants[0].quality.as_deref(), Some("1080p"));
    assert_eq!(found.variants[0].languages, ["en"]);
    assert_eq!(found.variants[0].size, Some(734_024_171));
    assert_eq!(found.variants[0].seeders, Some(12));
    assert!(found.needs_inspection);
    let work = source.inspect(&found.reference).await?;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.variants[0].reference, found.variants[0].reference);
    assert!(!work.needs_inspection);
    assert_eq!(
        work.description.as_deref(),
        Some("Generated public-domain sample")
    );
    let markdown_reference = PublicationRef {
        source: Source::Nyaa,
        id: "44".into(),
    };
    let before_description = fixture.hits.load(Ordering::SeqCst);
    let described = source.inspect(&markdown_reference).await?;
    let description = described
        .description
        .as_deref()
        .ok_or("missing description")?;
    assert!(description.starts_with("Generated instructions\n\nKeep all files, then verify them."));
    assert!(description.contains("1. Run qbutt --check."));
    assert!(description.contains("2. Read guide (https://example.test/guide?edition=1&lang=en)."));
    assert!(description.contains("Do not overwrite unrelated files."));
    assert!(description.contains("Preserve:\n  - video.mkv\n  - subtitles.ass"));
    assert!(description.contains("✓ Download complete"));
    assert!(description.contains("□ Verify files"));
    assert!(description.contains("Track\tLanguage\nVideo\tJapanese"));
    assert!(description.contains("echo \"**literal** <tag>\""));
    assert!(!description.contains("```"));
    assert!(description.contains(&format!(
        "Map ({}) (https://example.test/map)",
        fixture.origin.join("/download/map.png")?
    )));
    assert!(description.contains(&format!(
        "Plain HTML and relative guide ({}).",
        fixture.origin.join("/help")?
    )));
    assert!(description.contains("Fallback instructions remain visible."));
    for unsafe_text in ["SHOULD_NOT_APPEAR", "javascript:", "file:///"] {
        assert!(
            !description.contains(unsafe_text),
            "unsafe description: {unsafe_text}"
        );
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), before_description + 1);
    assert_eq!(fixture.payload.load(Ordering::SeqCst), 0);
    assert!(
        source
            .inspect(&PublicationRef {
                source: Source::Nyaa,
                id: "45".into()
            })
            .await?
            .description
            .is_none()
    );
    for id in ["46", "47", "48", "49", "50"] {
        assert!(
            matches!(
                source
                    .inspect(&PublicationRef {
                        source: Source::Nyaa,
                        id: id.into()
                    })
                    .await,
                Err(Error::ResponseTooLarge)
            ),
            "unbounded description: {id}"
        );
    }
    assert!(matches!(
        source.resolve(&found.variants[0].reference).await?,
        ResolvedContent::Torrent { .. }
    ));
    fixture.changed.store(true, Ordering::SeqCst);
    assert!(matches!(
        source.resolve(&found.variants[0].reference).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.changed.store(false, Ordering::SeqCst);
    for id in ["43", "404", "410"] {
        let unavailable = PublicationRef {
            source: Source::Nyaa,
            id: id.into(),
        };
        assert!(matches!(
            source.inspect(&unavailable).await,
            Err(Error::VariantUnavailable)
        ));
    }
    for category in [Category::Books, Category::Software, Category::Games] {
        let rows = source.search("generated", category).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].category, category);
        if category != Category::Books {
            assert!(rows[0].variants[0].languages.is_empty());
        }
        assert!(rows[0].variants[0].episode.is_none());
    }
    for (query, empty) in [
        ("empty", true),
        ("promotional", true),
        ("raw", false),
        ("non-english", false),
    ] {
        let rows = source.search(query, Category::Anime).await?;
        assert_eq!(rows.is_empty(), empty);
        if !empty {
            assert!(rows[0].variants[0].languages.is_empty());
        }
    }
    let before = fixture.hits.load(Ordering::SeqCst);
    for category in [Category::Movies, Category::Tv, Category::Manga] {
        assert!(source.search("generated", category).await?.is_empty());
    }
    for query in ["", "bad\nvalue", &"x".repeat(501)] {
        assert!(matches!(
            source.search(query, Category::Anime).await,
            Err(Error::InvalidQuery)
        ));
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), before);
    for fault in [
        "challenge",
        "identity",
        "category",
        "duplicate",
        "magnet",
        "size",
        "mime",
        "utf8",
    ] {
        assert!(
            source.search(fault, Category::Anime).await.is_err(),
            "fault accepted: {fault}"
        );
    }
    assert!(matches!(
        source.search("huge", Category::Anime).await,
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        source.search("delay", Category::Anime).await,
        Err(Error::Timeout)
    ));
    let mut request = Box::pin(source.search("cancel", Category::Anime));
    tokio::select! {
        started = tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.cancelled.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        }) => started?,
        _ = &mut request => panic!("cancel fixture completed before receiving the request"),
    }
    drop(request);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert!(matches!(
        http::SourceHttp::new(fixture.origin.clone(), reqwest::header::HeaderMap::new())?
            .html(Url::parse("https://127.0.0.1/catalog")?)
            .await,
        Err(Error::UnsafeAddress)
    ));
    assert!(fixture.payload.load(Ordering::SeqCst) == 0);
    fixture.stop().await;
    println!(
        "PASS nyaa_fixtures generated_https=ok categories=confirmed metadata=reported \
         exact_id_hash=ok changed_missing=rejected empty=ok challenge=rejected \
         bounded_response=ok timeout=ok cancel=connection_closed torrent_payload=none \
         description=paragraphs_lists_tasks_tables_code_links_html_safe \
         no_remote_images=ok description_input_markup_output_depth=bounded"
    );
    Ok(())
}

fn search_page(origin: &Url, query: &str, route: &str) -> String {
    let mut feed = origin.clone();
    feed.query_pairs_mut()
        .extend_pairs([("page", "rss"), ("q", query), ("c", route), ("f", "0")]);
    let feed = feed.as_str().replace('&', "&amp;");
    let code = match (route, query) {
        (_, "promotional") => "1_1",
        (_, "raw") => "1_4",
        (_, "non-english") => "1_3",
        ("3_0", _) => "3_1",
        ("6_1", _) => "6_1",
        ("6_2", _) => "6_2",
        _ => "1_2",
    };
    let row = format!(
        r#"<tr>
            <td><a href="/?c={code}">Category</a></td>
            <td colspan="2">
                <a href="/view/42#comments" class="comments">Comment</a>
                <a href="/view/42">[Generated] Legal &amp; sample - 02v2 [1080p]</a>
            </td>
            <td>
                <a href="/download/42.torrent">Download</a>
                <a href="magnet:?xt=urn:btih:{HASH}">Magnet</a>
            </td>
            <td>700.02 MiB</td><td>2026-10-12</td><td>12</td><td>0</td><td>1</td>
        </tr>"#
    );
    let content = if query == "empty" {
        "<h3>No results found</h3>".into()
    } else {
        format!(
            r#"<table class="torrent-list">
                <thead><tr>
                    <th class="hdr-name">Name</th>
                    <th class="hdr-size">Size</th>
                    <th class="hdr-seeders" title="Seeders"></th>
                </tr></thead>
                <tbody>{row}</tbody>
            </table>"#
        )
    };
    let body = format!(
        r#"<!doctype html><head><link rel="alternate" type="application/rss+xml" href="{feed}"></head><div class="container">{content}</div>"#
    );
    match query {
        "challenge" => "<html><h3>Checking that you are not a bot</h3></html>".into(),
        "identity" => body.replace("q=identity", "q=another"),
        "category" => body.replace("/?c=1_2", "/?c=6_2"),
        "duplicate" => body.replace(&row, &row.repeat(2)),
        "magnet" => body.replace(HASH, "not-a-hash"),
        "size" => body.replace("700.02 MiB", "-700 MiB"),
        _ => body,
    }
}

fn detail_page(id: &str, hash: &str, description: &str) -> String {
    let description = description
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<!doctype html><div class="container">
            <div class="panel">
                <div class="panel-heading">
                    <h3 class="panel-title">[Generated] Legal &amp; sample - 02v2 [1080p]</h3>
                </div>
                <div class="panel-body">
                    <div class="row">
                        <div>Category:</div>
                        <div><a href="/?c=1_0">Anime</a><a href="/?c=1_2">English-translated</a></div>
                        <div>Seeders:</div><div>12</div>
                    </div>
                    <div class="row"><div>File size:</div><div>700.02 MiB</div></div>
                    <div class="row"><div>Info hash:</div><div><kbd>{hash}</kbd></div></div>
                </div>
                <div class="panel-footer">
                    <a href="/download/{id}.torrent">Download</a>
                    <a href="magnet:?xt=urn:btih:{hash}">Magnet</a>
                </div>
            </div>
            <div id="torrent-description">{description}</div>
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
            .timeout(Duration::from_millis(300))
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
                        let query = url.query_pairs()
                            .find(|(name, _)| name == "q")
                            .map(|(_, value)| value.into_owned())
                            .unwrap_or_default();
                        let route = url.query_pairs()
                            .find(|(name, _)| name == "c")
                            .map(|(_, value)| value.into_owned())
                            .unwrap_or_default();

                        if matches!(query.as_str(), "delay" | "cancel") {
                            if query == "cancel" {
                                cancelled.store(1, Ordering::SeqCst);
                            }
                            let mut byte = [0];
                            let _ = stream.read(&mut byte).await;
                            if query == "cancel" {
                                cancelled.store(2, Ordering::SeqCst);
                            }
                            return;
                        }
                        if url.path().starts_with("/download/") {
                            payload.fetch_add(1, Ordering::SeqCst);
                        }
                        let (status, body) = match url.path() {
                            "/view/404" => ("404 Not Found", String::new()),
                            "/view/410" => ("410 Gone", String::new()),
                            "/view/43" => (
                                "200 OK",
                                detail_page("44", HASH, "Generated public-domain sample"),
                            ),
                            "/view/44" => ("200 OK", detail_page("44", HASH, MARKDOWN)),
                            "/view/45" => (
                                "200 OK",
                                detail_page("45", HASH, "#### No description."),
                            ),
                            "/view/46" => {
                                let markdown = format!("{}Generated", " ".repeat(32 * 1024));
                                ("200 OK", detail_page("46", HASH, &markdown))
                            }
                            "/view/47" => {
                                let markdown = format!(
                                    "{}\n\n[long]: https://example.test/{}",
                                    "[guide][long] ".repeat(64),
                                    "x".repeat(2048),
                                );
                                ("200 OK", detail_page("47", HASH, &markdown))
                            }
                            "/view/48" => {
                                let markdown = format!(
                                    "{}Nested{}",
                                    "<div>".repeat(80),
                                    "</div>".repeat(80),
                                );
                                ("200 OK", detail_page("48", HASH, &markdown))
                            }
                            "/view/49" => {
                                let markdown = format!(
                                    "{}\n\n[long]: https://example.test/{}",
                                    "[guide][long] ".repeat(24),
                                    "x".repeat(1500),
                                );
                                ("200 OK", detail_page("49", HASH, &markdown))
                            }
                            "/view/50" => {
                                let markdown = format!(
                                    "{}<table><tr><td>{}Nested{}</td></tr></table>{}",
                                    "<div>".repeat(60),
                                    "<span>".repeat(80),
                                    "</span>".repeat(80),
                                    "</div>".repeat(60),
                                );
                                ("200 OK", detail_page("50", HASH, &markdown))
                            }
                            "/view/42" => {
                                let hash = if changed.load(Ordering::SeqCst) { OTHER } else { HASH };
                                ("200 OK", detail_page("42", hash, "Generated public-domain sample"))
                            }
                            _ => ("200 OK", search_page(&origin, &query, &route)),
                        };
                        let mime = if query == "mime" {
                            "application/json"
                        } else {
                            "text/html; charset=utf-8"
                        };
                        let bytes = if query == "utf8" {
                            vec![0xff]
                        } else {
                            body.into_bytes()
                        };
                        let length = if query == "huge" {
                            3 * 1024 * 1024
                        } else {
                            bytes.len()
                        };
                        let header = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
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
