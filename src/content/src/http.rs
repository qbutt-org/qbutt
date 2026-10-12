use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE, HeaderMap, LOCATION, REFERER};
use serde::de::DeserializeOwned;
use url::{Host, Url};

use crate::{Error, Result};

#[path = "http/session.rs"]
mod session;
pub(crate) use session::SourceSession;

const MAX_RESPONSE_BYTES: usize = 24 * 1024 * 1024;
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
const MAX_FORM_BYTES: usize = 16 * 1024;
const MAX_REDIRECTS: usize = 5;

/// Streaming media has an idle read deadline rather than a total body deadline.
/// Redirects remain explicit so credentials can be scoped on every request.
pub(crate) fn media_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(PublicResolver))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(network_error)
}

#[derive(Clone, Copy)]
enum ResponseKind {
    Json,
    Html,
    Image,
    Playlist,
    Document,
    Metadata,
}

/// Raw, bounded source content. Providers classify login/challenge responses and
/// decode the source's advertised charset without losing HTTP failure bodies.
pub(crate) struct SourceDocument {
    pub status: u16,
    pub content_type: Option<String>,
    pub final_url: Url,
    pub body: Vec<u8>,
}

pub(crate) struct SourceHttp {
    client: reqwest::Client,
    origin: Url,
}

