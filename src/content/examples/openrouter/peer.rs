use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};

pub enum Reply {
    Json(Value),
    Status(u16),
    Oversized,
    Delay,
}

pub struct Peer {
    pub http: reqwest::Client,
    pub requests: Arc<Mutex<Vec<Value>>>,
    worker: tokio::task::JoinHandle<()>,
}

impl Peer {
    pub async fn start(replies: Vec<Reply>) -> Self {
        let certificate = rcgen::generate_simple_self_signed(vec!["openrouter.ai".into()]).unwrap();
        let tls = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve("openrouter.ai", address)
            .add_root_certificate(reqwest::Certificate::from_der(certificate.cert.der()).unwrap())
            .timeout(Duration::from_millis(350))
            .build()
            .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let worker = tokio::spawn(async move {
            let acceptor = TlsAcceptor::from(Arc::new(tls));
            for reply in replies {
                let (socket, _) = listener.accept().await.unwrap();
                let mut stream = acceptor.accept(socket).await.unwrap();
                let mut buffer = Vec::new();
                let (header_size, body_size) = loop {
                    let mut chunk = [0u8; 8192];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    buffer.extend_from_slice(&chunk[..count]);
                    assert!(buffer.len() <= 4 * 1024 * 1024);
                    if let Some(offset) = buffer.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header_size = offset + 4;
                        let header = std::str::from_utf8(&buffer[..header_size]).unwrap();
                        assert!(header.starts_with("POST /api/v1/chat/completions HTTP/1.1\r\n"));
                        let header = header.to_ascii_lowercase();
                        assert!(header.contains("authorization: bearer generated-fixture-key\r\n"));
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length: ")
                                    .map(|value| value.parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (header_size, length);
                    }
                };
                while buffer.len() < header_size + body_size {
                    let mut chunk = [0u8; 8192];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    buffer.extend_from_slice(&chunk[..count]);
                }
                let body: Value =
                    serde_json::from_slice(&buffer[header_size..header_size + body_size]).unwrap();
                captured.lock().await.push(body);
                match reply {
                    Reply::Delay => tokio::time::sleep(Duration::from_secs(30)).await,
                    Reply::Oversized => {
                        stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4194305\r\nConnection: close\r\n\r\n")
                            .await
                            .unwrap();
                    }
                    Reply::Status(status) => {
                        let body = "generated-fixture-key must not appear in an error";
                        let response = format!(
                            "HTTP/1.1 {status} Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream.write_all(response.as_bytes()).await.unwrap();
                    }
                    Reply::Json(body) => {
                        let body = serde_json::to_vec(&body).unwrap();
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        stream.write_all(header.as_bytes()).await.unwrap();
                        stream.write_all(&body).await.unwrap();
                    }
                }
                let _ = stream.shutdown().await;
            }
        });
        Self {
            http,
            requests,
            worker,
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
