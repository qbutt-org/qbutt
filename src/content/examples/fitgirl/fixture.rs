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
const OLD: &str = "abcdef0123456789abcdef0123456789abcdef01";
const CHANGED: &str = "123456789abcdef0123456789abcdef012345678";

fn search_page(origin: &Url, query: &str) -> String {
    if query == "challenge" {
        return "<html><h1>Checking your browser</h1></html>".into();
    }
    let escaped = query.replace('&', "&amp;");
    let article = format!(
        r#"<article class="category-lossless-repack"><h1 class="entry-title"><a rel="bookmark" href="{origin}generated-legal-game/">Generated &amp; Legal: Complete Edition – v2.0</a></h1></article>"#
    );
    let (class, content) = if query == "empty" {
        (
            "search-no-results",
            "<h1 class=\"page-title\">Nothing Found</h1>".into(),
        )
    } else {
        ("search-results", article.clone())
    };
    let page = format!(
        r#"<!doctype html><body class="search {class}"><form class="search-form" method="get" action="{origin}"><input name="s" value="{escaped}"></form><div id="content">{content}</div></body>"#
    );
    match query {
        "identity" => page.replace("value=\"identity\"", "value=\"another\""),
        "missing-action" => page.replace(&format!(" action=\"{origin}\""), ""),
        "duplicate" => page.replace(&article, &article.repeat(2)),
        "announcements" => page.replace("category-lossless-repack", "category-uncategorized"),
        "foreign" => page.replace(
            &format!("{origin}generated-legal-game/"),
            "https://clone.example/generated-legal-game/",
        ),
        _ => page,
    }
}

