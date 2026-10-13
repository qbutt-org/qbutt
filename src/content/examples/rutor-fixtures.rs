//! Generated, legal source and HTTP fault acceptance. The production files are
//! compiled unchanged; only this example supplies a private TLS fixture client.

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
mod rutor {
    include!("../src/rutor.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> RuTor {
        RuTor { http, origin }
    }
}

use rutor::response;

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
const CHANGED_HASH: &str = "abcdef0123456789abcdef0123456789abcdef01";

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let origin = Url::parse("https://rutor.info")?;
    let publication = PublicationRef {
        source: Source::RuTor,
        id: "42".into(),
    };
    let selected = VariantRef {
        publication: publication.clone(),
        selector: VariantSelector::TorrentHash {
            info_hash: HASH.into(),
        },
    };
    selected.validate()?;
    let parsed = response::search(
        &search_page(9, "700.02", "MB", true),
        &origin,
        9,
        Category::Software,
    )?;
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].title, "Generated & legal software v1.2.3");
    assert_eq!(parsed[0].variants[0].size, Some(734024171));
    assert_eq!(parsed[0].variants[0].seeders, Some(1234));
    assert!(parsed[0].variants[0].languages.is_empty());
    assert!(parsed[0].needs_inspection);
    let uppercase_topic = search_page(9, "1", "B", true).replace("urn:btih:", "URN:BTIH:");
    let uppercase_rows = response::search(&uppercase_topic, &origin, 9, Category::Software)?;
    assert_eq!(
        uppercase_rows[0].variants[0].info_hash.as_deref(),
        Some(HASH)
    );
    assert!(
        response::search(
            &search_page(9, "1", "B", false),
            &origin,
            9,
            Category::Software
        )?
        .is_empty()
    );
    assert!(
        response::search(
            &search_page(8, "1", "B", true),
            &origin,
            9,
            Category::Software
        )
        .is_err()
    );
    for body in [
        "<html><h1>Checking that you are not a bot</h1></html>".to_string(),
        search_page(9, "1", "B", true).replace("var search_category = 9;", ""),
        search_page(9, "1", "B", true).replace(
            "var search_category = 9;",
            "var search_category = 9;\nvar search_category = 8;",
        ),
        search_page(9, "1", "B", true).replace("Название", "Changed header"),
        search_page(9, "1", "B", true).replace("Результатов поиска 1", "Результатов поиска 0"),
        search_page(9, "1", "B", false).replace("Результатов поиска 0", "Результатов поиска 1"),
        search_page(9, "1", "B", true).replace("Результатов поиска 1", "No source count"),
        search_page(9, "1", "B", true).replace("/torrent/42/release", "/torrent/042/release"),
        search_page(9, "1", "B", true)
            .replace("/torrent/42/release", "/torrent/42/release?token=private"),
        search_page(9, "1", "B", true)
            .replace("/torrent/42/release", "/torrent/42/release#fragment"),
        search_page(9, "1", "B", true).replace(HASH, "not-a-hash"),
        search_page(9, "1", "B", true).replace(
            "&amp;dn=Generated",
            "&amp;xt=urn:btih:abcdef&amp;dn=Generated",
        ),
        search_page(9, "1", "B", true).replace("1&nbsp;234", "-1"),
        search_page(9, "1", "B", true).replace("1&nbsp;234", "18446744073709551616"),
        search_page(9, "1", "B", true).replace(
            "</span><span class=\"red\">",
            "</span><span class=\"green\">3</span><span class=\"red\">",
        ),
        search_page(9, "1", "B", true).replace(
            "Generated &amp; legal <b>software</b> v1.2.3",
            &"x".repeat(2049),
        ),
        search_page(9, "18446744073709551615.123456789", "TB", true),
        search_page(9, "1.2.3", "GB", true),
    ]
    .into_iter()
    .enumerate()
    {
        let (case, body) = body;
        assert!(
            response::search(&body, &origin, 9, Category::Software).is_err(),
            "parser fault {case}"
        );
    }
    let title_ending_unit = search_page(9, "1", "B", true).replace(
        "Generated &amp; legal <b>software</b> v1.2.3",
        "Generated title B",
    );
    assert_eq!(
        response::search(&title_ending_unit, &origin, 9, Category::Software)?[0].title,
        "Generated title B"
    );
    assert!(parsed[0].variants[0].quality.is_none());
    for (amount, unit, bytes) in [
        ("1", "B", 1),
        ("1,5", "kB", 1536),
        ("2.25", "GB", 2415919104),
        ("1", "TB", 1099511627776),
    ] {
        let rows = response::search(
            &search_page(9, amount, unit, true),
            &origin,
            9,
            Category::Software,
        )?;
        assert_eq!(rows[0].variants[0].size, Some(bytes));
    }
    for id in ["0", "042", "42/extra", "-42", "18446744073709551616"] {
        assert!(
            PublicationRef {
                source: Source::RuTor,
                id: id.into()
            }
            .validate()
            .is_err()
        );
    }
    let mut invalid = selected.clone();
    invalid.selector = VariantSelector::TorrentHash {
        info_hash: HASH.to_uppercase(),
    };
    assert!(invalid.validate().is_err());
    invalid.selector = VariantSelector::Torrent { id: 42 };
    assert!(invalid.validate().is_err());

    let fresh = response::release(&detail_page(42, HASH, "Софт"), &origin, &publication)?;
    assert_eq!(fresh.work.category, Category::Software);
    assert_eq!(fresh.work.variants[0].size, Some(734020818));
    assert_eq!(fresh.work.variants[0].seeders, Some(15));
    assert!(!fresh.work.needs_inspection);
    match fresh.resolve(&selected)? {
        ResolvedContent::Torrent {
            reference, magnet, ..
        } => {
            assert_eq!(reference, selected);
            assert_eq!(magnet.scheme(), "magnet");
        }
        _ => panic!("wrong transport"),
    }
    assert!(matches!(
        response::release(&detail_page(43, HASH, "Софт"), &origin, &publication),
        Err(Error::VariantUnavailable)
    ));
    assert!(matches!(
        response::release(
            &detail_page(42, CHANGED_HASH, "Софт"),
            &origin,
            &publication
        )?
        .resolve(&selected),
        Err(Error::VariantUnavailable)
    ));
    assert!(
        response::release(&detail_page(42, HASH, "Неизвестно"), &origin, &publication).is_err()
    );
    for body in [
        detail_page(42, HASH, "Софт").replace(
            "//d.rutor.info/download/42",
            "https://127.0.0.1/download/42",
        ),
        detail_page(42, HASH, "Софт").replace(
            "//d.rutor.info/download/42",
            "https://evil.example/download/42",
        ),
        detail_page(42, HASH, "Софт").replace(
            "//d.rutor.info/download/42",
            "https://user:password@d.rutor.info/download/42",
        ),
        detail_page(42, HASH, "Софт").replace("734020818 Bytes", "-1 Bytes"),
        detail_page(42, HASH, "Софт").replace("<h1>", "<h1>".repeat(2).as_str()),
    ] {
        assert!(response::release(&body, &origin, &publication).is_err());
    }
    for (reported, expected) in [
        ("Зарубежные фильмы", Category::Movies),
        ("Научно-популярные фильмы", Category::Movies),
        ("Наши сериалы", Category::Tv),
        ("Аниме", Category::Anime),
        ("Игры", Category::Games),
        ("Книги", Category::Books),
    ] {
        assert_eq!(
            response::release(&detail_page(42, HASH, reported), &origin, &publication)?
                .work
                .category,
            expected
        );
    }

    let fixture = Fixture::start().await?;
    let http = http::fixture(fixture.client.clone(), fixture.origin.clone());
    let provider = rutor::fixture(http, fixture.origin.clone());
    let works = provider.search("legal", Category::Software).await?;
    assert_eq!(works.len(), 1);
    assert_eq!(works[0].variants[0].reference, selected);
    assert_eq!(provider.inspect(&publication).await?.reference, publication);
    assert!(matches!(
        provider.resolve(&selected).await?,
        ResolvedContent::Torrent { .. }
    ));
    fixture.changed.store(true, Ordering::SeqCst);
    assert!(matches!(
        provider.resolve(&selected).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.changed.store(false, Ordering::SeqCst);
    for id in ["404", "410", "43"] {
        let absent = PublicationRef {
            source: Source::RuTor,
            id: id.into(),
        };
        assert!(matches!(
            provider.inspect(&absent).await,
            Err(Error::VariantUnavailable)
        ));
    }
    for category in [
        Category::Movies,
        Category::Tv,
        Category::Anime,
        Category::Games,
        Category::Books,
    ] {
        let rows = provider.search("legal", category).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].category, category);
    }
    let before_unsupported = fixture.hits.load(Ordering::SeqCst);
    assert!(provider.search("legal", Category::Manga).await?.is_empty());
    assert_eq!(fixture.hits.load(Ordering::SeqCst), before_unsupported);
    let before_invalid = fixture.hits.load(Ordering::SeqCst);
    for query in [
        "",
        ".",
        "..",
        "a",
        "ab",
        "я",
        "\n",
        "bad\u{0}",
        &"x".repeat(501),
    ] {
        assert!(
            matches!(
                provider.search(query, Category::Software).await,
                Err(Error::InvalidQuery)
            ),
            "invalid query was accepted: {query}"
        );
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), before_invalid);
    for query in ["abc", "яя", "яяя"] {
        assert_eq!(provider.search(query, Category::Software).await?.len(), 1);
    }
    for fault in [
        "json",
        "missing-mime",
        "utf8",
        "wrong-category",
        "challenge",
    ] {
        assert!(matches!(
            provider.search(fault, Category::Software).await,
            Err(Error::InvalidResponse(_))
        ));
    }
    for fault in ["huge-length", "huge-stream"] {
        assert!(matches!(
            provider.search(fault, Category::Software).await,
            Err(Error::ResponseTooLarge)
        ));
    }
    for fault in ["foreign", "local", "downgrade"] {
        assert!(matches!(
            provider.search(fault, Category::Software).await,
            Err(Error::UnsafeAddress)
        ));
    }
    assert!(matches!(
        provider.search("loop", Category::Software).await,
        Err(Error::InvalidRedirect)
    ));
    assert_eq!(
        provider.search("redirect", Category::Software).await?.len(),
        1
    );
    assert_eq!(
        provider
            .search("case-mime", Category::Software)
            .await?
            .len(),
        1
    );
    assert!(matches!(
        provider.search("delay", Category::Software).await,
        Err(Error::Timeout)
    ));
    let mut operation = Box::pin(provider.search("cancel", Category::Software));
    tokio::select! {
        biased;
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        _ = &mut operation => panic!("fixture completed before cancellation"),
    }
    drop(operation);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(provider.search("legal", Category::Software).await?.len(), 1);

    // The unmodified production constructor must never connect to local DNS/IP.
    let guarded = http::SourceHttp::new(fixture.origin.clone(), reqwest::header::HeaderMap::new())?;
    assert!(matches!(
        guarded.html(Url::parse("https://127.0.0.1/catalog")?).await,
        Err(Error::UnsafeAddress)
    ));
    assert!(matches!(
        guarded.html(Url::parse("https://localhost/catalog")?).await,
        Err(Error::UnsafeAddress)
    ));
    let before_guard = fixture.hits.load(Ordering::SeqCst);
    let local_url = Url::parse(&format!(
        "https://localhost:{}/catalog",
        fixture.origin.port().unwrap()
    ))?;
    // Even a rejected TLS handshake must count as reaching the local socket.
    let untrusted = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .timeout(Duration::from_millis(500))
        .resolve(
            "localhost",
            std::net::SocketAddr::from(([127, 0, 0, 1], fixture.origin.port().unwrap())),
        )
        .build()?;
    let controlled = http::fixture(untrusted, local_url.clone());
    assert!(matches!(
        controlled.html(local_url.clone()).await,
        Err(Error::Network(_))
    ));
    assert_eq!(
        fixture.hits.load(Ordering::SeqCst),
        before_guard + 1,
        "TLS rejection hid a TCP connection"
    );
    let before_guard = fixture.hits.load(Ordering::SeqCst);
    let same_local_origin =
        http::SourceHttp::new(local_url.clone(), reqwest::header::HeaderMap::new())?;
    match same_local_origin.html(local_url).await {
        Err(Error::Network(error)) => assert!(error.url().is_none()),
        _ => panic!("guarded resolver accepted local DNS"),
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), before_guard);
    fixture.stop().await;
    println!(
        "PASS rutor-fixtures entities=ok categories=confirmed valid_empty=ok exact_id_hash=ok changed_vanished=rejected metadata=bounded html_mime_utf8=checked redirects=guarded response_size=bounded cancellation=connection_closed production_local_dns=rejected torrent_payload=not_tested"
    );
    Ok(())
}

