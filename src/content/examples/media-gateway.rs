//! Generated HTTP/TLS playback and socket-lifetime acceptance. The gateway
//! source is compiled unchanged. Only this example maps two fixture DNS names
//! to local sockets; the product has no policy constructor or override.

pub use qbutt_content::{Error, Result, TorrentMetadata};

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[allow(dead_code)]
#[path = "../src/http.rs"]
mod policy;

mod http {
    pub(crate) use super::policy::validate_media_url;
    use std::io;
    use std::net::SocketAddr;
    use url::Url;

    pub(crate) async fn resolve_public_url(url: &Url) -> io::Result<Vec<SocketAddr>> {
        validate_media_url(url).map_err(|_| io::ErrorKind::PermissionDenied)?;
        match url.host_str() {
            Some("fixture.example" | "child.example") => Ok(vec![SocketAddr::from((
                [127, 0, 0, 1],
                url.port_or_known_default().unwrap(),
            ))]),
            Some("mixed.example") => super::policy::public_addresses(vec![
                SocketAddr::from(([1, 1, 1, 1], 443)),
                SocketAddr::from(([127, 0, 0, 1], 443)),
            ]),
            _ => super::policy::resolve_public_url(url).await,
        }
    }
}

#[path = "../src/media.rs"]
mod media;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use media::MediaGateway;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

type AcceptanceResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> AcceptanceResult<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let media_dir = args.first().map(PathBuf::from);
    let mpv = args.get(1).map(PathBuf::from);
    let plain = Fixture::start(false, media_dir.clone()).await?;
    let tls = Fixture::start(true, media_dir).await?;
    let gateway = MediaGateway::start().await?;
    let proxy = gateway.proxy_url().clone();
    assert_eq!(proxy.username(), "qbutt");
    assert_eq!(proxy.password().unwrap().len(), 48);
    assert_eq!(proxy.host_str(), Some("127.0.0.1"));
    assert!(proxy.query().is_none() && proxy.fragment().is_none());
    let auth = authorization(&proxy);
    let origin = plain.origin.as_str().trim_end_matches('/');
    let unauthorized = request(&proxy, &format!("GET {origin}/legal HTTP/1.1\r\n\r\n")).await?;
    assert!(unauthorized.starts_with(b"HTTP/1.1 407"));
    let wrong = request(
        &proxy,
        &format!("GET {origin}/legal HTTP/1.1\r\nProxy-Authorization: Basic wrong\r\n\r\n"),
    )
    .await?;
    assert!(wrong.starts_with(b"HTTP/1.1 407"));

    let forwarded = request(&proxy, &format!(
        "GET {origin}/legal HTTP/1.1\r\nProxy-Authorization: {auth}\r\nHost: wrong\r\nConnection: keep-alive, X-Remove\r\nX-Remove: removed\r\nKeep-Alive: timeout=5\r\nTE: trailers\r\nUpgrade: websocket\r\nRange: bytes=1-3\r\n\r\n"
    )).await?;
    assert!(forwarded.starts_with(b"HTTP/1.1 200"));
    assert!(forwarded.ends_with(b"generated legal media"));
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(proxy.as_str())?)
        .timeout(Duration::from_secs(15))
        .build()?;
    assert_eq!(
        client
            .head(format!("{origin}/legal"))
            .send()
            .await?
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{origin}/redirect"))
            .send()
            .await?
            .bytes()
            .await?
            .as_ref(),
        b"generated legal media"
    );
    let blocked_redirect = client
        .get(format!("{origin}/private-redirect"))
        .send()
        .await?;
    // Reqwest removes proxy auth on a cross-origin redirect. The redirected
    // local request must still be rejected before opening an outbound socket.
    assert!(matches!(blocked_redirect.status().as_u16(), 403 | 407));

    for (method, target, extra, status) in [
        ("POST", format!("{origin}/legal"), "", 405),
        (
            "GET",
            format!("{origin}/legal"),
            "Content-Length: 1\r\n",
            400,
        ),
        (
            "GET",
            format!("{origin}/legal"),
            "Content-Length: 0\r\nContent-Length: 0\r\n",
            400,
        ),
        (
            "GET",
            format!("{origin}/legal"),
            "Transfer-Encoding: chunked\r\n",
            400,
        ),
        (
            "GET",
            format!("{origin}/legal"),
            &format!("Proxy-Authorization: {auth}\r\n"),
            407,
        ),
        ("GET", "file:///legal".into(), "", 403),
        ("GET", "http://127.0.0.1/".into(), "", 403),
        ("GET", "http://localhost/".into(), "", 403),
        ("GET", "http://mixed.example/".into(), "", 403),
        ("GET", "http://user:secret@fixture.example/".into(), "", 403),
        ("GET", "http://[::ffff:127.0.0.1]/".into(), "", 403),
        ("GET", "http://10.0.0.1/".into(), "", 403),
        ("CONNECT", "127.0.0.1:443".into(), "", 403),
        ("CONNECT", "localhost:443".into(), "", 403),
        ("CONNECT", "user@fixture.example:443".into(), "", 400),
        ("CONNECT", "fixture.example:0".into(), "", 403),
    ] {
        let response = request(
            &proxy,
            &format!("{method} {target} HTTP/1.1\r\nProxy-Authorization: {auth}\r\n{extra}\r\n"),
        )
        .await?;
        assert!(
            response.starts_with(format!("HTTP/1.1 {status}").as_bytes()),
            "request fault returned wrong status"
        );
    }
    let oversized = request(
        &proxy,
        &format!(
            "GET {origin}/legal HTTP/1.1\r\nProxy-Authorization: {auth}\r\nX-Large: {}\r\n\r\n",
            "x".repeat(17 * 1024)
        ),
    )
    .await?;
    assert!(oversized.starts_with(b"HTTP/1.1 400"));
    let too_many_headers = request(
        &proxy,
        &format!(
            "GET {origin}/legal HTTP/1.1\r\nProxy-Authorization: {auth}\r\n{}\r\n",
            "X-Header: value\r\n".repeat(64)
        ),
    )
    .await?;
    assert!(too_many_headers.starts_with(b"HTTP/1.1 400"));
    let pipeline = request(&proxy, &format!("GET {origin}/legal HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\nGET /second HTTP/1.1\r\n\r\n")).await?;
    assert!(pipeline.starts_with(b"HTTP/1.1 400"));
    let bad_response = client.get(format!("{origin}/large-head")).send().await?;
    assert_eq!(bad_response.status(), 502);

    // The actual production constructor also rejects local DNS and IP. Count
    // fixture TCP accepts, rather than mistaking a TLS error for no connection.
    let production = qbutt_content::MediaGateway::start().await?;
    let production_client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(production.proxy_url().as_str())?)
        .timeout(Duration::from_secs(5))
        .build()?;
    let before = plain.accepted.load(Ordering::SeqCst);
    for host in ["127.0.0.1", "localhost"] {
        let response = production_client
            .get(format!(
                "http://{host}:{}/legal",
                plain.origin.port().unwrap()
            ))
            .send()
            .await?;
        assert_eq!(response.status(), 403);
    }
    assert_eq!(before, plain.accepted.load(Ordering::SeqCst));
    production.close().await;

    let untrusted = client.get(tls.origin.join("legal")?).send().await;
    assert!(
        untrusted.is_err(),
        "TLS accepted an untrusted fixture certificate"
    );
    let trusted = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(proxy.as_str())?)
        .add_root_certificate(reqwest::Certificate::from_pem(tls.cert_pem.as_bytes())?)
        .timeout(Duration::from_secs(5))
        .build()?;
    assert_eq!(
        trusted
            .get(tls.origin.join("legal")?)
            .send()
            .await?
            .bytes()
            .await?
            .as_ref(),
        b"generated legal media"
    );

    let mut tunnel = TcpStream::connect(("127.0.0.1", proxy.port().unwrap())).await?;
    tunnel.write_all(format!("CONNECT fixture.example:{} HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\nearly-payload", plain.origin.port().unwrap()).as_bytes()).await?;
    let mut established = vec![0; b"HTTP/1.1 200 Connection Established\r\n\r\n".len()];
    tunnel.read_exact(&mut established).await?;
    assert!(established.starts_with(b"HTTP/1.1 200"));
    // Early CONNECT bytes were delivered unchanged to the fixture. The server
    // is now waiting for the rest of that deliberately incomplete request.
    if let Some(mpv) = mpv {
        let fixture_dir = args.first().unwrap();
        run_mpv(&mpv, &proxy, &plain.origin, None, true).await?;
        run_mpv(&mpv, &proxy, &tls.origin, None, false).await?;
        let certificate = Path::new(fixture_dir).join("fixture-ca.pem");
        std::fs::write(&certificate, &tls.cert_pem)?;
        run_mpv(&mpv, &proxy, &tls.origin, Some(&certificate), true).await?;
        std::fs::remove_file(certificate)?;
    }

    let mut idle = TcpStream::connect(("127.0.0.1", proxy.port().unwrap())).await?;
    let mut pending_headers = TcpStream::connect(("127.0.0.1", proxy.port().unwrap())).await?;
    pending_headers.write_all(b"GET ").await?;

    gateway.retire();
    gateway.retire();
    gateway.close().await;
    for socket in [&mut tunnel, &mut idle, &mut pending_headers] {
        let mut byte = [0];
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte)).await?,
            Ok(0) | Err(_)
        ));
    }
    assert!(
        TcpStream::connect(("127.0.0.1", proxy.port().unwrap()))
            .await
            .is_err()
    );
    let dropped = MediaGateway::start().await?;
    let port = dropped.proxy_url().port().unwrap();
    let mut open = TcpStream::connect(("127.0.0.1", port)).await?;
    drop(dropped);
    let mut byte = [0];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), open.read(&mut byte)).await?,
        Ok(0) | Err(_)
    ));
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    assert_eq!(plain.leaked_headers.load(Ordering::SeqCst), 0);
    assert_eq!(tls.leaked_headers.load(Ordering::SeqCst), 0);
    tokio::try_join!(exercise_bounds(&plain.origin), exercise_long_stream(&tls))?;
    plain.stop().await;
    tls.stop().await;
    println!(
        "PASS media-gateway auth=required credentials=not_forwarded methods=bounded framing=checked public_ip_dns_mixed=guarded redirects=guarded tls=validated http_connect=ok headers=bounded sockets=32_pairs timeout=checked retirement=all_sockets_closed drop=closed"
    );
    Ok(())
}

