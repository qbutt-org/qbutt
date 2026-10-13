use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use encoding_rs::WINDOWS_1251;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

use super::CheckResult;

pub const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
pub const OTHER: &str = "abcdef0123456789abcdef0123456789abcdef01";

pub struct Fixture {
    pub origin: Url,
    pub client: reqwest::Client,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub started: Arc<Notify>,
    pub closed: Arc<Notify>,
    hits: Arc<AtomicUsize>,
    case: Arc<Mutex<&'static str>>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub async fn start() -> CheckResult<Self> {
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let case = Arc::new(Mutex::new("normal"));
        let started = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let (stop, mut shutdown) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let requests = requests.clone();
            let hits = hits.clone();
            let case = case.clone();
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
                            let origin = origin.clone();
                            let requests = requests.clone();
                            let hits = hits.clone();
                            let case = case.clone();
                            let started = started.clone();
                            let closed = closed.clone();
                            connections.spawn(async move {
                                let Ok(mut stream) = acceptor.accept(socket).await else { return; };
                                let mut bytes = Vec::new();
                                while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 8192 {
                                    let mut byte = [0];
                                    if stream.read_exact(&mut byte).await.is_err() { return; }
                                    bytes.push(byte[0]);
                                }
                                let head = String::from_utf8(bytes).unwrap();
                                assert!(head.starts_with("GET "));
                                let target = head.split_whitespace().nth(1).unwrap();
                                requests.lock().unwrap().push(target.to_owned());
                                hits.fetch_add(1, Ordering::SeqCst);
                                let url = origin.join(target).unwrap();
                                let case = *case.lock().unwrap();
                                if url.path() == "/forum/tracker.php" && matches!(case, "delay" | "cancel") {
                                    if case == "cancel" {
                                        started.notify_one();
                                    }
                                    let _ = stream.read(&mut [0]).await;
                                    if case == "cancel" {
                                        closed.notify_one();
                                    }
                                    return;
                                }
                                let (status, mime, extra, body) = response(&url, &head, case);
                                let length = if url.path() == "/forum/tracker.php" && case == "huge" { 3 * 1024 * 1024 } else { body.len() };
                                let header = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\n{extra}Connection: close\r\n\r\n");
                                let _ = stream.write_all(header.as_bytes()).await;
                                let _ = stream.write_all(&body).await;
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
            requests,
            started,
            closed,
            hits,
            case,
            stop,
            server,
        })
    }

    pub fn case(&self, case: &'static str) {
        *self.case.lock().unwrap() = case;
    }
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub async fn stop(self) -> CheckResult {
        let _ = self.stop.send(());
        self.server.await?;
        Ok(())
    }
}

