use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    None,
    WrongFiction,
    NowPaid,
    WrongChapter,
    MissingBody,
    EmptyBody,
    DuplicateChapter,
    InvalidManifest,
    WrongLanguage,
    MissingSubscription,
    InvalidSubscription,
    MissingManifest,
    UnsafeChapter,
}

pub(super) struct Fixture {
    pub(super) origin: Url,
    pub(super) client: reqwest::Client,
    pub(super) fault: Arc<Mutex<Fault>>,
    pub(super) hits: Arc<AtomicUsize>,
    pub(super) cancelled: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub(super) async fn start() -> Result<Self, Box<dyn std::error::Error>> {
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
            .timeout(Duration::from_millis(500))
            .resolve("fixture.example", address)
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .build()?;
        let fault = Arc::new(Mutex::new(Fault::None));
        let hits = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let (stop, mut shutdown) = oneshot::channel();
        let shared_fault = fault.clone();
        let shared_hits = hits.clone();
        let shared_cancelled = cancelled.clone();
        let shared_origin = origin.clone();
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown => break,
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); }
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        shared_hits.fetch_add(1, Ordering::SeqCst);
                        let acceptor = acceptor.clone();
                        let fault = *shared_fault.lock().unwrap();
                        let cancelled = shared_cancelled.clone();
                        let origin = shared_origin.clone();
                        connections.spawn(async move {
                            if let Ok(stream) = acceptor.accept(socket).await {
                                serve(stream, &origin, fault, cancelled).await;
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
            fault,
            hits,
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
    origin: &Url,
    fault: Fault,
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
    let request = String::from_utf8(request).unwrap();
    let path = request.split_whitespace().nth(1).unwrap();
    let url = origin.join(path).unwrap();
    let query = url
        .query_pairs()
        .find(|(key, _)| key == "title")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
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
    let mut mime = "text/html; charset=utf-8";
    let mut extra = String::new();
    let mut body = match url.path() {
        "/fiction/404" => {
            status = "404 Not Found";
            String::new()
        }
        "/fiction/410" => {
            status = "410 Gone";
            String::new()
        }
        "/fiction/42" => book(origin, fault),
        "/fiction/42/generated/chapter/22/second" => chapter(origin, fault),
        _ => search(&query),
    }
    .into_bytes();
    match query.as_str() {
        "mime" => mime = "application/json",
        "utf8" => body = vec![0xff],
        "challenge" | "missing" => body = b"<h1>Verify you are human</h1>".to_vec(),
        "huge-length" => extra = "Content-Length: 2097153\r\n".into(),
        "huge-stream" => {
            body = vec![b'x'; 2097153];
            extra = "Transfer-Encoding: chunked\r\n".into();
            body = [
                format!("{:x}\r\n", body.len()).as_bytes(),
                &body,
                b"\r\n0\r\n\r\n",
            ]
            .concat();
        }
        "foreign" | "local" | "downgrade" | "loop" => {
            status = "302 Found";
            let location = match query.as_str() {
                "foreign" => "https://evil.example/search",
                "local" => "https://127.0.0.1/search",
                "downgrade" => "http://fixture.example/search",
                _ => path,
            };
            extra = format!("Location: {location}\r\n");
        }
        _ => {}
    }
    if !matches!(query.as_str(), "huge-length" | "huge-stream") {
        extra.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    let header =
        format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{extra}Connection: close\r\n\r\n");
    if stream.write_all(header.as_bytes()).await.is_ok() {
        let _ = stream.write_all(&body).await;
    }
    let _ = stream.shutdown().await;
}

fn search(query: &str) -> String {
    let row = r#"<div class="fiction-list-item"><h2 class="fiction-title"><a href="/fiction/42/generated">Generated &amp; legal fiction</a></h2><img data-type="cover" src="https://www.royalroadcdn.com/public/covers-full/42.jpg?time=1"></div>"#;
    let rows = match query {
        "empty" => String::new(),
        "no-cover" => row.replace(
            "https://www.royalroadcdn.com/public/covers-full/42.jpg?time=1",
            "/dist/img/nocover-new-min.png",
        ),
        "duplicate" => row.repeat(2),
        "unsafe-row" => row.replace(
            "/fiction/42/generated",
            "https://evil.example/fiction/42/generated",
        ),
        _ => row.into(),
    };
    format!("<div class='fiction-list'>{rows}</div>")
}

fn book(origin: &Url, fault: Fault) -> String {
    let canonical = origin
        .join(if fault == Fault::WrongFiction {
            "/fiction/43/generated"
        } else {
            "/fiction/42/generated"
        })
        .unwrap();
    let metadata = serde_json::json!({
        "@type": "Book",
        "name": "Generated & legal fiction",
        "description": "<p>Generated synopsis.</p>",
        "url": canonical.as_str(),
        "inLanguage": if fault == Fault::WrongLanguage { "ja" } else { "en-US" }
    });
    let mut chapters = serde_json::json!([
        {
            "id": 22,
            "title": "Second public chapter",
            "order": 1,
            "visible": 1,
            "subscriptionTiers": if fault == Fault::NowPaid { serde_json::json!([1]) } else { serde_json::Value::Null },
            "url": "/fiction/42/generated/chapter/22/second"
        },
        {
            "id": 21,
            "title": "First public chapter",
            "order": 0,
            "visible": 1,
            "subscriptionTiers": [],
            "url": "/fiction/42/generated/chapter/21/first"
        },
        {
            "id": 23,
            "title": "Paid chapter",
            "order": 2,
            "visible": 1,
            "subscriptionTiers": [1],
            "url": "/fiction/42/generated/chapter/23/paid"
        },
        {
            "id": 24,
            "title": "Hidden chapter",
            "order": 3,
            "visible": 0,
            "subscriptionTiers": null,
            "url": "/fiction/42/generated/chapter/24/hidden"
        }
    ]);
    match fault {
        Fault::DuplicateChapter => chapters[1] = chapters[0].clone(),
        Fault::MissingSubscription => {
            chapters[0]
                .as_object_mut()
                .unwrap()
                .remove("subscriptionTiers");
        }
        Fault::InvalidSubscription => chapters[0]["subscriptionTiers"] = false.into(),
        Fault::UnsafeChapter => {
            chapters[0]["url"] =
                "https://evil.example/fiction/42/generated/chapter/22/second".into()
        }
        _ => {}
    }
    let manifest = if fault == Fault::InvalidManifest {
        "invalid".into()
    } else {
        chapters.to_string()
    };
    let script = if fault == Fault::MissingManifest {
        String::new()
    } else {
        format!("<script>\nwindow.chapters = {manifest};\n</script>")
    };
    format!(
        "<link rel='canonical' href='{canonical}'><script type='application/ld+json'>{metadata}</script>{script}"
    )
}

fn chapter(origin: &Url, fault: Fault) -> String {
    let canonical = origin
        .join(if fault == Fault::WrongChapter {
            "/fiction/43/generated/chapter/22/second"
        } else {
            "/fiction/42/generated/chapter/22/second"
        })
        .unwrap();
    let body = match fault {
        Fault::MissingBody => "<div>missing text</div>",
        Fault::EmptyBody => "<div class='chapter-inner chapter-content'></div>",
        _ => {
            "<div class='chapter-inner chapter-content'><p>Original paragraph with <em>emphasis</em>.<span class='noise'>Invisible noise.</span></p><p><strong>Second</strong> paragraph.</p><p style='display:none'>Hidden inline.</p><script>bad()</script></div>"
        }
    };
    format!(
        "<link rel='canonical' href='{canonical}'><style>.noise {{ display: none; speak: never; }}</style><aside>Author note outside chapter.</aside>{body}"
    )
}