fn authorization(proxy: &Url) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("qbutt:{}", proxy.password().unwrap()))
    )
}

async fn exercise_long_stream(fixture: &Fixture) -> AcceptanceResult<()> {
    let gateway = MediaGateway::start().await?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(gateway.proxy_url().as_str())?)
        .add_root_certificate(reqwest::Certificate::from_pem(fixture.cert_pem.as_bytes())?)
        .timeout(Duration::from_secs(75))
        .build()?;
    let started = std::time::Instant::now();
    let bytes = client
        .get(fixture.origin.join("long-stream")?)
        .send()
        .await?
        .bytes()
        .await?;
    assert_eq!(bytes.as_ref(), &[b'x'; 67]);
    assert!(started.elapsed() >= Duration::from_secs(66));
    gateway.close().await;
    println!(
        "PASS media-gateway-long-stream https_single_get=67s unidirectional_payload=67_verified_bytes active_tunnel=preserved"
    );
    Ok(())
}

async fn exercise_bounds(origin: &Url) -> AcceptanceResult<()> {
    let gateway = MediaGateway::start().await?;
    let proxy = gateway.proxy_url().clone();
    let auth = authorization(&proxy);
    let mut tunnels = Vec::new();
    for _ in 0..32 {
        let mut socket = TcpStream::connect(("127.0.0.1", proxy.port().unwrap())).await?;
        socket
            .write_all(
                format!(
                    "CONNECT fixture.example:{} HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n",
                    origin.port().unwrap()
                )
                .as_bytes(),
            )
            .await?;
        let mut established = vec![0; b"HTTP/1.1 200 Connection Established\r\n\r\n".len()];
        tokio::time::timeout(Duration::from_secs(2), socket.read_exact(&mut established)).await??;
        assert!(established.starts_with(b"HTTP/1.1 200"));
        tunnels.push(socket);
    }
    let mut excess = TcpStream::connect(("127.0.0.1", proxy.port().unwrap())).await?;
    let mut byte = [0];
    assert!(
        matches!(
            tokio::time::timeout(Duration::from_secs(2), excess.read(&mut byte)).await?,
            Ok(0) | Err(_)
        ),
        "connection beyond 32 pairs was accepted"
    );
    gateway.close().await;
    for socket in &mut tunnels {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte)).await?,
            Ok(0) | Err(_)
        ));
    }

    let gateway = MediaGateway::start().await?;
    let proxy = gateway.proxy_url().clone();
    let auth = authorization(&proxy);
    let started = std::time::Instant::now();
    let stalled = format!(
        "GET {} HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n",
        origin.join("stall")?
    );
    let (header_timeout, response_timeout) =
        tokio::try_join!(request(&proxy, "GET "), request(&proxy, &stalled))?;
    assert!(header_timeout.starts_with(b"HTTP/1.1 400"));
    assert!(response_timeout.starts_with(b"HTTP/1.1 502"));
    assert!((Duration::from_millis(9800)..Duration::from_secs(14)).contains(&started.elapsed()));
    println!("PASS media-gateway-timeout request_head=10s upstream_head=10s sockets=32_pairs");

    let mut idle = TcpStream::connect(("127.0.0.1", proxy.port().unwrap())).await?;
    idle.write_all(
        format!(
            "CONNECT fixture.example:{} HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n",
            origin.port().unwrap()
        )
        .as_bytes(),
    )
    .await?;
    let mut established = vec![0; b"HTTP/1.1 200 Connection Established\r\n\r\n".len()];
    idle.read_exact(&mut established).await?;
    let started = std::time::Instant::now();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(63), idle.read(&mut byte)).await?,
        Ok(0) | Err(_)
    ));
    assert!((Duration::from_secs(59)..Duration::from_secs(63)).contains(&started.elapsed()));
    gateway.close().await;
    println!("PASS media-gateway-timeout tunnel_idle=60s physical_socket=closed");
    Ok(())
}