fn search_page(category: u8, amount: &str, unit: &str, row: bool) -> String {
    let row = if row {
        format!(
            r#"<tr><td>12 Oct 26</td><td><a href="magnet:?xt=urn:btih:{HASH}&amp;dn=Generated">M</a><a href="/torrent/42/release">Generated &amp; legal <b>software</b> v1.2.3</a></td><td>5 comments</td><td>{amount}&nbsp;{unit}</td><td><span class="green">1&nbsp;234</span><span class="red">0</span></td></tr>"#
        )
    } else {
        String::new()
    };
    let total = usize::from(!row.is_empty());
    format!(
        "<!doctype html><script>var search_category = {category};</script><div id='index'><b>Страницы: 1</b> Результатов поиска {total} (max. 2000)<table><tr class='backgr'><td>Название</td></tr>{row}</table></div>"
    )
}

fn detail_page(id: u64, hash: &str, category: &str) -> String {
    format!(
        r#"<!doctype html><h1>Generated &amp; legal software v1.2.3</h1><div id="download"><a href="magnet:?xt=urn:btih:{hash}&amp;dn=Generated">M</a><a href="//d.rutor.info/download/{id}">Download</a></div><table id="details"><tr><td class="header">Категория</td><td><a href="/soft">{category}</a></td></tr><tr><td class="header">Размер</td><td>700.02&nbsp;MB (734020818 Bytes)</td></tr><tr><td class="header">Раздают</td><td>15</td></tr><tr><td colspan="2"><div id="index"><table><tr><td><a href="magnet:?xt=urn:btih:{CHANGED_HASH}">Related wrong magnet</a></td></tr></table></div></td></tr></table>"#
    )
}

