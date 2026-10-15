use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

pub(super) const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
pub(super) const OTHER_HASH: &str = "abcdef0123456789abcdef0123456789abcdef01";
pub(super) const PUBLICATION: &str = "42/Generated-Legal";

fn empty_page(category: &str) -> String {
    let message = if category.is_empty() {
        "No results were returned. Please refine your search."
    } else {
        "No results were returned."
    };
    format!(
        "<div class=box-info-heading><h1>Message:</h1></div><div class=box-info-detail><p>{message}</p></div>"
    )
}

pub(super) fn search_page(query: &str, category: &str) -> String {
    if query == "empty" || (query == "category empty" && !category.is_empty()) {
        return empty_page(category);
    }
    if !category.is_empty() {
        match query {
            "missing heading" => return "<div class=box-info-detail></div>".into(),
            "missing message" => {
                return "<div class=box-info-heading><h1>Message:</h1></div>".into();
            }
            "unknown message" => {
                return empty_page(category).replace(
                    "No results were returned.",
                    "The search is temporarily unavailable.",
                );
            }
            _ => {}
        }
    }
    let (subcategory, id) = match category {
        "Games" => (10, 52),
        "Apps" => (18, 53),
        "Other" => (36, 54),
        "Anime" => (28, 55),
        "TV" => (41, 56),
        _ => (54, 42),
    };
    let row = |id: u32, uploader: &str| {
        format!(
            r#"<tr>
        <td class=name><a class=icon href="/sub/{subcategory}/0/"></a><a href="/torrent/{id}/Generated-Legal/">Generated Legal</a></td>
        <td>4</td><td>0</td><td>Today</td><td>1.25 MB<span>4</span></td><td><a href="/user/{uploader}/">{uploader}</a></td>
    </tr>"#
        )
    };
    let normal = row(id, "LegalFixture");
    let rows = match query {
        "duplicate" => format!("{normal}{normal}"),
        "foreign" => normal.replace("href=\"/torrent/", "href=\"https://ads.example/torrent/"),
        _ => format!(
            "{normal}{}{}",
            row(66, "IGGGAMESCOM"),
            row(id + 100, "LegalNeighbor")
        ),
    };
    let table = if query == "missing table" {
        String::new()
    } else {
        format!("<table class=table-list><tbody>{rows}</tbody></table>")
    };
    let label = if query == "wrong query" {
        "another"
    } else {
        query
    };
    format!(
        "<script src='/cdn-cgi/challenge-platform/scripts/jsd/main.js'></script><div class=box-info-heading><h1>Searching for: <span>{label}</span> in:</h1></div>{table}"
    )
}

