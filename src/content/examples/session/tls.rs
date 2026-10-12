use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use rcgen::generate_simple_self_signed;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use url::Url;

use super::FixtureResult;
use crate::http::SourceHttp;

pub(super) struct Fixture {
    pub origin: Url,
    pub accepted: Arc<AtomicUsize>,
    pub canceled: Arc<AtomicBool>,
    address: SocketAddr,
    certificate: reqwest::Certificate,
    stop: oneshot::Sender<()>,
    task: JoinHandle<io::Result<()>>,
}

impl Fixture {
    pub async fn start() -> FixtureResult<Self> {
        let generated = generate_simple_self_signed(vec!["fixture.example".into()])?;
        let certificate = reqwest::Certificate::from_der(generated.cert.der())?;
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![generated.cert.der().clone()],
                PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der()).into(),
            )?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let origin = format!("https://fixture.example:{}/", address.port()).parse()?;
        let accepted = Arc::new(AtomicUsize::new(0));
        let canceled = Arc::new(AtomicBool::new(false));
        let (stop, stopping) = oneshot::channel();
        let task = tokio::spawn(serve(
            listener,
            TlsAcceptor::from(Arc::new(config)),
            stopping,
            accepted.clone(),
            canceled.clone(),
        ));
        Ok(Self {
            origin,
            accepted,
            canceled,
            address,
            certificate,
            stop,
            task,
        })
    }

    pub fn http(&self, trusted: bool) -> FixtureResult<SourceHttp> {
        let mut builder = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve("fixture.example", self.address);
        if trusted {
            builder = builder.add_root_certificate(self.certificate.clone());
        }
        Ok(crate::http::fixture(builder.build()?, self.origin.clone()))
    }

    pub async fn shutdown(self) -> FixtureResult {
        let _ = self.stop.send(());
        self.task.await??;
        Ok(())
    }
}

async fn serve(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    mut stop: oneshot::Receiver<()>,
    accepted: Arc<AtomicUsize>,
    canceled: Arc<AtomicBool>,
) -> io::Result<()> {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stop => break,
            result = listener.accept() => {
                let (stream, _) = result?;
                accepted.fetch_add(1, Ordering::SeqCst);
                let acceptor = acceptor.clone();
                let canceled = canceled.clone();
                tasks.spawn(async move { connection(stream, acceptor, canceled).await });
            }
            Some(result) = tasks.join_next(), if !tasks.is_empty() => { result??; }
        }
    }
    tasks.abort_all();
    while let Some(result) = tasks.join_next().await {
        match result {
            Err(error) if error.is_cancelled() => {}
            result => result??,
        }
    }
    Ok(())
}

