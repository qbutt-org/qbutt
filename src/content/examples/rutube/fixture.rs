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

pub const ID: &str = "0123456789abcdef0123456789abcdef";

pub struct Fixture {
    pub origin: Url,
    pub client: reqwest::Client,
    pub started: Arc<Notify>,
    pub closed: Arc<Notify>,
    pub hits: Arc<AtomicUsize>,
    pub payload: Arc<AtomicUsize>,
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
            .timeout(Duration::from_millis(500))
            .build()?;
        let started = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let hits = Arc::new(AtomicUsize::new(0));
        let payload = Arc::new(AtomicUsize::new(0));
        let case = Arc::new(Mutex::new("normal"));
        let (stop, mut shutdown) = oneshot::channel();
        let server = tokio::spawn({
            let origin = origin.clone();
            let started = started.clone();
            let closed = closed.clone();
            let hits = hits.clone();
            let payload = payload.clone();
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
                            let payload = payload.clone();
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
                                    .find(|(name, _)| name == "query")
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
                                if url.path().ends_with(".ts") {
                                    payload.fetch_add(1, Ordering::SeqCst);
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
            payload,
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

fn video(id: &str, title: &str, category: u32) -> Value {
    json!({
        "id": id,
        "title": title,
        "description": "Generated public metadata",
        "video_url": format!("https://rutube.ru/video/{id}/"),
        "category": {"id": category},
        "is_deleted": false,
        "is_hidden": false,
        "is_paid": false,
        "is_locked": false
    })
}

fn response(
    origin: &Url,
    url: &Url,
    query: &str,
    case: &str,
) -> (&'static str, &'static str, String, String) {
    if query.starts_with("redirect-") {
        let address = if query == "redirect-private" {
            "https://127.0.0.1/private"
        } else {
            "https://other.example/catalog"
        };
        return (
            "302 Found",
            "application/json",
            format!("Location: {address}\r\n"),
            String::new(),
        );
    }
    let body = if url.path().starts_with("/api/v1/anime/releases/") {
        let mut episode = json!({
            "id": "generated-episode-2", "ordinal": 2, "rutube_id": ID
        });
        match case {
            "episode-changed" => episode["id"] = json!("changed-episode"),
            "rutube-changed" => episode["rutube_id"] = json!("a".repeat(32)),
            "rutube-missing" => episode["rutube_id"] = Value::Null,
            "rutube-invalid" => episode["rutube_id"] = json!("not-a-video-id"),
            _ => {}
        }
        let mut episodes = vec![json!({
            "id": "generated-episode-1", "ordinal": 1, "rutube_id": "1".repeat(32)
        })];
        if case != "episode-missing" {
            episodes.push(episode);
        }
        json!({
            "id": if case == "release-changed" { 2 } else { 1 },
            "alias": "generated-release",
            "name": {"main": "Generated AniLiberty release"},
            "episodes": episodes
        })
        .to_string()
    } else if url.path().starts_with("/api/search/") {
        if query == "challenge" {
            return (
                "200 OK",
                "text/html",
                String::new(),
                "<html><title>Checking your browser</title></html>".into(),
            );
        }
        let title = format!("Generated {query}");
        let mut first = video(ID, &title, 4);
        let mut rows = vec![
            first.clone(),
            video(&"1".repeat(32), &title, 5),
            video(&"2".repeat(32), &title, 41),
            video(&"3".repeat(32), &title, 57),
        ];
        for (index, flag) in ["is_paid", "is_locked", "is_hidden", "is_deleted"]
            .iter()
            .enumerate()
        {
            let mut excluded = video(&format!("{:032x}", index + 100), &title, 4);
            excluded[*flag] = json!(true);
            rows.push(excluded);
        }
        rows.push(video(&"4".repeat(32), &format!("{title} trailer"), 4));
        match query {
            "empty" => rows.clear(),
            "unwanted" => rows = vec![video(ID, "Generated unwanted trailer", 4)],
            "unknown-category" => rows = vec![video(ID, &title, 57)],
            "multiline" => rows = vec![video(ID, "Generated\nmultiline title", 4)],
            "duplicate" => rows.push(first.clone()),
            "wrong-url" => {
                first["video_url"] = json!("https://other.example/video");
                rows = vec![first];
            }
            "missing-policy" => {
                first.as_object_mut().unwrap().remove("is_paid");
                rows = vec![first];
            }
            "too-many" => {
                rows = (0..101)
                    .map(|id| video(&format!("{id:032x}"), &title, 4))
                    .collect()
            }
            _ => {}
        }
        json!({"current_page": if query == "page" { 2 } else { 1 }, "results": rows}).to_string()
    } else if url.path().starts_with("/api/video/") {
        let id = url.path().trim_end_matches('/').rsplit('/').next().unwrap();
        if id == "f".repeat(32) || id == "e".repeat(32) {
            return (
                if id.starts_with('f') {
                    "404 Not Found"
                } else {
                    "410 Gone"
                },
                "application/json",
                String::new(),
                "{}".into(),
            );
        }
        let mut video = video(
            if case == "identity" {
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            } else {
                id
            },
            "Generated legal sample",
            4,
        );
        video.as_object_mut().unwrap().remove("is_locked");
        match case {
            "paid" => video["is_paid"] = json!(true),
            "locked" => video["is_locked"] = json!(true),
            "hidden" => video["is_hidden"] = json!(true),
            "deleted" => video["is_deleted"] = json!(true),
            _ => {}
        }
        video.to_string()
    } else if url.path().starts_with("/api/play/options/") {
        json!({
            "video_id": ID, "effective_video": if case == "effective" { "other" } else { ID },
            "has_video": case != "no-video", "is_hidden": false,
            "acl_access": {"allowed": case != "access"},
            "limits": if case == "limited" { json!(["restricted"]) } else { json!([]) },
            "drm_token": if case == "drm" { json!("generated-drm") } else { Value::Null },
            "video_balancer": {"m3u8": match case {
                "no-stream" => Value::Null,
                "private-stream" => json!("https://127.0.0.1/private.m3u8"),
                _ => json!(origin.join("/master.m3u8?sign=transient-signature").unwrap())
            }}
        })
        .to_string()
    } else if url.path() != "/master.m3u8" {
        match case {
            "child-master" => "#EXTM3U\n#EXT-X-STREAM-INF:RESOLUTION=640x360\n360.m3u8\n".into(),
            "child-challenge" => "<html>Challenge</html>".into(),
            "truncated-child" => "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\n".into(),
            "child-private-redirect" => {
                return (
                    "302 Found",
                    "application/vnd.apple.mpegurl",
                    "Location: https://127.0.0.1/private.m3u8\r\n".into(),
                    String::new(),
                );
            }
            "child-redirect" if url.path() == "/1080.m3u8" => {
                return (
                    "302 Found",
                    "application/vnd.apple.mpegurl",
                    "Location: /final1080.m3u8\r\n".into(),
                    String::new(),
                );
            }
            _ => "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nsegment.ts\n#EXT-X-ENDLIST\n"
                .into(),
        }
    } else {
        match case {
            "bad-playlist" => "<html>Challenge</html>".into(),
            "no-resolution" => "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nstream.m3u8\n".into(),
            "unsupported-quality" => "#EXTM3U\n#EXT-X-STREAM-INF:RESOLUTION=3840x2160\nstream.m3u8\n".into(),
            "missing-uri" => "#EXTM3U\n#EXT-X-STREAM-INF:RESOLUTION=1920x1080\n".into(),
            "private-child" => "#EXTM3U\n#EXT-X-STREAM-INF:RESOLUTION=1920x1080\nhttps://127.0.0.1/private.m3u8\n".into(),
            "ambiguous-quality" => "#EXTM3U\n#EXT-X-STREAM-INF:RESOLUTION=1920x1080,RESOLUTION=640x360\nstream.m3u8\n".into(),
            "ambiguous-maximum" => "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=200,RESOLUTION=1920x1080\n1080.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=300,RESOLUTION=1920x1080\nother1080.m3u8\n".into(),
            "mirror" => "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=200,RESOLUTION=1920x1080\n1080.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=200,RESOLUTION=1920x1080\nmirror1080.m3u8\n".into(),
            "ambiguous-lower" => "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100,RESOLUTION=1280x720\n720.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=150,RESOLUTION=1280x720\nother720.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=200,RESOLUTION=1920x1080\n1080.m3u8\n".into(),
            "external-audio" => "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"external\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=200,RESOLUTION=1920x1080,AUDIO=\"external\"\n1080.m3u8\n".into(),
            _ => "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100,RESOLUTION=1280x720\n720.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=200,CODECS=\"avc1,mp4a\",RESOLUTION=1920x1080\n1080.m3u8\n".into()
        }
    };
    ("200 OK", "application/json", String::new(), body)
}