async fn request(proxy: &Url, head: &str) -> AcceptanceResult<Vec<u8>> {
    let mut socket = TcpStream::connect(("127.0.0.1", proxy.port().unwrap())).await?;
    socket.write_all(head.as_bytes()).await?;
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), socket.read_to_end(&mut bytes)).await??;
    Ok(bytes)
}

async fn run_mpv(
    mpv: &Path,
    proxy: &Url,
    origin: &Url,
    ca: Option<&Path>,
    trusted: bool,
) -> AcceptanceResult<()> {
    let mut command = std::process::Command::new(mpv);
    command.args([
        "--no-config",
        "--vo=null",
        "--ao=null",
        "--audio-display=no",
        "--tls-verify=yes",
        "--terminal=yes",
        "--msg-level=all=warn,cplayer=info,statusline=status",
        "--term-status-msg=QBUTT_MEDIA_TIME_${time-pos}",
    ]);
    command.arg(format!("--http-proxy={proxy}"));
    if let Some(ca) = ca {
        command.arg(format!("--tls-ca-file={}", ca.display()));
    }
    command.arg(origin.join("master.m3u8")?.as_str());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = tokio::task::spawn_blocking(move || {
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while child.try_wait()?.is_none() {
            if std::time::Instant::now() > deadline {
                child.kill()?;
                child.wait()?;
                return Err(io::ErrorKind::TimedOut.into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child.wait_with_output()
    })
    .await??;
    if !trusted {
        assert!(
            !output.status.success(),
            "mpv accepted an untrusted TLS certificate"
        );
        println!("PASS media-gateway-mpv untrusted_tls=rejected");
        return Ok(());
    }
    assert!(output.status.success(), "mpv fixture playback failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("VO: [null]"),
        "mpv did not begin video decoding"
    );
    assert!(
        stdout.contains("QBUTT_MEDIA_TIME_00:00:03"),
        "mpv did not reach 3 seconds"
    );
    println!(
        "PASS media-gateway-mpv {} hls_children=decoded time=3s tls_verify=yes",
        origin.scheme()
    );
    Ok(())
}

struct Fixture {
    origin: Url,
    cert_pem: String,
    accepted: Arc<AtomicUsize>,
    leaked_headers: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start(tls: bool, media_dir: Option<PathBuf>) -> AcceptanceResult<Self> {
        let certificate = rcgen::generate_simple_self_signed(vec![
            "fixture.example".into(),
            "child.example".into(),
        ])?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(certificate.cert.der());
        let cert_pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            encoded
                .as_bytes()
                .chunks(64)
                .map(|line| std::str::from_utf8(line).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        );
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.cert.der().clone()],
                PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
            )?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let origin = Url::parse(&format!(
            "{}://fixture.example:{port}",
            if tls { "https" } else { "http" }
        ))?;
        let accepted = Arc::new(AtomicUsize::new(0));
        let leaked_headers = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let hits = accepted.clone();
        let leaks = leaked_headers.clone();
        let redirect_origin = origin.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    _ = connections.join_next(), if !connections.is_empty() => {}
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break };
                        hits.fetch_add(1, Ordering::SeqCst);
                        let acceptor = acceptor.clone();
                        let leaks = leaks.clone();
                        let origin = redirect_origin.clone();
                        let media_dir = media_dir.clone();
                        connections.spawn(async move {
                            if tls {
                                if let Ok(socket) = acceptor.accept(socket).await {
                                    let _ = respond(socket, origin, media_dir, leaks).await;
                                }
                            } else {
                                let _ = respond(socket, origin, media_dir, leaks).await;
                            }
                        });
                    }
                }
            }
            connections.shutdown().await;
        });
        Ok(Self {
            origin,
            cert_pem,
            accepted,
            leaked_headers,
            stop,
            task,
        })
    }

    async fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }
}