impl SourceHttp {
    pub(crate) fn new(origin: Url, headers: HeaderMap) -> Result<Self> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(PublicResolver))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("qbutt-content/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .build()
            .map_err(network_error)?;

        Ok(Self { client, origin })
    }

    pub(crate) async fn json<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let document = tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Json, &[], None, None, None),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        serde_json::from_slice(&document.body).map_err(Error::Json)
    }

    pub(crate) async fn form_json<T: DeserializeOwned>(&self, url: Url, form: &str) -> Result<T> {
        let document = tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Json, &[], Some(form), None, None),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        serde_json::from_slice(&document.body).map_err(Error::Json)
    }

    pub(crate) async fn session_json<T: DeserializeOwned>(
        &self,
        url: Url,
        session: &mut SourceSession,
    ) -> Result<T> {
        let document = tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Json, &[], None, Some(session), None),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        serde_json::from_slice(&document.body).map_err(Error::Json)
    }

    pub(crate) async fn session_image(
        &self,
        url: Url,
        allowed_hosts: &[&str],
        session: &mut SourceSession,
    ) -> Result<Vec<u8>> {
        tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(
                url,
                ResponseKind::Image,
                allowed_hosts,
                None,
                Some(session),
                None,
            ),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map(|document| document.body)
    }

    pub(crate) async fn image(&self, url: Url, allowed_hosts: &[&str]) -> Result<Vec<u8>> {
        tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Image, allowed_hosts, None, None, None),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map(|document| document.body)
    }

    pub(crate) async fn html(&self, url: Url) -> Result<String> {
        self.html_page(url).await.map(|(_, html)| html)
    }

    pub(crate) async fn html_page(&self, url: Url) -> Result<(Url, String)> {
        let document = tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Html, &[], None, None, None),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        let html = String::from_utf8(document.body)
            .map_err(|_| Error::InvalidResponse("HTML is not UTF-8"))?;
        Ok((document.final_url, html))
    }

    pub(crate) async fn playlist(&self, url: Url) -> Result<(Url, String)> {
        let document = tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Playlist, &[], None, None, None),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        let text = String::from_utf8(document.body)
            .map_err(|_| Error::InvalidResponse("playlist is not UTF-8"))?;
        Ok((document.final_url, text))
    }

    pub(crate) async fn session_document(
        &self,
        url: Url,
        session: &mut SourceSession,
    ) -> Result<SourceDocument> {
        tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Document, &[], None, Some(session), None),
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    pub(crate) async fn session_form_document(
        &self,
        url: Url,
        form: &str,
        session: &mut SourceSession,
    ) -> Result<SourceDocument> {
        tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(
                url,
                ResponseKind::Document,
                &[],
                Some(form),
                Some(session),
                None,
            ),
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// Raw acquisition bytes retain HTTP/HTML gates for the provider to classify.
    /// The native torrent handoff remains responsible for parsing the metadata.
    pub(crate) async fn session_metadata(
        &self,
        url: Url,
        session: &mut SourceSession,
        form: Option<&str>,
    ) -> Result<SourceDocument> {
        tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Metadata, &[], form, Some(session), None),
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    pub(crate) async fn session_referred_metadata(
        &self,
        url: Url,
        session: &mut SourceSession,
        referer: &Url,
    ) -> Result<SourceDocument> {
        tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(
                url,
                ResponseKind::Metadata,
                &[],
                None,
                Some(session),
                Some(referer),
            ),
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    async fn fetch(
        &self,
        mut url: Url,
        kind: ResponseKind,
        allowed_hosts: &[&str],
        mut form: Option<&str>,
        mut session: Option<&mut SourceSession>,
        referer: Option<&Url>,
    ) -> Result<SourceDocument> {
        if form.is_some_and(|form| form.len() > MAX_FORM_BYTES) {
            return Err(Error::ResponseTooLarge);
        }
        if session
            .as_ref()
            .is_some_and(|session| session.origin != url.origin())
        {
            return Err(Error::UnsafeAddress);
        }
        if let Some(referer) = referer {
            validate_https_url(referer)?;
            if referer.origin() != self.origin.origin() {
                return Err(Error::UnsafeAddress);
            }
        }
        let raw = matches!(kind, ResponseKind::Document | ResponseKind::Metadata);
        let same_origin = !matches!(kind, ResponseKind::Image | ResponseKind::Playlist);
        let max_bytes = match kind {
            ResponseKind::Metadata => crate::TorrentMetadata::MAX_BYTES,
            ResponseKind::Html | ResponseKind::Playlist | ResponseKind::Document => MAX_HTML_BYTES,
            ResponseKind::Json | ResponseKind::Image => MAX_RESPONSE_BYTES,
        };
        for redirects in 0..=MAX_REDIRECTS {
            validate_https_url(&url)?;
            // Catalog and form redirects stay on the fixed origin. Images and
            // playlists may use public CDNs; every connection uses checked DNS.
            if same_origin && url.origin() != self.origin.origin() {
                return Err(Error::UnsafeAddress);
            }
            if !allowed_hosts.is_empty()
                && (!url
                    .host_str()
                    .is_some_and(|host| allowed_hosts.contains(&host))
                    || url.port().is_some())
            {
                return Err(Error::UnsafeAddress);
            }

            let request = if let Some(form) = form {
                let request = self
                    .client
                    .post(url.clone())
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(form.to_owned());
                if matches!(kind, ResponseKind::Json) {
                    request.header("X-Requested-With", "XMLHttpRequest")
                } else {
                    request
                }
            } else {
                self.client.get(url.clone())
            };
            let request = if let Some(session) = session.as_ref() {
                session.apply(request, &url)?
            } else {
                request
            };
            let request = if let Some(referer) = referer {
                request.header(REFERER, referer.as_str())
            } else {
                request
            };
            let mut response = request
                .header(
                    ACCEPT,
                    match kind {
                        ResponseKind::Json => "application/json",
                        ResponseKind::Html | ResponseKind::Document => "text/html",
                        ResponseKind::Metadata => {
                            "application/x-bittorrent, application/octet-stream, text/html"
                        }
                        ResponseKind::Image => "image/*",
                        ResponseKind::Playlist => {
                            "application/vnd.apple.mpegurl, application/x-mpegURL, text/plain"
                        }
                    },
                )
                .header(ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(network_error)?;

            if let Some(session) = session.as_mut() {
                session.receive(response.headers(), &url)?;
            }

            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                if redirects == MAX_REDIRECTS {
                    return Err(Error::InvalidRedirect);
                }
                if form.is_some() && !matches!(response.status().as_u16(), 307 | 308) {
                    // Native acquisition follows browser form redirects. Signed
                    // JSON forms retain their existing strict replay contract.
                    if matches!(kind, ResponseKind::Metadata) {
                        form = None;
                    } else {
                        return Err(Error::InvalidRedirect);
                    }
                }

                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(Error::InvalidRedirect)?;
                url = url.join(location).map_err(|_| Error::InvalidRedirect)?;
                continue;
            }

            if !raw && !response.status().is_success() {
                return Err(Error::HttpStatus(response.status().as_u16()));
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .filter(|value| value.len() <= 1024)
                .map(str::to_owned);
            let mime = content_type
                .as_deref()
                .and_then(|value| value.split(';').next())
                .map(str::trim);
            match kind {
                ResponseKind::Html
                    if !mime.is_some_and(|value| value.eq_ignore_ascii_case("text/html")) =>
                {
                    return Err(Error::InvalidResponse("catalog response is not HTML"));
                }
                ResponseKind::Image if !mime.is_some_and(|value| value.starts_with("image/")) => {
                    return Err(Error::InvalidResponse("page response is not an image"));
                }
                _ => {}
            }
            if response
                .content_length()
                .is_some_and(|size| size > max_bytes as u64)
            {
                return Err(Error::ResponseTooLarge);
            }

            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(network_error)? {
                if chunk.len() > max_bytes - bytes.len() {
                    return Err(Error::ResponseTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }

            if bytes.is_empty() && !raw {
                return Err(Error::InvalidResponse("empty source response"));
            }
            return Ok(SourceDocument {
                status: response.status().as_u16(),
                content_type,
                final_url: url,
                body: bytes,
            });
        }

        Err(Error::InvalidRedirect)
    }
}

pub(crate) fn validate_https_url(url: &Url) -> Result<()> {
    validate_media_url(url)?;
    if url.scheme() != "https" {
        return Err(Error::UnsafeAddress);
    }
    Ok(())
}

pub(crate) fn validate_media_url(url: &Url) -> Result<()> {
    let valid = matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.port_or_known_default().is_some_and(|port| port != 0)
        && match url.host() {
            Some(Host::Domain(_)) => true,
            Some(Host::Ipv4(address)) => public_address(IpAddr::V4(address)),
            Some(Host::Ipv6(address)) => public_address(IpAddr::V6(address)),
            None => false,
        };
    if valid {
        Ok(())
    } else {
        Err(Error::UnsafeAddress)
    }
}

fn network_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout
    } else {
        // Reqwest errors can retain the full URL, including acquisition tokens.
        Error::Network(error.without_url())
    }
}

pub(crate) struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses =
                public_addresses(tokio::net::lookup_host((name.as_str(), 0)).await?.collect())?;

            // These checked addresses are handed directly to the connector;
            // there is no second DNS lookup between validation and connection.
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

pub(crate) fn public_addresses(addresses: Vec<SocketAddr>) -> io::Result<Vec<SocketAddr>> {
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !public_address(address.ip()))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Sources cannot access local network addresses",
        ));
    }
    Ok(addresses)
}