fn detail_page(origin: &Url, slug: &str, changed: bool) -> String {
    let canonical = if slug == "identity" {
        "another-game"
    } else {
        slug
    };
    let hash = if changed { CHANGED } else { HASH };
    let current = format!(
        r#"<a href="magnet:?xt=urn:btih:{hash}&amp;dn=Generated+Legal%3A+Complete+Edition+%28v2.0%29">magnet</a>"#
    );
    let old = format!(
        r#"<div class="su-spoiler"><div class="su-spoiler-title">Old v1.0 repack</div><div class="su-spoiler-content"><a href="magnet:?xt=urn:btih:{OLD}&amp;dn=Generated+Legal+%28v1.0%29">magnet</a></div></div>"#
    );
    let page = format!(
        r#"<!doctype html><head><link rel="canonical" href="{origin}{canonical}/"></head><body><div id="content"><article class="category-lossless-repack"><h1 class="entry-title">Generated &amp; Legal: Complete Edition – v2.0</h1><div class="entry-content"><h3>Download Mirrors (Direct Links)</h3><a href="{origin}payload.zip">Direct file</a><h3>Download Mirrors (Torrent)</h3>{current}{current}{old}</div></article></div></body>"#
    );
    if let Some(case) = slug.strip_prefix("attachment-") {
        let id = match case {
            "normal" | "mirror" | "foreign" | "key" | "ambiguous" => "0000000000000001",
            "hash" => "0000000000000002",
            "auth" => "0000000000000003",
            "work" => "0000000000000004",
            "huge" => "0000000000000005",
            "expanded" => "0000000000000006",
            "type" => "0000000000000007",
            "expired" => "0000000000000008",
            "unavailable" => "0000000000000009",
            "identity" => "000000000000000a",
            "delay" => "000000000000000b",
            "challenge" => "000000000000000c",
            _ => panic!("unknown torrent attachment fixture"),
        };
        let host = if case == "mirror" {
            "pastefg.hermietkreeft.site"
        } else {
            "paste.fitgirl-repacks.site"
        };
        let host = if case == "foreign" {
            "clone.example"
        } else {
            host
        };
        let key = if case == "key" {
            "bad-key"
        } else {
            super::paste_fixture::KEY
        };
        let link = format!(r#"<a href="https://{host}/?{id}#{key}">.torrent file only</a>"#);
        let links = if case == "ambiguous" {
            link.repeat(2)
        } else {
            link
        };
        let attachment = format!("<li>{current}{links}</li>");
        return page
            .replace(&format!("{current}{current}{old}"), &attachment)
            .replace(hash, &super::paste_fixture::hash());
    }
    match slug {
        "direct-only" => page.replace(&current, "").replace(&old, ""),
        "announcement" => page.replace("category-lossless-repack", "category-uncategorized"),
        "bad-magnet" => page.replace(hash, "not-a-hash"),
        "duplicate-canonical" => page.replace(
            "</head>",
            &format!(r#"<link rel="canonical" href="{origin}{canonical}/"></head>"#),
        ),
        "challenge" => "<html><h1>Checking your browser</h1></html>".into(),
        _ => page,
    }
}

pub(super) struct Fixture {
    pub(super) origin: Url,
    pub(super) client: reqwest::Client,
    pub(super) changed: Arc<AtomicBool>,
    pub(super) hits: Arc<AtomicUsize>,
    pub(super) cancelled: Arc<AtomicUsize>,
    pub(super) payload: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub(super) async fn start() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let certificate = rcgen::generate_simple_self_signed(vec![
            "fixture.example".into(),
            "paste.fitgirl-repacks.site".into(),
            "pastefg.hermietkreeft.site".into(),
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
        let origin = Url::parse(&format!("https://fixture.example:{}/", address.port()))?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(reqwest::Certificate::from_der(certificate.cert.der())?)
            .resolve("fixture.example", address)
            .resolve("paste.fitgirl-repacks.site", address)
            .resolve("pastefg.hermietkreeft.site", address)
            .timeout(Duration::from_secs(2))
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
                            .find(|(name, _)| name == "s")
                            .map(|(_, value)| value.into_owned())
                            .unwrap_or_default();
                        let paste_id = url.query().filter(|query| {
                            query.len() == 16
                                && query.bytes().all(|byte| byte.is_ascii_hexdigit())
                        });

                        if matches!(query.as_str(), "delay" | "cancel")
                            || paste_id == Some("000000000000000b")
                        {
                            if query == "cancel" || paste_id.is_some() {
                                cancelled.store(1, Ordering::SeqCst);
                            }
                            let mut byte = [0];
                            let _ = stream.read(&mut byte).await;
                            if query == "cancel" || paste_id.is_some() {
                                cancelled.store(2, Ordering::SeqCst);
                            }
                            return;
                        }
                        if url.path().contains("payload") {
                            payload.fetch_add(1, Ordering::SeqCst);
                        }

                        if let Some(id) = paste_id {
                            let (status, location, body) = super::paste_fixture::response(id);
                            let location = location
                                .map(|value| format!("Location: {value}\r\n"))
                                .unwrap_or_default();
                            let mime = if id.ends_with("0c") { "text/html" } else { "application/json" };
                            let header = format!("HTTP/1.1 {status}\r\n{location}Content-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            let _ = stream.write_all(header.as_bytes()).await;
                            let _ = stream.write_all(&body).await;
                            let _ = stream.shutdown().await;
                            return;
                        }

                        let (status, location, body) = match (url.path(), query.as_str()) {
                            (_, "foreign-redirect") => ("302 Found", Some("https://clone.example/"), String::new()),
                            (_, "redirect-loop") => ("302 Found", Some("/?s=redirect-loop"), String::new()),
                            ("/redirected/", _) => ("302 Found", Some("/different/"), String::new()),
                            ("/missing/", _) => ("404 Not Found", None, String::new()),
                            ("/gone/", _) => ("410 Gone", None, String::new()),
                            ("/", _) => ("200 OK", None, search_page(&origin, &query)),
                            (path, _) => (
                                "200 OK",
                                None,
                                detail_page(&origin, path.trim_matches('/'), changed.load(Ordering::SeqCst)),
                            ),
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
                        let location = location
                            .map(|value| format!("Location: {value}\r\n"))
                            .unwrap_or_default();
                        let header = format!("HTTP/1.1 {status}\r\n{location}Content-Type: {mime}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
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

    pub(super) async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.expect("TLS fixture stopped");
    }
}
