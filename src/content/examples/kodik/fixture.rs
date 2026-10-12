use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

use super::CheckResult;

#[path = "responses.rs"]
mod responses;
pub use responses::Case;

pub struct Fixture {
    pub origin: Url,
    pub client: reqwest::Client,
    pub started: Arc<Notify>,
    pub closed: Arc<Notify>,
    pub forms: Arc<AtomicUsize>,
    pub stop: oneshot::Sender<()>,
    pub server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub async fn start() -> CheckResult<Self> {
        let names = [
            "api.fixture.example",
            "kodikplayer.com",
            "cdn.fixture.example",
        ];
        let certified = rcgen::generate_simple_self_signed(names.map(String::from).to_vec())?;
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certified.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()).into(),
            )?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let origin = Url::parse(&format!("https://api.fixture.example:{}/", address.port()))?;
        let mut client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?);
        for name in names {
            client = client.resolve(name, address);
        }
        let client = client.build()?;
        let started = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let forms = Arc::new(AtomicUsize::new(0));
        let signals = (started.clone(), closed.clone(), forms.clone());
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
            origin,
            client,
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
    let target = head.split_whitespace().nth(1).unwrap();
    let url = Url::parse(&format!("https://fixture.example{target}")).unwrap();
    let is_form = url.path() == "/ftor" || url.path() == "/ftor-final";
    let id = if is_form {
        assert!(head.starts_with("POST "));
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("content-type: application/x-www-form-urlencoded"));
        assert!(lower.contains("x-requested-with: xmlhttprequest"));
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
            fields.get("d").map(|value| value.as_ref()),
            Some("fixture-domain")
        );
        assert_eq!(fields.get("info").map(|value| value.as_ref()), Some("{}"));
        assert_eq!(
            fields.get("cdn_is_working").map(|value| value.as_ref()),
            Some("false")
        );
        forms.fetch_add(1, Ordering::SeqCst);
        fields["id"].parse::<u64>().unwrap()
    } else {
        url.path_segments()
            .unwrap()
            .find_map(|part| part.parse::<u64>().ok())
            .unwrap()
    };
    let case = |value: Case| id == value as u64;
    let is_api = url.path().starts_with("/anime/");
    let is_page = url.path().starts_with("/season/")
        || url.path().starts_with("/serial/")
        || url.path().starts_with("/seria/")
        || url.path().starts_with("/video/");
    if !is_api && !is_page && !is_form && (case(Case::Cancel) || case(Case::Timeout)) {
        started.notify_one();
        let mut byte = [0];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            closed.notify_one();
        }
        return;
    }

    let response = responses::response(id, &url);
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}
