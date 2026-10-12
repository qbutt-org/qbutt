use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

use super::{CheckResult, pages};

pub struct Fixture {
    pub origin: Url,
    pub client: reqwest::Client,
    pub started: Arc<Notify>,
    pub closed: Arc<Notify>,
    pub hits: Arc<AtomicUsize>,
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
        let origin = Url::parse(&format!("https://fixture.example:{}", address.port()))?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .resolve("fixture.example", address)
            .timeout(Duration::from_millis(500))
            .build()?;
        let started = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let hits = Arc::new(AtomicUsize::new(0));
        let case = Arc::new(Mutex::new("normal"));
        let (stop, mut shutdown) = oneshot::channel();
        let server = tokio::spawn({
            let started = started.clone();
            let closed = closed.clone();
            let hits = hits.clone();
            let case = case.clone();
            async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = &mut shutdown => break,
                        Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); }
                        accepted = listener.accept() => {
                            let (socket, _) = accepted.unwrap();
                            let acceptor = acceptor.clone();
                            let started = started.clone();
                            let closed = closed.clone();
                            let hits = hits.clone();
                            let case = *case.lock().unwrap();
                            connections.spawn(async move {
                                if let Ok(stream) = acceptor.accept(socket).await {
                                    serve(stream, case, hits, started, closed).await;
                                }
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

    pub async fn stop(self) {
        self.stop.send(()).unwrap();
        self.server.await.unwrap();
    }
}

async fn serve(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    case: &'static str,
    hits: Arc<AtomicUsize>,
    started: Arc<Notify>,
    closed: Arc<Notify>,
) {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
        let mut byte = [0];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        request.push(byte[0]);
    }
    hits.fetch_add(1, Ordering::SeqCst);
    if case == "cancel" {
        started.notify_one();
        let mut byte = [0];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            closed.notify_one();
        }
        return;
    }
    if case == "delay" {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let request = String::from_utf8(request).unwrap();
    let path = request.split_whitespace().nth(1).unwrap();
    let mut status = "200 OK";
    let mut mime = "text/html; charset=UTF-8";
    let mut extra = String::new();
    let mut body = pages::document(path, case).into_bytes();
    match case {
        "challenge" => body = b"<h1>Checking your browser</h1>".to_vec(),
        "error" => status = "503 Service Unavailable",
        "missing" => status = "404 Not Found",
        "wrong-mime" => mime = "application/json",
        "utf8" => body = vec![0xff],
        "huge" => extra = "Content-Length: 2097153\r\n".into(),
        "foreign" | "local" | "downgrade" | "loop" => {
            status = "302 Found";
            let target = match case {
                "foreign" => "https://evil.example/search",
                "local" => "https://127.0.0.1/search",
                "downgrade" => "http://fixture.example/search",
                _ => path,
            };
            extra = format!("Location: {target}\r\n");
        }
        "redirect-search" if !path.contains("redirected") => {
            status = "302 Found";
            extra = "Location: /search?q=generated&redirected=1\r\n".into();
        }
        "redirect-work" if path == format!("/works/{}", pages::WORK) => {
            status = "302 Found";
            extra = "Location: /works/999\r\n".into();
        }
        _ => {}
    }
    if case != "huge" {
        extra.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    let header =
        format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{extra}Connection: close\r\n\r\n");
    if stream.write_all(header.as_bytes()).await.is_ok() {
        let _ = stream.write_all(&body).await;
    }
    let _ = stream.shutdown().await;
}
