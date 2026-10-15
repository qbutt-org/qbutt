//! Generated legal responses and the owned loopback HTTPS listener.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, oneshot};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

pub(super) fn legal_torrent() -> &'static [u8] {
    b"d8:announce32:https://fixture.example/announce4:infod6:lengthi4e4:name9:legal.txt12:piece lengthi4e6:pieces20:\xa9J\x8f\xe5\xcc\xb1\x9b\xa6\x1cL\x08s\xd3\x91\xe9\x87\x98/\xbb\xd3ee"
}

pub(super) fn changed_torrent() -> &'static [u8] {
    b"d8:announce32:https://fixture.example/announce4:infod6:lengthi4e4:name9:other.txt12:piece lengthi4e6:pieces20:\xa9J\x8f\xe5\xcc\xb1\x9b\xa6\x1cL\x08s\xd3\x91\xe9\x87\x98/\xbb\xd3ee"
}

pub(super) fn alternate_torrent() -> &'static [u8] {
    b"d8:announce32:https://fixture.example/announce4:infod6:lengthi4e4:name9:third.txt12:piece lengthi4e6:pieces20:\xa9J\x8f\xe5\xcc\xb1\x9b\xa6\x1cL\x08s\xd3\x91\xe9\x87\x98/\xbb\xd3ee"
}

pub(super) struct Fixture {
    pub(super) origin: Url,
    pub(super) client: reqwest::Client,
    pub(super) changed: Arc<AtomicBool>,
    pub(super) alternate_mode: Arc<AtomicUsize>,
    pub(super) downloads: Arc<AtomicUsize>,
    pub(super) ajax_started: Arc<Notify>,
    pub(super) ajax_closed: Arc<Notify>,
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
        let alternate_mode = Arc::new(AtomicUsize::new(0));
        let downloads = Arc::new(AtomicUsize::new(0));
        let ajax_started = Arc::new(Notify::new());
        let ajax_closed = Arc::new(Notify::new());
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let changed = Arc::clone(&changed);
            let alternate_mode = Arc::clone(&alternate_mode);
            let downloads = Arc::clone(&downloads);
            let ajax_started = Arc::clone(&ajax_started);
            let ajax_closed = Arc::clone(&ajax_closed);
            async move {
                loop {
                    let socket = tokio::select! {
                        _ = &mut stopped => break,
                        result = listener.accept() => match result {
                            Ok((socket, _)) => socket,
                            Err(_) => break,
                        },
                    };
                    let acceptor = acceptor.clone();
                    let origin = origin.clone();
                    let changed = Arc::clone(&changed);
                    let alternate_mode = Arc::clone(&alternate_mode);
                    let downloads = Arc::clone(&downloads);
                    let ajax_started = Arc::clone(&ajax_started);
                    let ajax_closed = Arc::clone(&ajax_closed);
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
                        let headers = String::from_utf8_lossy(&request).into_owned();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if length > 1024 {
                            return;
                        }
                        let mut form = vec![0; length];
                        if stream.read_exact(&mut form).await.is_err() {
                            return;
                        }
                        let target = headers.split_whitespace().nth(1).unwrap_or("").to_owned();
                        let Ok(url) = origin.join(&target) else {
                            return;
                        };
                        if url.path() == "/cancel-ajax" {
                            ajax_started.notify_one();
                            let mut byte = [0];
                            let _ = stream.read(&mut byte).await;
                            ajax_closed.notify_one();
                            return;
                        }
                        let (status, mime, bytes) = response(
                            &origin,
                            &url,
                            changed.load(Ordering::SeqCst),
                            alternate_mode.load(Ordering::SeqCst),
                            &headers,
                            &form,
                        );
                        if url
                            .query_pairs()
                            .any(|(name, value)| name == "do" && value == "download")
                        {
                            downloads.fetch_add(1, Ordering::SeqCst);
                        }
                        let head = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nSet-Cookie: byrut_fixture=legal; Secure; HttpOnly; Path=/\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        );
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream.write_all(&bytes).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });
        Ok(Self {
            origin,
            client,
            changed,
            alternate_mode,
            downloads,
            ajax_started,
            ajax_closed,
            stop,
            server,
        })
    }

    pub(super) async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.expect("TLS fixture stopped");
    }
}

