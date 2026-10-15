use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

use super::{AcceptanceResult, FIRST_ID, METADATA, SECOND_ID, detail_page, search_page};

pub(super) struct Fixture {
    pub(super) origin: Url,
    pub(super) client: reqwest::Client,
    pub(super) changed: Arc<AtomicBool>,
    pub(super) cancelled: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub(super) async fn start() -> AcceptanceResult<Self> {
        let certified = rcgen::generate_simple_self_signed(vec![
            "fixture.example".into(),
            "svr1.file-me.top".into(),
        ])?;
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
            .resolve("svr1.file-me.top", address)
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .build()?;
        let changed = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let (stop, mut shutdown) = oneshot::channel();
        let server_origin = origin.clone();
        let server_changed = changed.clone();
        let server_cancelled = cancelled.clone();
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown => break,
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); }
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let acceptor = acceptor.clone();
                        let origin = server_origin.clone();
                        let changed = server_changed.load(Ordering::SeqCst);
                        let cancelled = server_cancelled.clone();
                        connections.spawn(async move {
                            if let Ok(stream) = acceptor.accept(socket).await {
                                serve(stream, origin, changed, cancelled).await;
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
            stop,
            server,
        })
    }

    pub(super) async fn stop(self) {
        self.stop.send(()).unwrap();
        self.server.await.unwrap();
    }
}

async fn serve(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    origin: Url,
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
    let header = String::from_utf8(request).unwrap();
    let mut first_line = header.split_whitespace();
    let method = first_line.next().unwrap();
    let path = first_line.next().unwrap();
    let url = origin.join(path).unwrap();
    let query = url
        .query_pairs()
        .find(|(name, _)| name == "s")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let case = url
        .query_pairs()
        .find(|(name, _)| name == "case")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let length = header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .unwrap_or(0);
    assert!(length <= 16384);
    let mut posted = vec![0; length];
    if stream.read_exact(&mut posted).await.is_err() {
        return;
    }
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
    let mut mime = "text/html; charset=UTF-8";
    let mut extra = String::new();
    let first = url.path() == format!("/{FIRST_ID}.html");
    let second = url.path() == format!("/{SECOND_ID}.html");
    let mut body = if url.path() == "/metadata" {
        assert_eq!(method, "GET");
        assert!(!header.contains("fixture=stage1"));
        assert!(!header.to_ascii_lowercase().contains("authorization:"));
        assert_eq!(length, 0);
        mime = "application/x-bittorrent";
        METADATA.to_vec()
    } else if case == "redirect-loop" {
        status = "302 Found";
        extra.push_str(&format!("Location: {}\r\n", url.as_str()));
        Vec::new()
    } else if first || second {
        let id = if first { FIRST_ID } else { SECOND_ID };
        let op = "download2";
        if method == "GET" {
            extra.push_str("Set-Cookie: fixture=stage1; Path=/; Secure\r\n");
            match case.as_str() {
                "gate" => b"<div class='cf-turnstile'></div>".to_vec(),
                "rate" => b"You have to wait 1 minute till next download".to_vec(),
                "wrong-id" => host_form("xxxxxxxxyyyy", op).into_bytes(),
                "external-action" => host_form(id, op)
                    .replace("action=''", "action='https://evil.example/'")
                    .into_bytes(),
                "duplicate-op" => host_form(id, op)
                    .replace(
                        "</form>",
                        "<input type='hidden' name='op' value='other'></form>",
                    )
                    .into_bytes(),
                _ => host_form(id, op).into_bytes(),
            }
        } else {
            assert_eq!(method, "POST");
            if case == "cancel" {
                let mut byte = [0];
                if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                    cancelled.fetch_add(1, Ordering::SeqCst);
                }
                return;
            }
            assert!(
                header
                    .to_ascii_lowercase()
                    .contains("cookie: fixture=stage1")
            );
            #[cfg(windows)]
            assert!(
                header
                    .to_ascii_lowercase()
                    .contains("authorization: bearer generated-fixture-authorization")
            );
            let fields = url::form_urlencoded::parse(&posted).collect::<Vec<_>>();
            assert!(
                fields
                    .iter()
                    .any(|(name, value)| name == "id" && value == id)
            );
            let operation = fields
                .iter()
                .find(|(name, _)| name == "op")
                .unwrap()
                .1
                .as_ref();
            if case == "unsupported" {
                b"<h1>Try another download page</h1>".to_vec()
            } else if case.starts_with("redirect") {
                status = if case == "redirect-replay" {
                    "307 Temporary Redirect"
                } else {
                    "302 Found"
                };
                let target = match case.as_str() {
                    "redirect-foreign" => "https://evil.example/metadata".into(),
                    "redirect-private" => "https://127.0.0.1/metadata".into(),
                    "redirect-http" => "http://svr1.file-me.top/metadata".into(),
                    _ => format!(
                        "https://svr1.file-me.top:{}/metadata",
                        origin.port().unwrap()
                    ),
                };
                extra.push_str(&format!("Location: {target}\r\n"));
                Vec::new()
            } else {
                assert_eq!(operation, "download2");
                mime = "application/x-bittorrent";
                match case.as_str() {
                    "malformed" => b"d4:infod4:name4:faileee".to_vec(),
                    "trailing" => [METADATA, b"extra"].concat(),
                    "empty" => Vec::new(),
                    "large" => {
                        extra.push_str("Content-Length: 10485761\r\n");
                        Vec::new()
                    }
                    _ => METADATA.to_vec(),
                }
            }
        }
    } else if url.query().is_some() {
        search_page(
            &origin,
            if query == "wrong-query" {
                "other"
            } else {
                &query
            },
        )
        .into_bytes()
    } else {
        match url.path() {
            "/missing/" => status = "404 Not Found",
            "/gone/" => status = "410 Gone",
            _ => {}
        }
        let detail = detail_page(&origin, changed);
        if url.path() == "/generated-%ce%b4/" {
            detail
                .replace("post-42", "post-43")
                .replace("/generated-game/", "/generated-%ce%b4/")
                .into_bytes()
        } else {
            detail.into_bytes()
        }
    };
    match query.as_str() {
        "json" => mime = "application/json",
        "utf8" => body = vec![0xff],
        "challenge" => body = b"<h1>Checking your browser</h1>".to_vec(),
        "huge-length" => extra = "Content-Length: 2097153\r\n".into(),
        "foreign" | "local" | "downgrade" | "loop" | "redirect" => {
            status = "302 Found";
            let location = match query.as_str() {
                "foreign" => "https://evil.example/catalog",
                "local" => "https://127.0.0.1/catalog",
                "downgrade" => "http://fixture.example/catalog",
                "loop" => path,
                _ => "/redirected?s=redirect",
            };
            if query != "redirect" || url.path() != "/redirected" {
                extra = format!("Location: {location}\r\n");
            } else {
                status = "200 OK";
            }
        }
        _ => {}
    }
    if query != "huge-length" && case != "large" {
        extra.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    let header =
        format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{extra}Connection: close\r\n\r\n");
    if stream.write_all(header.as_bytes()).await.is_ok() {
        let _ = stream.write_all(&body).await;
    }
    let _ = stream.shutdown().await;
}

fn host_form(id: &str, operation: &str) -> String {
    format!(
        "<form method='POST' action=''>
            <input type='hidden' name='op' value='{operation}'>
            <input type='hidden' name='id' value='{id}'>
            <input type='hidden' name='rand' value='fixture-rand'>
            <input type='hidden' name='fname' value='GeneratedLegal.torrent'>
        </form>"
    )
}