struct Fixture {
    origin: Url,
    client: reqwest::Client,
    changed: Arc<AtomicBool>,
    cancelled: Arc<AtomicUsize>,
    hits: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> std::result::Result<Self, Box<dyn std::error::Error>> {
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
            .timeout(Duration::from_millis(500))
            .resolve("fixture.example", address)
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .build()?;
        let changed = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let hits = Arc::new(AtomicUsize::new(0));
        let (stop, mut shutdown) = oneshot::channel();
        let shared_changed = changed.clone();
        let shared_cancelled = cancelled.clone();
        let shared_hits = hits.clone();
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown => break,
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); }
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        shared_hits.fetch_add(1, Ordering::SeqCst);
                        let acceptor = acceptor.clone();
                        let changed = shared_changed.clone();
                        let cancelled = shared_cancelled.clone();
                        connections.spawn(async move {
                            if let Ok(stream) = acceptor.accept(socket).await {
                                serve(stream, changed.load(Ordering::SeqCst), cancelled).await;
                            }
                        });
                    }
                }
            }
            connections.shutdown().await;
        });
        Ok(Self {
            origin,
            client,
            changed,
            cancelled,
            hits,
            stop,
            server,
        })
    }

    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.server.await.unwrap();
    }
}

async fn serve(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    changed: bool,
    cancelled: Arc<AtomicUsize>,
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
    let path = request.split_whitespace().nth(1).unwrap();
    let query = path.trim_end_matches('/').rsplit('/').next().unwrap();
    if query == "cancel" {
        let mut byte = [0];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            cancelled.fetch_add(1, Ordering::SeqCst);
        }
        return;
    }
    if query == "delay" {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let mut status = "200 OK";
    let mut mime = "Content-Type: text/html; charset=UTF-8\r\n";
    let mut extra = String::new();
    let mut body = if path.starts_with("/torrent/") {
        let id = query.parse::<u64>().unwrap();
        if id == 404 {
            status = "404 Not Found";
        }
        if id == 410 {
            status = "410 Gone";
        }
        detail_page(
            if id == 43 { 44 } else { id },
            if changed { CHANGED_HASH } else { HASH },
            "Софт",
        )
    } else {
        let category = path.split('/').nth(3).unwrap().parse::<u8>().unwrap();
        search_page(
            if query == "wrong-category" {
                0
            } else {
                category
            },
            "700.02",
            "MB",
            true,
        )
    }
    .into_bytes();
    match query {
        "json" => mime = "Content-Type: application/json\r\n",
        "case-mime" => mime = "Content-Type: Text/HTML; charset=UTF-8\r\n",
        "missing-mime" => mime = "",
        "utf8" => body = vec![0xff, 0xfe],
        "challenge" => body = b"<h1>Checking that you are not a bot</h1>".to_vec(),
        "huge-length" => extra = "Content-Length: 2097153\r\n".into(),
        "huge-stream" => {
            body = vec![b'x'; 2097153];
            extra = "Transfer-Encoding: chunked\r\n".into();
            body = [
                format!("{:x}\r\n", body.len()).as_bytes(),
                &body,
                b"\r\n0\r\n\r\n",
            ]
            .concat();
        }
        "foreign" | "local" | "downgrade" | "loop" | "redirect" => {
            status = "302 Found";
            let location = match query {
                "foreign" => "https://evil.example/catalog",
                "local" => "https://127.0.0.1/catalog",
                "downgrade" => "http://fixture.example/catalog",
                "loop" => path,
                _ => "/search/0/9/100/0/legal/",
            };
            extra = format!("Location: {location}\r\n");
        }
        _ => {}
    }
    if !matches!(query, "huge-length" | "huge-stream") {
        extra.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    let header = format!("HTTP/1.1 {status}\r\n{mime}{extra}Connection: close\r\n\r\n");
    if stream.write_all(header.as_bytes()).await.is_ok() {
        let _ = stream.write_all(&body).await;
    }
    let _ = stream.shutdown().await;
}
