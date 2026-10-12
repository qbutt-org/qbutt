use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};

use super::CheckResult;

#[path = "responses.rs"]
mod responses;

pub struct Fixture {
    pub address: SocketAddr,
    pub certificate: Vec<u8>,
    pub started: Arc<Notify>,
    pub closed: Arc<Notify>,
    pub forms: Arc<AtomicUsize>,
    pub stop: oneshot::Sender<()>,
    pub server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub async fn start() -> CheckResult<Self> {
        let names = [
            "my.animespirit.ru",
            "kodikplayer.com",
            "cdn.fixture.example",
        ];
        let certified = rcgen::generate_simple_self_signed(names.map(String::from).to_vec())?;
        let certificate = certified.cert.der().to_vec();
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certified.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()).into(),
            )?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let started = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let forms = Arc::new(AtomicUsize::new(0));
        let signals = (
            Arc::clone(&started),
            Arc::clone(&closed),
            Arc::clone(&forms),
        );
        let (stop, mut shutdown) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown => break,
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); },
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let acceptor = acceptor.clone();
                        let signals = signals.clone();
                        connections.spawn(async move {
                            if let Ok(stream) = acceptor.accept(socket).await {
                                serve(stream, signals.0, signals.1, signals.2).await;
                            }
                        });
                    }
                }
            }
            connections.shutdown().await;
        });
        Ok(Self {
            address,
            certificate,
            started,
            closed,
            forms,
            stop,
            server,
        })
    }
}

async fn serve(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    started: Arc<Notify>,
    closed: Arc<Notify>,
    forms: Arc<AtomicUsize>,
) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") && head.len() < 32 * 1024 {
        let mut byte = [0];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let method = head.split_whitespace().next().unwrap();
    let target = head.split_whitespace().nth(1).unwrap();
    let host = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("host: ")
                .map(str::to_owned)
        })
        .unwrap();

    if target.starts_with("/serial/44/") {
        started.notify_one();
        let mut byte = [0];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            closed.notify_one();
        }
        return;
    }

    if target == "/ftor" {
        assert_eq!(method, "POST");
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("content-type: application/x-www-form-urlencoded"));
        let length: usize = lower
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await.unwrap();
        let fields: std::collections::HashMap<_, _> = url::form_urlencoded::parse(&body).collect();
        assert_eq!(
            fields.get("d").map(|field| field.as_ref()),
            Some("fixture-domain")
        );
        assert_eq!(
            fields.get("type").map(|field| field.as_ref()),
            Some("seria")
        );
        assert_eq!(fields.get("id").map(|field| field.as_ref()), Some("4202"));
        assert_eq!(
            fields.get("hash").map(|field| field.as_ref()),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        forms.fetch_add(1, Ordering::SeqCst);
    } else {
        assert_eq!(method, "GET");
    }

    let response = responses::response(&host, target);
    let _ = stream.write_all(&response).await;
    let _ = stream.shutdown().await;
}
