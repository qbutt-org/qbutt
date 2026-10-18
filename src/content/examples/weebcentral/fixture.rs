// SPDX-License-Identifier: GPL-3.0-or-later
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

const BOOK: &str = "01J76XY7E9FNDZ1DBBM6PBJPFK";
const FOREIGN: &str = "01J76XY7E827QQQT0ERKCGH4CD";
const FIRST: &str = "01J76XYYR7VK2XCSHBZT8BZ4C2";
const SECOND: &str = "01J76XYYR7R8A4C9QK5HW9KVCJ";

pub(super) struct Fixture {
    pub(super) client: reqwest::Client,
    pub(super) origin: Url,
    fault: Arc<Mutex<&'static str>>,
    shutdown: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub(super) async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let certified = rcgen::generate_simple_self_signed(vec![
            "fixture.example".into(),
            "hot.planeptune.us".into(),
            "scans.lastation.us".into(),
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
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(500))
            .resolve("fixture.example", address)
            .resolve("hot.planeptune.us", address)
            .resolve("scans.lastation.us", address)
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .build()?;
        let fault = Arc::new(Mutex::new(""));
        let shared_fault = fault.clone();
        let shared_origin = origin.clone();
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
                let origin = shared_origin.clone();
                let fault = *shared_fault.lock().unwrap();
                connections.spawn(async move {
                    let Ok(mut tls) = acceptor.accept(socket).await else {
                        return;
                    };
                    let mut bytes = Vec::new();
                    while bytes.len() < 16 * 1024 {
                        let mut buffer = [0u8; 1024];
                        let Ok(size) = tls.read(&mut buffer).await else {
                            return;
                        };
                        if size == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&buffer[..size]);
                        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&bytes);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let (status, kind, body) = response(&origin, path, fault);
                    if path.contains("timeout") {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    let header = format!(
                        "HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
            shutdown,
            server,
        })
    }

    pub(super) fn set_fault(&self, fault: &'static str) {
        *self.fault.lock().unwrap() = fault;
    }

    pub(super) async fn stop(self) {
        let _ = self.shutdown.send(());
        let _ = self.server.await;
    }
}

pub(super) fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC")
        .unwrap()
}

fn response(origin: &Url, path: &str, fault: &str) -> (u16, &'static str, Vec<u8>) {
    let url = origin.join(path).unwrap();
    if url.path().ends_with(".png") {
        return (200, "image/png", png());
    }
    let mut status = 200;
    let body = if url.path() == "/search/data" {
        assert!(
            url.query_pairs()
                .any(|(key, value)| key == "adult" && value == "Any")
        );
        let query = url
            .query_pairs()
            .find(|(key, _)| key == "text")
            .unwrap()
            .1
            .into_owned();
        match query.as_str() {
            "empty" => "<div role=alert><span>No results found</span></div>".into(),
            "malformed" => "<html><h1>Something changed</h1></html>".into(),
            "blocked" => {
                status = 403;
                "<html>Blocked</html>".into()
            }
            "oversized" => " ".repeat(2 * 1024 * 1024 + 1),
            _ => format!(
                "<article class=bg-base-300><a class=line-clamp-1 href='{origin}series/{BOOK}/Generated'>Generated Adventure</a></article>"
            ),
        }
    } else if url.path().ends_with("/full-chapter-list") {
        let chapter = |id, number| {
            format!(
                "<div><a class=flex-1 href='/chapters/{id}'><span class=grow><span>Chapter {number}</span></span></a></div>"
            )
        };
        let mut chapters = chapter(SECOND, 2)
            + &chapter(
                if fault == "duplicate_chapter" {
                    SECOND
                } else {
                    FIRST
                },
                1,
            );
        chapters.push_str("<a href='#top' aria-label='back to top'>Top</a>");
        if fault == "large_chapters" {
            chapters.push_str(&" ".repeat(2 * 1024 * 1024));
        }
        if fault == "oversized_chapters" {
            chapters.push_str(&" ".repeat(4 * 1024 * 1024));
        }
        chapters
    } else if url.path().starts_with("/series/") {
        if fault == "missing_publication" {
            return (404, "text/html", b"<h1>Missing publication</h1>".to_vec());
        }
        let identity = if fault == "changed_publication" {
            FOREIGN
        } else {
            BOOK
        };
        let adult = match fault {
            "missing_adult" => String::new(),
            _ => {
                let value = if fault == "adult" { "Yes" } else { "No" };
                format!("<li><strong>Adult Content: </strong><a>{value}</a></li>")
            }
        };
        format!(
            "<html><link rel=canonical href='{origin}series/{identity}/Generated'><h1>Generated Adventure</h1><ul>{adult}<li><strong>Released: </strong>2026</li></ul></html>"
        )
    } else if url.path().ends_with("/images") {
        if fault == "missing_images" {
            return (200, "text/html", b"<html>Missing</html>".to_vec());
        }
        let image_origin = match fault {
            "legacy_cdn" => "https://hot.planeptune.us",
            "unsafe_image" => "https://127.0.0.1",
            "foreign_image" => "https://scans.lastation.us.evil.invalid",
            "image_port" => "https://scans.lastation.us:444",
            _ => "https://scans.lastation.us",
        };
        // The fixture's client connects this production hostname to its private
        // TLS server. No production code admits loopback or a fixture host.
        let second = if fault == "duplicate_image" { 1 } else { 2 };
        let second_alt = if fault == "page_order" { 1 } else { 2 };
        format!(
            "<section id=chapter-images><img alt='Page 1' src='{image_origin}/manga/generated/1.png'><img alt='Page {second_alt}' src='{image_origin}/manga/generated/{second}.png'></section>"
        )
    } else {
        let identity = if fault == "changed_chapter" {
            FIRST
        } else {
            SECOND
        };
        let book = if fault == "foreign_publication" {
            FOREIGN
        } else {
            BOOK
        };
        format!(
            "<html><link rel=canonical href='{origin}chapters/{identity}'><a class=col-span-6 href='{origin}series/{book}/Generated'>Generated Adventure</a></html>"
        )
    };
    (status, "text/html", body.into_bytes())
}
