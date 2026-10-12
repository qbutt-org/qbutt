//! Authenticated public-network transport for a direct playback session.
//! Anonymous HTTPS uses opaque CONNECT; protected byte resources keep their
//! credentials here and use the existing guarded Rust HTTPS client.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use url::Url;

use crate::http::{resolve_public_url, validate_media_url};

#[path = "media/resource.rs"]
mod resource;
pub(crate) use resource::Resources as MediaResources;
pub use resource::{PreparedPlayback, PreparedSubtitle};

const MAX_CONNECTIONS: usize = 32;
const MAX_HEADERS: usize = 64;
const MAX_HEAD_BYTES: usize = 16 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum MediaGatewayError {
    #[error("The protected media connection could not be started")]
    Bind(#[source] io::Error),
    #[error("The protected media connection could not create its credentials")]
    Credentials(#[source] getrandom::Error),
    #[error("The browser returned an invalid media descriptor")]
    InvalidDescriptor,
    #[error("This video requires a manifest format that is not supported yet")]
    UnsupportedManifest,
    #[error("The protected media source could not be reached")]
    Source,
}

/// Keep this owner for the entire playback lifetime, independent of selection
/// or search generations. Dropping or retiring it closes all of its sockets.
/// It uses the caller's Tokio runtime and creates no thread or process.
pub struct MediaGateway {
    proxy_url: Url,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl MediaGateway {
    pub async fn start() -> Result<Self, MediaGatewayError> {
        let (listener, proxy_url, authorization) = bind().await?;
        Ok(Self::serve(listener, proxy_url, authorization, None))
    }

    /// Keep this gateway until playback finishes. The returned descriptor
    /// contains local capabilities only; do not supply a proxy to the player.
    pub async fn prepare(
        media: crate::PlaybackMedia,
    ) -> Result<(Self, PreparedPlayback), MediaGatewayError> {
        let (listener, proxy_url, authorization) = bind().await?;
        let (resources, prepared) = resource::Resources::prepare(media, &proxy_url).await?;
        let gateway = Self::serve(
            listener,
            proxy_url,
            authorization,
            Some(Arc::new(resources)),
        );
        Ok((gateway, prepared))
    }

    fn serve(
        listener: TcpListener,
        proxy_url: Url,
        authorization: String,
        resources: Option<Arc<resource::Resources>>,
    ) -> Self {
        let (shutdown, receiver) = watch::channel(false);
        let task = tokio::spawn(serve(listener, authorization, resources, receiver));
        Self {
            proxy_url,
            shutdown,
            task,
        }
    }

    /// This URL contains a session secret; never persist or log it.
    pub fn proxy_url(&self) -> &Url {
        &self.proxy_url
    }

    pub fn retire(&self) {
        self.shutdown.send_replace(true);
    }

    /// Retire and wait until the listener and owned connection tasks are dropped.
    pub async fn close(mut self) {
        self.retire();
        let _ = (&mut self.task).await;
    }
}

async fn bind() -> Result<(TcpListener, Url, String), MediaGatewayError> {
    let mut secret = [0; 24];
    getrandom::fill(&mut secret).map_err(MediaGatewayError::Credentials)?;
    let password: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    let authorization = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("qbutt:{password}"))
    );
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(MediaGatewayError::Bind)?;
    let port = listener
        .local_addr()
        .map_err(MediaGatewayError::Bind)?
        .port();
    let proxy_url = Url::parse(&format!("http://qbutt:{password}@127.0.0.1:{port}"))
        .expect("fixed loopback proxy URL");
    Ok((listener, proxy_url, authorization))
}

impl Drop for MediaGateway {
    fn drop(&mut self) {
        self.retire();
    }
}

async fn serve(
    listener: TcpListener,
    authorization: String,
    resources: Option<Arc<resource::Resources>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = connections.join_next(), if !connections.is_empty() => {}
            accepted = listener.accept() => {
                let Ok((socket, _)) = accepted else { break };
                if *shutdown.borrow() { break; }
                if connections.len() < MAX_CONNECTIONS {
                    let authorization = authorization.clone();
                    let resources = resources.clone();
                    connections.spawn(async move {
                        let _ = handle(socket, &authorization, resources.as_deref()).await;
                    });
                }
            }
        }
    }
    drop(listener);
    connections.shutdown().await;
}

