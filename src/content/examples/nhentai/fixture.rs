// SPDX-License-Identifier: GPL-3.0-or-later
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

pub(super) struct Fixture {
    pub(super) client: reqwest::Client,
    pub(super) origin: Url,
    fault: Arc<Mutex<&'static str>>,
    pub(super) image_requests: Arc<Mutex<Vec<String>>>,
    shutdown: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub(super) async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let certified = rcgen::generate_simple_self_signed(vec![
            "fixture.example".into(),
            "i1.nhentai.net".into(),
            "t1.nhentai.net".into(),
        ])?;
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certified.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()).into(),
            )?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:443").await?;
        let address = listener.local_addr()?;
        let origin = Url::parse("https://fixture.example/")?;
        let client = reqwest::Client::builder()
            .default_headers(reqwest::header::HeaderMap::from_iter([(
                reqwest::header::REFERER,
                reqwest::header::HeaderValue::from_static("https://fixture.example/"),
            )]))
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(500))
            .resolve("fixture.example", address)
            .resolve("i1.nhentai.net", address)
            .resolve("t1.nhentai.net", address)
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .build()?;
        let fault = Arc::new(Mutex::new(""));
        let image_requests = Arc::new(Mutex::new(Vec::new()));
        let shared_fault = Arc::clone(&fault);
        let shared_images = Arc::clone(&image_requests);
        let (shutdown, mut stop) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                let socket = tokio::select! {
                    _ = &mut stop => break,
                    result = listener.accept() => match result {
                        Ok((socket, _)) => socket,
                        Err(_) => break,
                    },
                };
                let acceptor = acceptor.clone();
                let fault = *shared_fault.lock().unwrap();
                let images = Arc::clone(&shared_images);
                connections.spawn(async move {
                    let Ok(mut tls) = acceptor.accept(socket).await else { return };
                    let mut bytes = Vec::new();
                    while bytes.len() < 16 * 1024 {
                        let mut buffer = [0u8; 1024];
                        let Ok(size) = tls.read(&mut buffer).await else { return };
                        if size == 0 { return }
                        bytes.extend_from_slice(&buffer[..size]);
                        if bytes.windows(4).any(|window| window == b"\r\n\r\n") { break }
                    }
                    let request = String::from_utf8_lossy(&bytes);
                    let path = request.lines().next()
                        .and_then(|line| line.split_whitespace().nth(1)).unwrap_or("/");
                    if path.starts_with("/galleries/") {
                        images.lock().unwrap().push(path.into());
                        assert!(request.to_ascii_lowercase().contains("referer: https://fixture.example/\r\n"));
                    }
                    if path.contains("timeout") {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    let (status, kind, body, redirect) = response(path, fault);
                    let header = format!(
                        "HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n{redirect}\r\n",
                        body.len()
                    );
                    if tls.write_all(header.as_bytes()).await.is_ok() {
                        let _ = tls.write_all(&body).await;
                        let _ = tls.shutdown().await;
                    }
                });
            }
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        });
        Ok(Self {
            client,
            origin,
            fault,
            image_requests,
            shutdown,
            server,
        })
    }

    pub(super) fn set_fault(&self, fault: &'static str) {
        *self.fault.lock().unwrap() = fault;
    }

    pub(super) async fn stop(self) {
        let _ = self.shutdown.send(());
        self.server.await.unwrap();
    }
}

pub(super) fn png(page: u32) -> Vec<u8> {
    // Authored solid-color 4x3 PNGs distinguish the two exact pages after decoding.
    let image = if page == 1 {
        "iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAEElEQVR4nGO4o6EBRww4OQAa3g4RLYR9sQAAAABJRU5ErkJggg=="
    } else {
        "iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAD0lEQVR4nGNgmPsdgXByAGXqEvFaQ2BaAAAAAElFTkSuQmCC"
    };
    base64::engine::general_purpose::STANDARD
        .decode(image)
        .unwrap()
}