/// Return the checked addresses that the caller must hand directly to its
/// connector. Never validate one DNS answer and then connect by hostname.
pub(crate) async fn resolve_public_url(url: &Url) -> io::Result<Vec<SocketAddr>> {
    validate_media_url(url).map_err(|_| {
        io::Error::new(io::ErrorKind::PermissionDenied, "Unsupported media address")
    })?;
    let port = url.port_or_known_default().expect("validated media port");
    let addresses = match url.host().expect("validated media host") {
        Host::Ipv4(address) => vec![SocketAddr::new(IpAddr::V4(address), port)],
        Host::Ipv6(address) => vec![SocketAddr::new(IpAddr::V6(address), port)],
        Host::Domain(host) => tokio::net::lookup_host((host, port)).await?.collect(),
    };
    public_addresses(addresses)
}

fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [a, b, _, _] = address.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 100 && (64..=127).contains(&b)))
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                public_address(IpAddr::V4(mapped))
            } else {
                // Keep the existing source policy: global unicast 2000::/3.
                address.segments()[0] & 0xe000 == 0x2000
            }
        }
    }
}

/// Validate a returned media descriptor without downloading it. The eventual
/// media client must enforce the same address policy on its actual connection.
pub(crate) async fn public_media_url(input: &str) -> Result<Url> {
    let url = Url::parse(input).map_err(|_| Error::UnsafeAddress)?;
    tokio::time::timeout(Duration::from_secs(10), resolve_public_url(&url))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|_| Error::UnsafeAddress)?;
    Ok(url)
}