struct Request {
    target: Url,
    tunnel: bool,
    upstream_head: Vec<u8>,
}

async fn handle(
    mut client: TcpStream,
    authorization: &str,
    resources: Option<&resource::Resources>,
) -> io::Result<()> {
    let (head, end) = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut client)).await {
        Ok(Ok(head)) => head,
        _ => return reject(&mut client, 400).await,
    };
    if let Some(resources) = resources {
        return resources.handle(client, &head[..end], &head[end..]).await;
    }
    let request = match parse_request(&head[..end], authorization, &head[end..]) {
        Ok(request) => request,
        Err(status) => return reject(&mut client, status).await,
    };
    let mut upstream = match tokio::time::timeout(CONNECT_TIMEOUT, connect(&request.target)).await {
        Ok(Ok(socket)) => socket,
        Ok(Err(error)) if error.kind() == io::ErrorKind::PermissionDenied => {
            return reject(&mut client, 403).await;
        }
        _ => return reject(&mut client, 502).await,
    };

    let (activity, mut observed) = watch::channel(());
    let transfer = async {
        if request.tunnel {
            tokio::time::timeout(
                HEAD_TIMEOUT,
                client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n"),
            )
            .await??;
            tokio::time::timeout(HEAD_TIMEOUT, upstream.write_all(&head[end..])).await??;
            let (mut client_read, mut client_write) = client.split();
            let (mut upstream_read, mut upstream_write) = upstream.split();
            tokio::try_join!(
                relay(&mut client_read, &mut upstream_write, &activity),
                relay(&mut upstream_read, &mut client_write, &activity)
            )?;
        } else {
            tokio::time::timeout(HEAD_TIMEOUT, upstream.write_all(&request.upstream_head))
                .await??;
            let (response, end) =
                match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut upstream)).await {
                    Ok(Ok(response)) => response,
                    _ => return reject(&mut client, 502).await,
                };
            let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
            if !matches!(
                httparse::Response::new(&mut headers).parse(&response[..end]),
                Ok(httparse::Status::Complete(_))
            ) {
                return reject(&mut client, 502).await;
            }
            // Preserve HTTP framing (including chunked payload) and redirects. The
            // player's next request goes through the same guarded connector.
            tokio::time::timeout(HEAD_TIMEOUT, client.write_all(&response)).await??;
            relay(&mut upstream, &mut client, &activity).await?;
        }
        Ok(())
    };
    tokio::select! {
        result = transfer => result,
        _ = async {
            // Either direction keeps a TLS tunnel alive. A response can stream
            // for hours after the client has sent its single GET request.
            while matches!(
                tokio::time::timeout(IDLE_TIMEOUT, observed.changed()).await,
                Ok(Ok(()))
            ) {}
        } => Err(io::ErrorKind::TimedOut.into()),
    }
}