fn response(
    origin: &Url,
    url: &Url,
    changed: bool,
    alternate_mode: usize,
    headers: &str,
    form: &[u8],
) -> (&'static str, &'static str, Vec<u8>) {
    let query = url
        .query_pairs()
        .find(|(name, _)| name == "story")
        .map(|(_, value)| value.into_owned());
    if url.path() == "/index.php" && query.is_some() {
        let query = query.unwrap();
        if query == "challenge" {
            return ("403 Forbidden", "text/html", b"error".to_vec());
        }
        let shown = if query == "mismatch" { "other" } else { &query };
        let link = if query == "foreign" {
            "https://clone.example/42-generated-legal.html".into()
        } else {
            format!("{origin}42-generated-legal.html")
        };
        let row = format!(
            "<a class=\"search_res\" href=\"{link}\"><span class=\"search_res_title\">Generated Legal</span><div class=\"search_res_sub\"><span class=\"js-ver\">v 1.2</span></div></a>"
        );
        let unreleased = format!(
            r#"<a class="search_res" href="{origin}46-unreleased.html">
                <span class="search_res_title">Unreleased legal fixture</span>
                <span class="status-badge status-badge--ann">Нет раздачи</span>
            </a>"#
        );
        let result = match query.as_str() {
            "empty" | "source-error" | "missing-results" => String::new(),
            "unreleased" => unreleased,
            "mixed" => format!("{row}{unreleased}"),
            "duplicate" => row.repeat(2),
            _ => row,
        };
        let feedback = match query.as_str() {
            "empty" => {
                r#"<div class="search-page__feedback"><div class="berrors_text">По вашему запросу ничего не найдено. Попробуйте изменить или сократить запрос.</div></div>"#
            }
            "source-error" => {
                r#"<div class="search-page__feedback"><div class="berrors_text">Поиск временно недоступен.</div></div>"#
            }
            _ => "",
        };
        let body = format!(
            r#"<html><main>
                <div id="dle-content">
                    <form id="fullsearch"><input name="story" value="{shown}"></form>
                    {result}
                </div>
                {feedback}
            </main></html>"#
        );
        return ("200 OK", "text/html; charset=utf-8", body.into_bytes());
    }
    if url.path().ends_with(".html") {
        if url.path() == "/45-missing.html" {
            return ("404 Not Found", "text/html", Vec::new());
        }
        if url.path() == "/44-challenge.html" {
            return ("403 Forbidden", "text/html", b"error".to_vec());
        }
        let slug = if url.path() == "/43-identity.html" {
            "42-generated-legal"
        } else {
            url.path().trim_start_matches('/').trim_end_matches(".html")
        };
        let (number, _) = slug.split_once('-').expect("fixture publication slug");
        let main = match slug {
            "46-unreleased" | "48-malformed" => String::new(),
            "47-storage-only" | "50-storage-with-alternate" => {
                r#"<div class="info-basictor__item--status info-basictor__item--altnav">
                    <strong class="info-basictor__value">Раздача из файлового хранилища (без торрента).</strong>
                </div>"#.into()
            }
            _ => format!(
                r#"<div class="info-basictor__row">
                    <div class="info-basictor__item">
                        <strong class="info-basictor__value">Open fixture</strong>
                    </div>
                    <div class="info-basictor__item info-basictor__item--version">
                        <strong class="info-basictor__value">v 1.2</strong>
                    </div>
                </div>
                <div class="info-basictor__item--status">
                    <strong class="info-basictor__value">Полная версия</strong>
                </div>
                <a class="itemdown_games" href="{origin}index.php?do=download&amp;id=91">Скачать .torrent</a>"#
            ),
        };
        let notice = if slug == "46-unreleased" {
            r#"<div class="tupd"><span class="tupd-text">Торрента пока нет</span></div>"#
        } else {
            ""
        };
        let title = if slug.contains("-po-seti-") {
            "Generated Legal по сети"
        } else {
            "Generated Legal"
        };
        let alternate = |name: &str, title: &str, version: &str, owner: &str| {
            format!(
                r#"<div class="ajax-block" data-ajax="{name}">
                <button class="ajax-trigger"><span class="showtitle">{title}</span>
                    <span class="show_ver torrent">{version}</span></button>
                <div class="ajax-content" id="ajaxsp-{name}-{owner}"></div>
            </div>"#
            )
        };
        let repack = alternate("features2", "Generated RePack", "v 1.1", number);
        let portable = alternate("features3", "Generated Portable", "v 1.0", number);
        let alternates = match slug {
            "42-generated-legal" => {
                if alternate_mode == 2 {
                    portable
                } else {
                    format!("{repack}{portable}")
                }
            }
            "51-invalid-block" => alternate("features2&id=43", "Invalid", "v 1.1", number),
            "52-alternate-identity" => alternate("features2", "Invalid", "v 1.1", "43"),
            "53-duplicate-block" => repack.repeat(2),
            "60-excessive-alternates" => (2..=18)
                .map(|index| alternate(&format!("features{index}"), "Excessive", "v 1.1", number))
                .collect(),
            "50-storage-with-alternate"
            | "54-foreign-alternate"
            | "55-alternate-challenge"
            | "56-alternate-mime"
            | "57-alternate-missing-link"
            | "58-alternate-oversize"
            | "59-alternate-bad-torrent" => repack,
            _ => String::new(),
        };
        let body = format!(
            r#"<html>
                <head><link rel="canonical" href="{origin}{slug}.html"></head>
                <body>
                    <article class="main_content" data-news-id="{number}">
                        <div class="game_details"><h1>{title}</h1></div>
                        <div class="main-release-block">{main}</div>
                        {notice}
                        <div id="other_repacks">{alternates}
                            <div class="ajax-block" data-ajax="features_storage">
                                <span class="showtitle" data-storage>File storage</span>
                                <span class="show_ver" data-storage>v 9.9</span>
                            </div>
                        </div>
                    </article>
                </body>
            </html>"#
        );
        return ("200 OK", "text/html; charset=utf-8", body.into_bytes());
    }
    if url.path() == "/engine/ajax/controller.php" {
        let ajax = headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("X-Requested-With: XMLHttpRequest"));
        let cookie = headers
            .lines()
            .any(|line| line.starts_with("cookie: ") && line.contains("byrut_fixture=legal"));
        let fields: Vec<_> = url::form_urlencoded::parse(form).collect();
        if !headers.starts_with("POST ")
            || !ajax
            || !cookie
            || fields.len() != 2
            || fields[0].0 != "block"
            || fields[1].0 != "id"
            || !["features2", "features3"].contains(&fields[0].1.as_ref())
        {
            return ("403 Forbidden", "text/html", b"error".to_vec());
        }
        let owner = fields[1].1.as_ref();
        if owner == "55" {
            return ("403 Forbidden", "text/html", b"error".to_vec());
        }
        if owner == "56" {
            return ("200 OK", "application/json", b"{}".to_vec());
        }
        if owner == "57" {
            return (
                "200 OK",
                "text/html",
                b"<div>No torrent link</div>".to_vec(),
            );
        }
        if owner == "58" {
            return ("200 OK", "text/html", vec![b'x'; 2 * 1024 * 1024 + 1]);
        }
        let download = if owner == "54" {
            "https://clone.example/index.php?do=download&id=92".into()
        } else {
            let id = if owner == "59" {
                94
            } else if fields[0].1 == "features2" {
                92
            } else {
                93
            };
            format!("{origin}index.php?do=download&amp;id={id}")
        };
        return (
            "200 OK",
            "text/html; charset=utf-8",
            format!(
                r#"<div class="opencast"><div class="packagedownld-spoiler">
                <a class="packagedownld-spoiler__action" href="{download}">Download .torrent</a>
            </div></div>"#
            )
            .into_bytes(),
        );
    }
    if url
        .query_pairs()
        .any(|(name, value)| name == "do" && value == "download")
    {
        let id = url
            .query_pairs()
            .find(|(name, _)| name == "id")
            .map(|(_, id)| id.into_owned());
        let body = match id.as_deref() {
            Some("91") if changed => changed_torrent(),
            Some("91") => legal_torrent(),
            Some("92") if alternate_mode == 1 => alternate_torrent(),
            Some("92") if alternate_mode == 3 => legal_torrent(),
            Some("92") => changed_torrent(),
            Some("93") => alternate_torrent(),
            Some("94") => b"invalid torrent metadata",
            _ => return ("404 Not Found", "text/html", Vec::new()),
        };
        return ("200 OK", "application/x-bittorrent", body.to_vec());
    }
    ("404 Not Found", "text/html", Vec::new())
}
