use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

use super::CheckResult;

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
        let origin = Url::parse(&format!("https://fixture.example:{}/", address.port()))?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(reqwest::Certificate::from_der(certified.cert.der())?)
            .resolve("fixture.example", address)
            .timeout(Duration::from_millis(400))
            .build()?;
        let started = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let hits = Arc::new(AtomicUsize::new(0));
        let case = Arc::new(Mutex::new("normal"));
        let (stop, mut shutdown) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let started = started.clone();
            let closed = closed.clone();
            let hits = hits.clone();
            let case = case.clone();
            async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = &mut shutdown => break,
                        Some(result) = connections.join_next(), if !connections.is_empty() => {
                            result.unwrap();
                        }
                        socket = listener.accept() => {
                            let (socket, _) = socket.unwrap();
                            let acceptor = acceptor.clone();
                            let origin = origin.clone();
                            let started = started.clone();
                            let closed = closed.clone();
                            let hits = hits.clone();
                            let case = case.clone();
                            connections.spawn(async move {
                                let Ok(mut stream) = acceptor.accept(socket).await else {
                                    return;
                                };
                                let mut head = Vec::new();
                                while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                                    let mut byte = [0];
                                    if stream.read_exact(&mut byte).await.is_err() {
                                        return;
                                    }
                                    head.push(byte[0]);
                                }
                                hits.fetch_add(1, Ordering::SeqCst);
                                let head = String::from_utf8(head).unwrap();
                                assert!(head.starts_with("GET "));
                                let target = head.split_whitespace().nth(1).unwrap();
                                let url = origin.join(target).unwrap();
                                let query = url.query_pairs()
                                    .find(|(key, _)| key == "q")
                                    .map(|(_, value)| value.into_owned())
                                    .unwrap_or_default();
                                if query == "cancel" || query == "delay" {
                                    if query == "cancel" {
                                        started.notify_one();
                                    }
                                    let _ = stream.read(&mut [0]).await;
                                    if query == "cancel" {
                                        closed.notify_one();
                                    }
                                    return;
                                }
                                let case = *case.lock().unwrap();
                                let (status, mime, extra, body) = response(&origin, &url, &query, case);
                                let length = if query == "huge" {
                                    25 * 1024 * 1024
                                } else {
                                    body.len()
                                };
                                let header = format!(
                                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\n{extra}Connection: close\r\n\r\n"
                                );
                                let _ = stream.write_all(header.as_bytes()).await;
                                let _ = stream.write_all(body.as_bytes()).await;
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

    pub async fn stop(self) -> CheckResult<()> {
        let _ = self.stop.send(());
        self.server.await?;
        Ok(())
    }
}

fn response(
    origin: &Url,
    url: &Url,
    query: &str,
    case: &str,
) -> (&'static str, &'static str, String, String) {
    if query.starts_with("redirect-") {
        let target = if query == "redirect-private" {
            "https://127.0.0.1/private"
        } else {
            "https://other.example/private"
        };
        return (
            "302 Found",
            "application/json",
            format!("Location: {target}\r\n"),
            String::new(),
        );
    }
    if query == "http" {
        return (
            "503 Unavailable",
            "application/json",
            String::new(),
            "{}".into(),
        );
    }
    if query == "invalid-json" {
        return ("200 OK", "application/json", String::new(), "{".into());
    }
    if url.path() == "/ranobe/7" {
        return (
            "308 Permanent Redirect",
            "text/html",
            "Location: /ranobe/7-legal\r\n".into(),
            String::new(),
        );
    }
    if url.path() == "/ranobe/7-legal" {
        let id = if case == "book-id" { 8 } else { 7 };
        let body = format!(
            "<link rel='canonical' href='{origin}ranobe/{id}-legal'><div class='book-copy'><h1 class='book-title-cyrillic'>Legal story</h1><p class='book-original-title'>Original title</p><a class='book-kicker-year' href='/year/2026'>2026</a></div><div class='book-description-copy'><p>Generated public description.</p></div>"
        );
        return ("200 OK", "text/html", String::new(), body);
    }

    let value = if url.path() == "/api/search" {
        if query == "empty" {
            json!({ "books": [] })
        } else {
            assert!(matches!(query, "generated & legal" | "huge"));
            json!({ "books": [{ "id": 7, "slug": "legal", "title": "Legal story", "originalTitle": "Original title", "posterUrl": "/api/media/9?size=medium", "year": 2026 }] })
        }
    } else if url.path() == "/api/books/7/chapters" {
        assert!(
            url.query_pairs()
                .any(|(key, value)| key == "sort" && value == "asc")
        );
        let offset: usize = url
            .query_pairs()
            .find(|(key, _)| key == "offset")
            .unwrap()
            .1
            .parse()
            .unwrap();
        let mut chapter = chapter(if offset == 0 || case == "duplicate" {
            100
        } else {
            101
        });
        if case == "chapter-parent" {
            chapter["bookId"] = json!(8);
        }
        if case == "paid-list" {
            chapter["paid"] = json!(true);
        }
        if case == "draft-list" {
            chapter["isDraft"] = json!(true);
        }
        if case == "ai-list" {
            chapter["translationOptions"][0]["memberDetails"][0]["isAi"] = json!(true);
        }
        if case == "unknown-ai" {
            chapter["translationOptions"][0]["memberDetails"][0]
                .as_object_mut()
                .unwrap()
                .remove("isAi");
        }
        if case == "missing-branch" {
            chapter["translationOptions"][0]["slug"] = json!("other");
        }
        if case == "duplicate-branches" {
            let branch = chapter["translationOptions"][0].clone();
            chapter["translationOptions"]
                .as_array_mut()
                .unwrap()
                .push(branch);
        }
        let total = if case == "too-many" { 30_001 } else { 2 };
        let next = if offset == 0 && case != "partial" {
            Some(if case == "pagination" { 0 } else { 1 })
        } else {
            None
        };
        json!({ "items": [chapter], "total": total, "offset": offset, "nextOffset": next })
    } else if matches!(url.path(), "/api/chapters/101" | "/api/chapters/201") {
        let alternate = url.path() == "/api/chapters/201";
        let branch = if alternate { "alternate" } else { "main" };
        assert!(
            url.query_pairs()
                .any(|(key, value)| key == "translation" && value == branch)
        );
        json!({ "chapter": {
            "id": if case == "body-id" { 100 } else if alternate { 201 } else { 101 },
            "bookId": if case == "body-book" { 8 } else { 7 },
            "translationBranch": { "slug": if case == "body-branch" { "other" } else { branch } },
            "html": if case == "empty-text" { "<script>generated()</script>" } else { "<p>First generated paragraph.</p><p><ruby>語<rt>ご</rt></ruby> <em>second</em><br>line</p><script>forbidden()</script><iframe src='https://127.0.0.1/private'></iframe>" },
            "paid": case == "paid-body",
            "isDraft": case == "draft-body"
        } })
    } else {
        panic!("unexpected generated request: {}", url.path());
    };
    (
        "200 OK",
        "application/json",
        String::new(),
        value.to_string(),
    )
}

fn chapter(id: u64) -> Value {
    json!({
        "id": id,
        "bookId": 7,
        "title": format!("Generated chapter {id}"),
        "volume": 2,
        "number": if id == 100 { json!(2.5) } else { json!(2) },
        "isDraft": false,
        "publishedAt": "2026-10-01T00:00:00Z",
        "translationOptions": [
            { "slug": "main", "name": "Human translation", "memberDetails": [{ "name": "Generated team", "isAi": false }] },
            { "slug": "alternate", "name": "Alternate translation", "chapterId": id + 100, "memberDetails": [{ "name": "Second team", "isAi": false }] },
            { "slug": "ai-translation", "name": "AI translation", "memberDetails": [{ "name": "Generated AI", "isAi": true }] }
        ]
    })
}
