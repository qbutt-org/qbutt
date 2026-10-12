//! Generated legal CP1251 traffic through the production provider and guarded
//! HTTP transport. The private TLS client exists only in this executable.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
pub use qbutt_content::webview;
#[path = "../src/browser/mod.rs"]
pub mod browser;
#[path = "browser/fixture.rs"]
#[allow(dead_code)]
mod browser_fixture;
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
mod nnmclub {
    include!("../src/nnmclub.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> NnmClub {
        NnmClub { http, origin }
    }
}

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use browser::{BrowserBroker, BrowserChoice};
use encoding_rs::WINDOWS_1251;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

type CheckResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
const CHANGED: &str = "abcdef0123456789abcdef0123456789abcdef01";

#[tokio::main]
async fn main() -> CheckResult {
    if std::env::args().nth(1).as_deref() == Some("--live") {
        let source = qbutt_content::NnmClub::new()?;
        let result = source
            .search(
                "Sintel",
                qbutt_content::Category::Movies,
                None,
                qbutt_content::browser::BrowserChoice::Auto,
            )
            .await;
        match result {
            Ok(works) => println!("PASS live_nnmclub_search={} results", works.len()),
            Err(qbutt_content::Error::Challenge) => {
                println!("BLOCKED live_nnmclub_search=source_challenge")
            }
            Err(error) => return Err(error.into()),
        }
        return Ok(());
    }
    let fixture = Fixture::start().await?;
    let source = Arc::new(nnmclub::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    ));
    let search = {
        let source = source.clone();
        tokio::spawn(async move {
            source
                .search(
                    "Синтел & legal",
                    Category::Movies,
                    None,
                    BrowserChoice::Auto,
                )
                .await
        })
    };
    let found = search.await??.remove(0);
    assert_eq!(found.title, "Синтел & legal");
    assert_eq!(found.category, Category::Movies);
    assert_eq!(found.reference.id, "42");
    assert!(found.variants.is_empty());
    assert!(found.needs_inspection);
    assert_eq!(fixture.posts.lock().unwrap().len(), 1);
    assert!(fixture.posts.lock().unwrap()[0].contains("nm=%D1%E8%ED%F2%E5%EB+%26+legal"));
    assert!(fixture.posts.lock().unwrap()[0].contains("f%5B%5D=216"));
    assert!(!fixture.posts.lock().unwrap()[0].contains("f%5B%5D=533"));
    assert!(!fixture.posts.lock().unwrap()[0].contains("f%5B%5D=256"));
    assert!(!fixture.posts.lock().unwrap()[0].contains("f%5B%5D=257"));

    let inspect = {
        let source = source.clone();
        let reference = found.reference.clone();
        tokio::spawn(async move { source.inspect(&reference, None, BrowserChoice::Auto).await })
    };
    let work = inspect.await??;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.variants[0].info_hash.as_deref(), Some(HASH));
    assert_eq!(
        work.description.as_deref(),
        Some("Generated legal public-domain video")
    );
    assert!(!work.needs_inspection);
    let selected = work.variants[0].reference.clone();
    let resolve = {
        let source = source.clone();
        let selected = selected.clone();
        tokio::spawn(async move { source.resolve(&selected, None, BrowserChoice::Auto).await })
    };
    match resolve.await?? {
        ResolvedContent::Torrent {
            reference, magnet, ..
        } => {
            assert_eq!(reference, selected);
            assert_eq!(magnet.scheme(), "magnet");
        }
        _ => panic!("expected torrent"),
    }
    library(&work)?;

    let software = source
        .search("GoldWave", Category::Software, None, BrowserChoice::Auto)
        .await?
        .remove(0);
    assert_eq!(software.reference.id, "43");
    assert_eq!(software.category, Category::Software);
    assert!(fixture.posts.lock().unwrap()[1].contains("f%5B%5D=533"));
    assert!(!fixture.posts.lock().unwrap()[1].contains("f%5B%5D=216"));
    let software = source
        .inspect(&software.reference, None, BrowserChoice::Auto)
        .await?;
    assert_eq!(software.variants[0].info_hash.as_deref(), Some(HASH));
    assert!(matches!(
        source
            .resolve(&software.variants[0].reference, None, BrowserChoice::Auto)
            .await?,
        ResolvedContent::Torrent { .. }
    ));

    fixture.case("changed-hash");
    assert!(matches!(
        source.resolve(&selected, None, BrowserChoice::Auto).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.case("changed-topic");
    assert!(matches!(
        source.resolve(&selected, None, BrowserChoice::Auto).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.case("missing-magnet");
    assert!(matches!(
        source.resolve(&selected, None, BrowserChoice::Auto).await,
        Err(Error::UnsupportedAcquisition(_))
    ));
    fixture.case("gone");
    assert!(matches!(
        source
            .inspect(&found.reference, None, BrowserChoice::Auto)
            .await,
        Err(Error::VariantUnavailable)
    ));
    fixture.case("challenge");
    assert!(matches!(
        source
            .search("legal", Category::Movies, None, BrowserChoice::Auto)
            .await,
        Err(Error::Challenge)
    ));
    fixture.case("bad-result");
    assert!(matches!(
        source
            .search("legal", Category::Movies, None, BrowserChoice::Auto)
            .await,
        Err(Error::InvalidResponse(_))
    ));
    fixture.case("bad-taxonomy");
    assert!(matches!(
        source
            .search("legal", Category::Movies, None, BrowserChoice::Auto)
            .await,
        Err(Error::InvalidResponse(_))
    ));
    fixture.case("empty");
    assert!(
        source
            .search("legal", Category::Movies, None, BrowserChoice::Auto)
            .await?
            .is_empty()
    );
    fixture.case("normal");
    let before = fixture.posts.lock().unwrap().len();
    for invalid in ["", "bad\nquery", "😀", &"a".repeat(61)] {
        assert!(matches!(
            source
                .search(invalid, Category::Movies, None, BrowserChoice::Auto)
                .await,
            Err(Error::InvalidQuery)
        ));
    }
    assert_eq!(fixture.posts.lock().unwrap().len(), before);
    assert!(
        source
            .search("legal", Category::Books, None, BrowserChoice::Auto)
            .await?
            .is_empty()
    );

    fixture.case("cancel");
    let pending = {
        let source = source.clone();
        tokio::spawn(async move {
            source
                .search("legal", Category::Movies, None, BrowserChoice::Auto)
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), fixture.started.notified()).await?;
    pending.abort();
    let _ = pending.await;
    tokio::time::timeout(Duration::from_secs(2), fixture.closed.notified()).await?;

    browser_session(&fixture, &source).await?;

    fixture.stop().await?;
    println!(
        "PASS nnmclub_cp1251_post=ok search_inspect_resolve_spawn=Send exact_hash=ok library=roundtrip changed_hash=rejected challenge=typed malformed=rejected cancellation=closed"
    );
    Ok(())
}

fn library(work: &Work) -> CheckResult {
    use qbutt_content::{Library, LibraryView, WorkGroup};

    let directory = std::env::temp_dir().join(format!(
        "qbutt-nnmclub-library-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir(&directory)?;
    let result = (|| -> CheckResult {
        let mut work: qbutt_content::Work = serde_json::from_value(serde_json::to_value(work)?)?;
        work.url = "https://nnmclub.to/forum/viewtopic.php?t=42".parse()?;

        let mut library = Library::open(&directory)?;
        library.set_favorite(&WorkGroup::single(work.clone()), true)?;
        for url in [
            "https://nnmclub.to/forum/viewtopic.php?t=43",
            "https://nnmclub.to/forum/viewtopic.php?t=42&token=private",
            "https://nnmclub.to/forum/download.php?id=42",
            "https://nnmclub.example/forum/viewtopic.php?t=42",
        ] {
            let mut invalid = work.clone();
            invalid.url = url.parse()?;
            assert!(
                library
                    .set_favorite(&WorkGroup::single(invalid), true)
                    .is_err()
            );
        }

        drop(library);
        let library = Library::open(&directory)?;
        assert_eq!(
            library.entries(LibraryView::Favorites)?[0]
                .group
                .anchor_work()
                .unwrap(),
            &work
        );
        Ok(())
    })();
    std::fs::remove_dir_all(directory)?;
    result
}

async fn browser_session(fixture: &Fixture, source: &nnmclub::NnmClub) -> CheckResult {
    let directory = std::env::temp_dir().join(format!(
        "qbutt-nnmclub-browser-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir(&directory)?;
    let discovery = directory.join("browser-bridge.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let handle = broker.handle();
    fixture.case("session");
    let mut edge = browser_fixture::extension(&discovery, "edge", "fixture-browser", false).await?;
    let mut status = handle.subscribe();
    tokio::time::timeout(Duration::from_secs(3), async {
        while status.borrow().connections.len() != 1 {
            status.changed().await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await??;

    let operation = source.search(
        "legal",
        Category::Movies,
        Some(&handle),
        BrowserChoice::Edge,
    );
    let (result, reply) = tokio::join!(operation, answer_session(&mut edge, fixture, false));
    reply?;
    let found = result?.remove(0);
    let operation = source.inspect(&found.reference, Some(&handle), BrowserChoice::Edge);
    let (result, reply) = tokio::join!(operation, answer_session(&mut edge, fixture, false));
    reply?;
    let inspected = result?;
    let operation = source.resolve(
        &inspected.variants[0].reference,
        Some(&handle),
        BrowserChoice::Edge,
    );
    let (result, reply) = tokio::join!(operation, answer_session(&mut edge, fixture, false));
    reply?;
    assert!(matches!(result?, ResolvedContent::Torrent { .. }));

    let operation = source.search(
        "legal",
        Category::Movies,
        Some(&handle),
        BrowserChoice::Edge,
    );
    let (result, reply) = tokio::join!(operation, answer_session(&mut edge, fixture, true));
    reply?;
    assert!(matches!(result, Err(Error::Challenge)));

    drop(edge);
    broker.shutdown().await?;
    assert!(!discovery.exists());
    std::fs::remove_dir(directory)?;
    fixture.case("normal");
    Ok(())
}

async fn answer_session(
    edge: &mut tokio::net::windows::named_pipe::NamedPipeClient,
    fixture: &Fixture,
    expired: bool,
) -> CheckResult {
    let wire = browser_fixture::read(edge).await?;
    assert_eq!(wire["method"], "cookies");
    assert_eq!(
        wire["url"],
        fixture.origin.join("/forum/tracker.php")?.as_str()
    );
    let mut cookie = json!({
        "name":"bbsession", "value":"fixture-session", "domain":"fixture.example",
        "path":"/forum/", "secure":true, "hostOnly":true
    });
    if expired {
        cookie["expirationDate"] = json!(1.0);
    }
    browser_fixture::write(edge, &json!({"id":wire["id"], "result":[cookie]})).await
}

struct Fixture {
    origin: Url,
    client: reqwest::Client,
    posts: Arc<Mutex<Vec<String>>>,
    mode: Arc<Mutex<&'static str>>,
    started: Arc<Notify>,
    closed: Arc<Notify>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> CheckResult<Self> {
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
        let origin = Url::parse(&format!("https://fixture.example:{}/", address.port()))?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .resolve("fixture.example", address)
            .timeout(Duration::from_millis(800))
            .build()?;
        let posts = Arc::new(Mutex::new(Vec::new()));
        let mode = Arc::new(Mutex::new("normal"));
        let started = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let (stop, mut shutdown) = oneshot::channel();
        let server = tokio::spawn({
            let posts = posts.clone();
            let mode = mode.clone();
            let started = started.clone();
            let closed = closed.clone();
            async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = &mut shutdown => break,
                        Some(result) = connections.join_next(), if !connections.is_empty() => result.unwrap(),
                        accepted = listener.accept() => {
                            let (socket, _) = accepted.unwrap();
                            let acceptor = acceptor.clone();
                            let posts = posts.clone();
                            let mode = mode.clone();
                            let started = started.clone();
                            let closed = closed.clone();
                            connections.spawn(async move {
                                let Ok(mut stream) = acceptor.accept(socket).await else { return; };
                                let mut head = Vec::new();
                                while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                                    let mut byte = [0];
                                    if stream.read_exact(&mut byte).await.is_err() { return; }
                                    head.push(byte[0]);
                                }
                                let head = String::from_utf8(head).unwrap();
                                let mut fields = head.split_whitespace();
                                let method = fields.next().unwrap();
                                let path = fields.next().unwrap();
                                let length = head.lines()
                                    .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned))
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                                    .unwrap_or(0);
                                let mut body = vec![0; length];
                                stream.read_exact(&mut body).await.unwrap();
                                let form = String::from_utf8(body).unwrap();
                                if method == "POST" {
                                    assert_eq!(path, "/forum/tracker.php");
                                    posts.lock().unwrap().push(form.clone());
                                }
                                let mode = *mode.lock().unwrap();
                                if mode == "cancel" && method == "POST" {
                                    started.notify_one();
                                    let _ = stream.read(&mut [0]).await;
                                    closed.notify_one();
                                    return;
                                }
                                let authenticated = head.to_ascii_lowercase().contains("user-agent: fixture-browser\r\n")
                                    && head.contains("bbsession=fixture-session");
                                let (status, html) = page(path, method, &form, mode, authenticated);
                                let (bytes, _, invalid) = WINDOWS_1251.encode(&html);
                                assert!(!invalid);
                                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=windows-1251\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                                stream.write_all(reply.as_bytes()).await.unwrap();
                                stream.write_all(&bytes).await.unwrap();
                                let _ = stream.shutdown().await;
                            });
                        }
                    }
                }
                connections.shutdown().await;
            }
        });
        Ok(Self {
            origin,
            client,
            posts,
            mode,
            started,
            closed,
            stop,
            server,
        })
    }

    fn case(&self, mode: &'static str) {
        *self.mode.lock().unwrap() = mode;
    }

    async fn stop(self) -> CheckResult {
        let _ = self.stop.send(());
        self.server.await?;
        Ok(())
    }
}

fn page(
    path: &str,
    method: &str,
    form: &str,
    mode: &str,
    authenticated: bool,
) -> (&'static str, String) {
    if (mode == "challenge" && method == "POST") || (mode == "session" && !authenticated) {
        return (
            "403 Forbidden",
            "<script src='/cdn-cgi/challenge-platform/check.js'></script>".into(),
        );
    }
    if path == "/forum/tracker.php" && method == "GET" {
        let boundary = if mode == "bad-taxonomy" {
            String::new()
        } else {
            "<option value='256'>Театр и Музыкальное видео</option>".into()
        };
        return (
            "200 OK",
            format!(
                "<form id='search_form'><select id='fs' name='f[]'><optgroup label='Трекер: Видео. Кино, Театр, Муз.видео'><option value='216'>Кино</option>{boundary}<option value='257'>Клипы</option></optgroup><optgroup label='Трекер: Программы, Операционные системы'><option value='533'>Программы</option></optgroup></select></form>"
            ),
        );
    }
    if path == "/forum/tracker.php" && method == "POST" {
        if mode == "empty" {
            return (
                "200 OK",
                "<table class='forumline tablesorter'><tbody></tbody></table>".into(),
            );
        }
        let software = form.contains("f%5B%5D=533");
        let (id, forum, title) = if software {
            ("43", 533, "Generated legal software")
        } else {
            ("42", 216, "Синтел &amp; legal")
        };
        let topic = if mode == "bad-result" { "043" } else { id };
        return (
            "200 OK",
            format!(
                "<table class='forumline tablesorter'><tbody><tr class='prow1'><td></td><td><a href='tracker.php?f={forum}'>Forum</a></td><td><a href='viewtopic.php?t={topic}'>{title}</a></td><td></td><td><a href='download.php?id=99'>DL</a></td><td><u>734003200</u></td><td><b>12</b></td><td></td><td></td><td></td></tr></tbody></table>"
            ),
        );
    }
    if matches!(
        path,
        "/forum/viewtopic.php?t=42" | "/forum/viewtopic.php?t=43"
    ) {
        if mode == "gone" {
            return ("404 Not Found", "missing".into());
        }
        let software = path.ends_with("43");
        let (id, forum, title, description) = if software {
            (
                43,
                533,
                "Generated legal software",
                "Generated legal software release",
            )
        } else {
            (
                42,
                216,
                "Синтел &amp; legal",
                "Generated legal public-domain video",
            )
        };
        let hash = if mode == "changed-hash" {
            CHANGED
        } else {
            HASH
        };
        let topic_id = if mode == "changed-topic" { id + 1 } else { id };
        let magnet = if mode == "missing-magnet" {
            String::new()
        } else {
            format!("<a href='magnet:?xt=urn:btih:{hash}'>Magnet</a>")
        };
        return (
            "200 OK",
            format!(
                "<h1><a class='maintitle' href='viewtopic.php?t={topic_id}'>{title}</a></h1><a class='nav' href='viewforum.php?f={forum}'>Forum</a><table id='post_1'><tr><td><div class='postbody'>{description}</div></td></tr></table><table class='btTbl'><tr><td>{magnet}</td></tr></table>"
            ),
        );
    }
    panic!("unexpected fixture request: {method} {path}")
}