fn response(path: &str, fault: &str) -> (u16, &'static str, Vec<u8>, &'static str) {
    let url = Url::parse("https://fixture.example")
        .unwrap()
        .join(path)
        .unwrap();
    if path.starts_with("/galleries/") {
        return match fault {
            "image_redirect" => (
                302,
                "text/html",
                Vec::new(),
                "Location: https://foreign.example/2.png\r\n",
            ),
            "image_mime" => (200, "text/html", b"<script>generated</script>".to_vec(), ""),
            "image_oversized" => (200, "image/png", vec![0; 24 * 1024 * 1024 + 1], ""),
            _ => (
                200,
                "image/png",
                png(if path.ends_with("/1.png") { 1 } else { 2 }),
                "",
            ),
        };
    }
    let body = if url.path() == "/api/v2/cdn" {
        let server = match fault {
            "cdn_private" => "https://127.0.0.1",
            "cdn_missing_number" => "https://i.nhentai.net/",
            "cdn_foreign" => "https://i1.nhentai.net.foreign.example",
            "cdn_path" => "https://i1.nhentai.net/other/",
            "cdn_query" => "https://i1.nhentai.net/?token=generated",
            "cdn_port" => "https://i1.nhentai.net:8443/",
            "cdn_credentials" => "https://generated@i1.nhentai.net/",
            "cdn_http" => "http://i1.nhentai.net/",
            _ => "https://i1.nhentai.net/",
        };
        json!({"image_servers": if fault == "cdn_empty" { Vec::<&str>::new() } else { vec![server] },
            "thumb_servers": ["https://t1.nhentai.net/"]})
    } else if url.path() == "/api/v2/search" {
        assert!(
            url.query_pairs()
                .any(|(key, value)| key == "page" && value == "1")
        );
        let query = url
            .query_pairs()
            .find(|(key, _)| key == "query")
            .unwrap()
            .1
            .into_owned();
        match query.as_str() {
            "empty" => json!({"result": [], "num_pages": 0, "per_page": 25}),
            "malformed" => return (200, "text/html", b"<html>Challenge</html>".to_vec(), ""),
            "blocked" => return (403, "text/html", b"Blocked".to_vec(), ""),
            "oversized" => {
                return (
                    200,
                    "application/json",
                    vec![b' '; 24 * 1024 * 1024 + 1],
                    "",
                );
            }
            _ => {
                let row = json!({"id": 123, "media_id": "456", "english_title": "Generated Adventure",
                    "japanese_title": "生成された冒険", "thumbnail": "galleries/456/thumb.webp",
                    "num_pages": 2, "blacklisted": false});
                let mut blocked = row.clone();
                blocked["id"] = json!(124);
                blocked["blacklisted"] = json!(true);
                json!({"result": [row, blocked], "num_pages": 1, "per_page": 25})
            }
        }
    } else if url.path() == "/api/v2/galleries/123" {
        if fault == "missing" {
            return (404, "application/json", b"{}".to_vec(), "");
        }
        let mut gallery = json!({
            "id": 123, "media_id": "456", "title": {"pretty": "Generated Adventure",
                "english": "Generated Adventure [Generated group]", "japanese": "生成された冒険"},
            "cover": {"path": "galleries/456/cover.webp.webp"}, "num_pages": 2,
            "scanlator": "Generated group", "tags": [
                {"type": "language", "name": "english"}, {"type": "language", "name": "translated"},
                {"type": "tag", "name": "non-h"}],
            "pages": [{"number": 2, "path": "galleries/456/2.png"}, {"number": 1, "path": "galleries/456/1.png"}]
        });
        match fault {
            "changed" => gallery["id"] = json!(124),
            "missing_pages" => gallery
                .as_object_mut()
                .unwrap()
                .remove("pages")
                .map(|_| ())
                .unwrap(),
            "empty_pages" => gallery["pages"] = json!([]),
            "excessive_pages" => gallery["num_pages"] = json!(2001),
            "page_gap" => gallery["pages"][0]["number"] = json!(3),
            "duplicate_page" => gallery["pages"][0]["number"] = json!(1),
            "duplicate_path" => gallery["pages"][0]["path"] = gallery["pages"][1]["path"].clone(),
            "foreign_media" => gallery["pages"][0]["path"] = json!("galleries/999/2.png"),
            "absolute_path" => gallery["pages"][0]["path"] = json!("https://foreign.example/2.png"),
            "traversal" => gallery["pages"][0]["path"] = json!("galleries/456/../2.png"),
            "encoded_traversal" => {
                gallery["pages"][0]["path"] = json!("galleries/456/%2e%2e%2f2.png")
            }
            "image_query" => {
                gallery["pages"][0]["path"] = json!("galleries/456/2.png?secret=generated")
            }
            "invalid_media" => gallery["media_id"] = json!("0456"),
            _ => {}
        }
        gallery
    } else {
        return (404, "application/json", b"{}".to_vec(), "");
    };
    (
        200,
        "application/json",
        serde_json::to_vec::<Value>(&body).unwrap(),
        "",
    )
}