fn response(
    url: &Url,
    head: &str,
    case: &str,
) -> (&'static str, &'static str, &'static str, Vec<u8>) {
    if case == "auto-challenge" && url.path() == "/forum/tracker.php" {
        let query = url
            .query_pairs()
            .find(|(name, _)| name == "nm")
            .map(|(_, value)| value);
        let expected_cookie = if query.as_deref() == Some("cached") {
            "clearance=replaced"
        } else {
            "clearance=fixture-pass"
        };
        if query.as_deref() == Some("persistent")
            || !head.contains(expected_cookie)
            || !head
                .to_ascii_lowercase()
                .contains("user-agent: fixture-webview\r\n")
        {
            return cp1251(
                "403 Forbidden",
                "<script src='/cdn-cgi/challenge-platform/check.js'></script>".into(),
            );
        }
    }
    if url.path() == "/forum/index.php" {
        return cp1251("200 OK", map());
    }
    if case == "redirect" {
        return (
            "302 Found",
            "text/html",
            "Location: https://foreign.example/forum/tracker.php\r\n",
            b"redirect".to_vec(),
        );
    }
    let authenticated = head
        .to_ascii_lowercase()
        .contains("user-agent: fixture-browser\r\n")
        && head.contains("bbsession=fixture-session");
    match case {
        "login" => return cp1251("200 OK", login()),
        "login-until-cookie" if !head.contains("clearance=fixture-pass") => {
            return cp1251("200 OK", login());
        }
        "session" if !authenticated => return cp1251("200 OK", login()),
        "challenge" => {
            return cp1251(
                "403 Forbidden",
                "<script src='/cdn-cgi/challenge-platform/check.js'></script>".into(),
            );
        }
        "input-recaptcha" => return cp1251("200 OK", "<input name='g-recaptcha-response'>".into()),
        "utf8-challenge" => {
            return (
                "403 Forbidden",
                "text/html; charset=UTF-8",
                "",
                "<div class='cf-turnstile'>Проверка</div>"
                    .as_bytes()
                    .to_vec(),
            );
        }
        "gone" if url.path() == "/forum/viewtopic.php" => {
            return cp1251("404 Not Found", "not found".into());
        }
        _ => {}
    }
    let body = if url.path() == "/forum/tracker.php" {
        search(url, case)
    } else if url.path() == "/forum/viewtopic.php" {
        topic(case)
    } else {
        panic!("unexpected fixture request")
    };
    if case == "auto-challenge" && url.path() == "/forum/tracker.php" {
        let (status, mime, _, bytes) = cp1251("200 OK", body);
        return (
            status,
            mime,
            "Set-Cookie: clearance=replaced; Path=/; Secure; HttpOnly\r\n",
            bytes,
        );
    }
    match case {
        "mime" => ("200 OK", "application/json", "", body.into_bytes()),
        "encoding" => ("200 OK", "text/html; charset=UTF-8", "", vec![0xff]),
        "charset" => (
            "200 OK",
            "text/html; charset=unknown",
            "",
            body.into_bytes(),
        ),
        _ => cp1251("200 OK", body),
    }
}

fn cp1251(
    status: &'static str,
    body: String,
) -> (&'static str, &'static str, &'static str, Vec<u8>) {
    let (bytes, _, invalid) = WINDOWS_1251.encode(&body);
    assert!(!invalid);
    (
        status,
        "text/html; charset=Windows-1251",
        "",
        bytes.into_owned(),
    )
}

fn map() -> String {
    let mut body = String::from("<div id='f-map'><ul class='tree-root'>");
    for (parent, child) in [
        (33, 9001),
        (7, 9002),
        (2100, 9003),
        (899, 9004),
        (1016, 9005),
        (862, 9007),
        (99, 9008),
    ] {
        body.push_str(&format!("<li><span><a href='https://feed.fixture.example/atom/f/{parent}.atom'>Feed</a><a href='viewforum.php?f={parent}'>Родитель</a></span><ul><li><span><a href='viewforum.php?f={child}'>Потомок</a></span></li></ul></li>"));
    }
    body.push_str("<li><span><span class='c-title' title='Книги и журналы'></span></span><ul><li><span><a href='viewforum.php?f=9006'>Книги</a></span></li></ul></li></ul></div>");
    body
}

fn login() -> String {
    "<form action='login.php'><input name='login_username'><input name='login_password' type='password'></form>".into()
}