fn detail_page(hash: &str, magnet_hash: Option<&str>, uploader: &str) -> String {
    let magnet = magnet_hash.map_or(String::new(), |value| {
        format!(r#"<a href="magnet:?xt=urn:btih:{value}&amp;dn=Generated+Legal">Magnet</a>"#)
    });
    format!(
        r#"<script src='/cdn-cgi/challenge-platform/scripts/jsd/main.js'></script><div class=torrent-detail-page>
        <div class=box-info-heading><h1>Generated Legal</h1></div>
        <ul class=list>
            <li><strong>Category</strong><span>Movies</span></li>
            <li><strong>Type</strong><span>HD</span></li>
            <li><strong>Language</strong><span>English</span></li>
            <li><strong>Total size</strong><span>1.25 MB</span></li>
            <li><strong>Uploaded By</strong><span><small></small><a href="/user/{uploader}/">{uploader}</a></span></li>
            <li><strong>Seeders</strong><span>4</span></li>
        </ul>
        <div class=infohash-box><p><strong>Infohash :</strong><span>{hash}</span></p></div>
        {magnet}{magnet}<a href="https://ads.example/payload.exe">Sponsored player</a>
        <a href="https://ads.example/payload.torrent">External torrent</a>
    </div>"#
    )
}

pub(super) struct Fixture {
    pub(super) origin: Url,
    pub(super) client: reqwest::Client,
    pub(super) changed: Arc<AtomicBool>,
    pub(super) hits: Arc<AtomicUsize>,
    pub(super) cancelled: Arc<AtomicUsize>,
    pub(super) payload: Arc<AtomicUsize>,
    pub(super) cookie_faults: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub(super) async fn start() -> std::result::Result<Self, Box<dyn std::error::Error>> {
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
        let cookie_faults = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let changed = Arc::clone(&changed);
            let hits = Arc::clone(&hits);
            let cancelled = Arc::clone(&cancelled);
            let payload = Arc::clone(&payload);
            let cookie_faults = Arc::clone(&cookie_faults);
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
                    let cookie_faults = Arc::clone(&cookie_faults);
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
                        let request = String::from_utf8_lossy(&request).into_owned();
                        let target = request.split_whitespace().nth(1).unwrap_or("");
                        let Ok(url) = origin.join(target) else {
                            return;
                        };
                        let segments = url.path_segments().unwrap().collect::<Vec<_>>();
                        let query = if url.path() == "/srch" {
                            url.query_pairs()
                                .find(|(name, _)| name == "search")
                                .map(|(_, value)| value.into_owned())
                                .unwrap_or_default()
                        } else {
                            let query = segments.get(1).unwrap_or(&"").replace('+', " ");
                            percent_encoding::percent_decode_str(&query)
                                .decode_utf8_lossy()
                                .into_owned()
                        };
                        if query == "cancel" {
                            cancelled.store(1, Ordering::SeqCst);
                            let mut byte = [0];
                            let _ = stream.read(&mut byte).await;
                            cancelled.store(2, Ordering::SeqCst);
                            return;
                        }
                        if target.ends_with(".torrent") || target.ends_with(".exe") {
                            payload.fetch_add(1, Ordering::SeqCst);
                        }
                        let mut headers = String::new();
                        let mut mime = "text/html; charset=utf-8";
                        let lower = request.to_ascii_lowercase();
                        let cleared = lower.contains("user-agent: fixture-webview\r\n")
                            && lower.lines().any(|line| {
                                line.starts_with("cookie:") && line.contains("clearance=fixture-pass")
                            });
                        let needs_clearance = (query.starts_with("webview")
                            && (query != "webview category"
                                || url.path().starts_with("/category-search/")))
                            || url.path().starts_with("/torrent/57/");
                        let (status, body) = if needs_clearance
                            && (!cleared || query == "webview persistent")
                        {
                            ("403 Forbidden", "<div id='cf-challenge'>Complete check</div>".into())
                        } else if url.path() == "/srch" {
                            let redirected = if query == "wrong redirect" {
                                "another"
                            } else {
                                &query
                            };
                            headers = format!("Set-Cookie: search_ready=1; Path=/; Secure; HttpOnly\r\nLocation: /search/{}/1/\r\n", redirected.replace(' ', "+"));
                            ("302 Found", String::new())
                        } else if url.path().starts_with("/torrent/") {
                            match segments.get(1).copied().unwrap_or("") {
                                "404" => ("404 Not Found", String::new()),
                                "410" => ("410 Gone", String::new()),
                                "44" => ("200 OK", detail_page(HASH, Some(OTHER_HASH), "LegalFixture")),
                                "45" => ("200 OK", detail_page(HASH, None, "LegalFixture")),
                                "46" => {
                                    mime = "application/json";
                                    ("200 OK", detail_page(HASH, Some(HASH), "LegalFixture"))
                                },
                                "47" => ("200 OK", detail_page(HASH, Some(HASH), "LegalFixture").replace("</ul>", "<li><strong>Seeders</strong><span>8</span></li></ul>")),
                                "66" => ("200 OK", detail_page(HASH, Some(HASH), "IGGGAMESCOM")),
                                id => {
                                    let hash = if changed.load(Ordering::SeqCst) {
                                        OTHER_HASH
                                    } else {
                                        HASH
                                    };
                                    let (category, kind) = match id {
                                        "52" => ("Games", "PC Game"),
                                        "53" => ("Apps", "PC Software"),
                                        "54" => ("Other", "E-Books"),
                                        "55" => ("Anime", "Anime"),
                                        "56" => ("TV", "HD"),
                                        _ => ("Movies", "HD"),
                                    };
                                    ("200 OK", detail_page(hash, Some(hash), "LegalFixture")
                                        .replace("<span>Movies</span>", &format!("<span>{category}</span>"))
                                        .replace("<span>HD</span>", &format!("<span>{kind}</span>")))
                                },
                            }
                        } else if !lower.lines().any(|line| {
                            line.starts_with("cookie:") && line.contains("search_ready=1")
                        }) {
                            cookie_faults.fetch_add(1, Ordering::SeqCst);
                            ("200 OK", "Bad search request".into())
                        } else if query == "challenge" || query == "browser" {
                            ("403 Forbidden", "<script src='/cdn-cgi/challenge-platform/fixture'></script>".into())
                        } else if query == "bad session" {
                            ("200 OK", "Bad search request".into())
                        } else {
                            let category = if segments[0] == "category-search" {
                                segments.get(2).copied().unwrap_or("")
                            } else {
                                ""
                            };
                            ("200 OK", search_page(&query, category))
                        };
                        let header = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(body.as_bytes()).await;
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
            cookie_faults,
            stop,
            server,
        })
    }

    pub(super) async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.expect("TLS fixture stopped");
    }
}