async fn respond<S: AsyncRead + AsyncWrite + Unpin>(
    mut socket: S,
    origin: Url,
    media_dir: Option<PathBuf>,
    leaks: Arc<AtomicUsize>,
) -> io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
        if head.len() > 64 * 1024 {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
    let head = String::from_utf8_lossy(&head);
    if head.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        [
            "proxy-authorization:",
            "proxy-connection:",
            "keep-alive:",
            "x-remove:",
            "te:",
            "upgrade:",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    }) {
        leaks.fetch_add(1, Ordering::SeqCst);
    }
    let path = head.split_whitespace().nth(1).unwrap_or("");
    if path == "/stall" {
        return socket.read(&mut byte).await.map(|_| ());
    }
    if path == "/long-stream" {
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 67\r\nConnection: close\r\n\r\n")
            .await?;
        for _ in 0..67 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            socket.write_all(b"x").await?;
            socket.flush().await?;
        }
        return socket.shutdown().await;
    }
    if path == "/large-head" {
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nX-Large: {}\r\n\r\n",
                    "x".repeat(17 * 1024)
                )
                .as_bytes(),
            )
            .await?;
        return socket.shutdown().await;
    }
    if path == "/redirect" || path == "/private-redirect" {
        let target = if path == "/private-redirect" {
            "http://127.0.0.1/forbidden".into()
        } else {
            origin.join("legal").unwrap().to_string()
        };
        socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await?;
        return socket.shutdown().await;
    }
    let body = if path == "/master.m3u8" {
        format!(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100000\n{}://child.example:{}/playlist.m3u8\n",
            origin.scheme(),
            origin.port().unwrap()
        )
        .into_bytes()
    } else if let Some(directory) = &media_dir {
        let name = path.trim_start_matches('/');
        if name == "playlist.m3u8"
            || (name.starts_with("segment")
                && name.ends_with(".ts")
                && !name.contains('/')
                && !name.contains('\\'))
        {
            std::fs::read(directory.join(name))?
        } else {
            b"generated legal media".to_vec()
        }
    } else {
        b"generated legal media".to_vec()
    };
    let mime = if path.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else if path.ends_with(".ts") {
        "video/mp2t"
    } else {
        "application/octet-stream"
    };
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await?;
    if !head.starts_with("HEAD ") {
        socket.write_all(&body).await?;
    }
    socket.shutdown().await
}