async fn connection(
    stream: TcpStream,
    acceptor: TlsAcceptor,
    canceled: Arc<AtomicBool>,
) -> io::Result<()> {
    // An intentionally untrusted client rejects this generated certificate.
    let Ok(mut stream) = acceptor.accept(stream).await else {
        return Ok(());
    };
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        if stream.read(&mut byte).await? == 0 {
            return Ok(());
        }
        bytes.push(byte[0]);
        if bytes.len() > 64 * 1024 {
            return Err(io::Error::other("Fixture request exceeds limit"));
        }
    }
    let headers = String::from_utf8(bytes).map_err(io::Error::other)?;
    let mut lines = headers.lines();
    let request = lines
        .next()
        .ok_or_else(|| io::Error::other("Missing request"))?;
    let mut request_parts = request.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| io::Error::other("Missing fixture method"))?;
    let target = request_parts
        .next()
        .ok_or_else(|| io::Error::other("Missing fixture target"))?;
    assert_eq!(request_parts.next(), Some("HTTP/1.1"));
    let headers: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let content_length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .map(|(_, value)| value.parse::<usize>())
        .transpose()
        .map_err(io::Error::other)?
        .unwrap_or(0);
    assert!(content_length <= 16 * 1024);
    let mut body = vec![0; content_length];
    stream.read_exact(&mut body).await?;
    let cookie_header = headers
        .iter()
        .find(|(name, _)| name == "cookie")
        .map(|(_, value)| value.as_str())
        .unwrap_or("");
    let cookies: Vec<_> = cookie_header
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();
    match target {
        "/metadata/get" | "/metadata/post" => {
            if target.ends_with("post") {
                assert_eq!(method, "POST");
                assert_eq!(body, b"id=fixture&op=download1");
            } else {
                assert_eq!(method, "GET");
                assert!(body.is_empty());
            }
            assert!(!headers.iter().any(|(name, _)| name == "x-requested-with"));
            assert!(cookies.contains(&"form=rotated"));
            response(
                &mut stream,
                200,
                "Content-Type: application/x-bittorrent\r\n",
                super::metadata::TORRENT,
            )
            .await?;
        }
        "/metadata/gate" => {
            response(
                &mut stream,
                403,
                "Content-Type: text/html\r\n",
                b"<html>Generated verification gate</html>",
            )
            .await?
        }
        "/metadata/cross" => {
            response(
                &mut stream,
                302,
                "Location: https://other.example/\r\n",
                b"",
            )
            .await?
        }
        "/metadata/large" => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                crate::TorrentMetadata::MAX_BYTES
            );
            stream.write_all(head.as_bytes()).await?;
            let body = vec![b'x'; crate::TorrentMetadata::MAX_BYTES];
            let _ = stream.write_all(&body).await;
        }
        "/metadata/oversize" => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                crate::TorrentMetadata::MAX_BYTES + 1
            );
            stream.write_all(head.as_bytes()).await?;
        }
        target if target.starts_with("/metadata/form/") => {
            let code: u16 = target
                .trim_start_matches("/metadata/form/")
                .parse()
                .map_err(io::Error::other)?;
            assert!(matches!(code, 301 | 302 | 303 | 307 | 308));
            assert_eq!(method, "POST");
            assert_eq!(body, b"id=fixture&op=download1");
            let target = if matches!(code, 307 | 308) {
                "post"
            } else {
                "get"
            };
            let headers = format!(
                "Location: /metadata/{target}\r\nSet-Cookie: form=rotated; Path=/metadata; Secure\r\n"
            );
            response(&mut stream, code, &headers, b"").await?;
        }
        "/forum/start" => {
            assert_eq!(
                headers
                    .iter()
                    .find(|(name, _)| name == "user-agent")
                    .map(|(_, value)| value.as_str()),
                Some("fixture edge original")
            );
            assert!(
                cookies.contains(&"sid=old")
                    && cookies.contains(&"wide=root")
                    && cookies.contains(&"gone=remove")
            );
            assert!(!cookies.contains(&"expired=never"));
            response(&mut stream, 302, "Location: /forum/final\r\nSet-Cookie: sid=new; Domain=fixture.example; Secure\r\nSet-Cookie: gone=; Max-Age=0; Path=/\r\nSet-Cookie: off=hidden; Path=/other\r\nSet-Cookie: evil=never; Domain=other.example; Path=/\r\nSet-Cookie: temp=brief; Max-Age=1; Path=/forum\r\n", b"").await?;
        }
        "/forum/final" => {
            assert_eq!(
                cookies
                    .iter()
                    .filter(|value| value.starts_with("sid="))
                    .copied()
                    .collect::<Vec<_>>(),
                vec!["sid=new"]
            );
            assert!(!cookies.iter().any(|value| value.starts_with("gone=")
                || value.starts_with("off=")
                || value.starts_with("evil=")));
            assert!(cookies.contains(&"temp=brief"));
            let (body, _, error) = encoding_rs::WINDOWS_1251.encode("Проверка источника");
            assert!(!error);
            response(&mut stream, 403, "Content-Type: text/html; charset=windows-1251\r\nSet-Cookie: at_final=yes; Path=/forum; Secure\r\nSet-Cookie: quoted=\"valid\"; Path=/forum; Secure\r\n", &body).await?;
        }
        "/forum/after" => {
            assert!(cookies.contains(&"at_final=yes") && cookies.contains(&"sid=new"));
            assert!(cookies.contains(&"quoted=\"valid\""));
            assert!(!cookies.contains(&"temp=brief"));
            response(
                &mut stream,
                200,
                "Set-Cookie: sid=; Domain=fixture.example; Max-Age=0\r\n",
                b"after",
            )
            .await?;
        }
        "/forum/deleted" => {
            assert!(!cookies.iter().any(|value| value.starts_with("sid=")));
            response(&mut stream, 200, "", b"deleted").await?;
        }
        "/outside" => {
            assert_eq!(cookies, vec!["wide=root"]);
            response(
                &mut stream,
                200,
                "Content-Type: application/octet-stream\r\n",
                b"outside",
            )
            .await?;
        }
        "/cross" => {
            response(
                &mut stream,
                302,
                "Location: https://other.example/\r\n",
                b"",
            )
            .await?
        }
        "/scheme" => {
            response(
                &mut stream,
                302,
                "Location: http://fixture.example/\r\n",
                b"",
            )
            .await?
        }
        "/loop" => response(&mut stream, 302, "Location: /loop\r\n", b"").await?,
        "/large-length" => {
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n",
                )
                .await?
        }
        "/large-chunks" => {
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n200001\r\n").await?;
            let body = vec![b'x'; 2 * 1024 * 1024 + 1];
            // The bounded client may close as soon as the limit is crossed.
            let _ = stream.write_all(&body).await;
            let _ = stream.write_all(b"\r\n0\r\n\r\n").await;
        }
        "/empty" => response(&mut stream, 401, "", b"").await?,
        "/cookie-length" => {
            let header = format!("Set-Cookie: long={}; Path=/\r\n", "x".repeat(16 * 1024));
            response(&mut stream, 200, &header, b"cookie").await?;
        }
        "/cookie-header" => {
            let headers: String = (0..5)
                .map(|index| format!("Set-Cookie: h{index}={}; Path=/\r\n", "x".repeat(14 * 1024)))
                .collect();
            response(&mut stream, 200, &headers, b"cookies").await?;
        }
        "/cancel" => {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n")
                .await?;
            stream.flush().await?;
            let mut byte = [0];
            let read = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut byte)).await;
            assert!(matches!(read, Ok(Ok(0)) | Ok(Err(_))));
            canceled.store(true, Ordering::SeqCst);
        }
        "/slow" => tokio::time::sleep(Duration::from_secs(46)).await,
        target if target.starts_with("/cookie-count/") => {
            let batch: usize = target
                .trim_start_matches("/cookie-count/")
                .parse()
                .map_err(io::Error::other)?;
            let headers: String = (0..90)
                .map(|index| format!("Set-Cookie: n{}=v; Path=/\r\n", batch * 90 + index))
                .collect();
            response(&mut stream, 200, &headers, b"cookies").await?;
        }
        _ => return Err(io::Error::other("Unknown fixture route")),
    }
    let _ = stream.shutdown().await;
    Ok(())
}

async fn response(
    stream: &mut TlsStream<TcpStream>,
    status: u16,
    headers: &str,
    body: &[u8],
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await
}