fn parse_request(head: &[u8], authorization: &str, remainder: &[u8]) -> Result<Request, u16> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut parsed = httparse::Request::new(&mut headers);
    if !matches!(parsed.parse(head), Ok(httparse::Status::Complete(_))) {
        return Err(400);
    }
    let mut supplied = parsed
        .headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case("proxy-authorization"));
    let authenticated = supplied
        .next()
        .is_some_and(|header| header.value.ct_eq(authorization.as_bytes()).into())
        && supplied.next().is_none();
    if !authenticated {
        return Err(407);
    }
    let method = parsed.method.ok_or(400u16)?;
    let target = parsed.path.ok_or(400u16)?;
    let tunnel = method == "CONNECT";
    if !matches!(method, "GET" | "HEAD" | "CONNECT") {
        return Err(405);
    }
    // Media acquisition has no request body. Reject every ambiguous framing
    // rather than forwarding it to a server with a different parser.
    let mut lengths = 0;
    for header in parsed.headers.iter() {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(400);
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            lengths += 1;
            if lengths > 1 || header.value != b"0" {
                return Err(400);
            }
        }
    }
    if !tunnel && !remainder.is_empty() {
        return Err(400);
    }
    let target = if tunnel {
        let (_, port) = target.rsplit_once(':').ok_or(400u16)?;
        if port.is_empty()
            || port.len() > 5
            || !port.bytes().all(|byte| byte.is_ascii_digit())
            || !target.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
            })
        {
            return Err(400);
        }
        Url::parse(&format!("https://{target}/")).map_err(|_| 400u16)?
    } else {
        Url::parse(target).map_err(|_| 400u16)?
    };
    validate_media_url(&target).map_err(|_| 403u16)?;
    if !tunnel && target.scheme() != "http" {
        return Err(403);
    }

    let mut upstream_head = Vec::new();
    if !tunnel {
        let connection_headers: Vec<&str> = parsed
            .headers
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case("connection"))
            .filter_map(|header| std::str::from_utf8(header.value).ok())
            .flat_map(|value| value.split(',').map(str::trim))
            .collect();
        let host = &target[url::Position::BeforeHost..url::Position::AfterPort];
        upstream_head.extend_from_slice(
            format!(
                "{method} {} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n",
                &target[url::Position::BeforePath..url::Position::AfterQuery]
            )
            .as_bytes(),
        );
        for header in parsed.headers.iter() {
            if hop_header(header.name)
                || header.name.eq_ignore_ascii_case("host")
                || header.name.eq_ignore_ascii_case("content-length")
                || connection_headers
                    .iter()
                    .any(|name| header.name.eq_ignore_ascii_case(name))
            {
                continue;
            }
            upstream_head.extend_from_slice(header.name.as_bytes());
            upstream_head.extend_from_slice(b": ");
            upstream_head.extend_from_slice(header.value);
            upstream_head.extend_from_slice(b"\r\n");
        }
        upstream_head.extend_from_slice(b"\r\n");
    }
    Ok(Request {
        target,
        tunnel,
        upstream_head,
    })
}

fn hop_header(name: &str) -> bool {
    [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ]
    .iter()
    .any(|excluded| name.eq_ignore_ascii_case(excluded))
}

async fn connect(target: &Url) -> io::Result<TcpStream> {
    let addresses = resolve_public_url(target).await?;
    let mut last_error = io::Error::from(io::ErrorKind::NotConnected);
    for address in addresses {
        match TcpStream::connect(address).await {
            Ok(socket) => return Ok(socket),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

async fn read_head(socket: &mut TcpStream) -> io::Result<(Vec<u8>, usize)> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 2048];
    loop {
        let count = socket.read(&mut buffer).await?;
        if count == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            if end + 4 > MAX_HEAD_BYTES {
                return Err(io::ErrorKind::InvalidData.into());
            }
            return Ok((bytes, end + 4));
        }
        if bytes.len() >= MAX_HEAD_BYTES {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
}

async fn relay<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    source: &mut R,
    destination: &mut W,
    activity: &watch::Sender<()>,
) -> io::Result<()> {
    let mut buffer = [0; 32 * 1024];
    loop {
        let count = source.read(&mut buffer).await?;
        if count == 0 {
            return destination.shutdown().await;
        }
        activity.send_replace(());
        tokio::time::timeout(IDLE_TIMEOUT, destination.write_all(&buffer[..count])).await??;
    }
}

async fn reject(socket: &mut TcpStream, status: u16) -> io::Result<()> {
    let (reason, extra) = match status {
        400 => ("Bad Request", ""),
        403 => ("Forbidden", ""),
        405 => ("Method Not Allowed", "Allow: GET, HEAD, CONNECT\r\n"),
        407 => (
            "Proxy Authentication Required",
            "Proxy-Authenticate: Basic realm=\"qbutt\"\r\n",
        ),
        _ => ("Bad Gateway", ""),
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n{extra}Connection: close\r\nContent-Length: 0\r\n\r\n"
    );
    tokio::time::timeout(HEAD_TIMEOUT, socket.write_all(response.as_bytes())).await??;
    socket.shutdown().await
}