fn search(url: &Url, case: &str) -> String {
    // Decode nm from the complete request target. Url::query_pairs assumes UTF-8,
    // and url.path() would lose the query that this fixture must verify.
    let encoded = url
        .query()
        .unwrap()
        .split('&')
        .find_map(|field| field.strip_prefix("nm="))
        .unwrap();
    let mut bytes = Vec::new();
    let mut remaining = encoded.as_bytes();
    while let Some((&byte, tail)) = remaining.split_first() {
        match byte {
            b'%' => {
                bytes.push(
                    u8::from_str_radix(std::str::from_utf8(&tail[..2]).unwrap(), 16).unwrap(),
                );
                remaining = &tail[2..];
            }
            b'+' => {
                bytes.push(b' ');
                remaining = tail;
            }
            _ => {
                bytes.push(byte);
                remaining = tail;
            }
        }
    }
    let (query, invalid) = WINDOWS_1251.decode_without_bom_handling(&bytes);
    assert!(!invalid);
    let query = query.replace('&', "&amp;").replace('\'', "&#39;");
    let mut body = format!(
        "<input name='nm' value='{query}'><form id='login-form-quick'><input name='login_username'><input name='login_password'></form>"
    );
    if case == "empty" {
        body.push_str("<div id='search-results'>По вашему запросу ничего не найдено</div>");
        return body;
    }
    if case == "malformed-empty" {
        body.push_str("<div id='search-results'>Сервис недоступен</div>");
        return body;
    }
    body.push_str("<table id='tor-tbl'><tbody>");
    for (id, forum) in [
        (42, 9001),
        (43, 9002),
        (44, 9003),
        (45, 9004),
        (46, 9005),
        (47, 9006),
        (48, 9007),
        (49, 9008),
    ] {
        let mut row = format!(
            r#"<tr id='trs-tr-{id}'>
                <td><a href='tracker.php?f={forum}&amp;nm=generated'>Форум</a></td>
                <td class='t-title-col'>
                    <a class='tLink' data-topic_id='{id}' href='viewtopic.php?t={id}'>Синтел [версия 2] &amp; legal</a>
                </td>
                <td class='tor-size' data-ts_text='734003200'>700 MB</td>
                <td data-ts_text='12'><b class='seedmed'>12</b></td>
            </tr>"#
        );
        if id == 42 {
            row = match case {
                "missing-href" => row.replace(" href='viewtopic.php?t=42'", ""),
                "missing-topic-id" => row.replace(" data-topic_id='42'", ""),
                "row-id" => row.replace("trs-tr-42", "trs-tr-50"),
                "foreign-topic" => row.replace(
                    "href='viewtopic.php?t=42'",
                    "href='https://foreign.example/forum/viewtopic.php?t=42'",
                ),
                "unknown-forum" => row.replace("f=9001", "f=99999"),
                "duplicate" => format!("{row}{row}"),
                _ => row,
            };
        }
        body.push_str(&row);
    }
    body.push_str("</tbody></table>");
    body
}

fn topic(case: &str) -> String {
    let title_id = if case == "changed-topic" { 43 } else { 42 };
    let download_id = if case == "changed-download" { 43 } else { 42 };
    let hash = match case {
        "changed-hash" => OTHER,
        "bad-magnet" => "invalid",
        _ => HASH,
    };
    let size = if case == "bad-size" {
        "invalid"
    } else {
        "734003200"
    };
    let seeders = if case == "bad-seeders" {
        "invalid"
    } else {
        "12"
    };
    let magnet = if case == "torrent-only" {
        String::new()
    } else {
        format!(
            "<a class='magnet-link' href='magnet:?xt=urn:btih:{hash}&amp;tr=https%3A%2F%2Ftracker.example%2Ffresh'>magnet</a>"
        )
    };
    format!(
        r#"<div class='nav'>
            <a href='viewforum.php?f=33'>Аниме</a>
            <a href='viewforum.php?f=9001'>Потомок</a>
        </div>
        <a id='topic-title' href='viewtopic.php?t={title_id}'>Синтел [версия 2] &amp; legal</a>
        <table id='t-tor-stats'>
            <tr><td><span class='seed'>Сиды: <b>{seeders}</b></span></td></tr>
        </table>
        <div class='post_body'>Созданный fixture для общественного достояния</div>
        <table class='attach'>
            <tr><td>
                <a class='dl-stub' href='dl.php?t={download_id}'>Скачать торрент</a>
                <span id='tor-size-humn' title='{size}'>700 MB</span>
                {magnet}
            </td></tr>
        </table>"#
    )
}
