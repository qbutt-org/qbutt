use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE, LOCATION};
use serde::de::DeserializeOwned;
use url::{Host, Url};

use crate::{Error, Result};

const MAX_RESPONSE_BYTES: usize = 24 * 1024 * 1024;
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Copy)]
enum ResponseKind {
    Json,
    Html,
    Image,
}

pub(crate) struct SourceHttp {
    client: reqwest::Client,
    origin: Url,
}

impl SourceHttp {
    pub(crate) fn new(origin: Url) -> Result<Self> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(PublicResolver))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("qbutt-content/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(network_error)?;

        Ok(Self { client, origin })
    }

    pub(crate) async fn json<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let bytes =
            tokio::time::timeout(Duration::from_secs(45), self.fetch(url, ResponseKind::Json))
                .await
                .map_err(|_| Error::Timeout)??;
        serde_json::from_slice(&bytes).map_err(Error::Json)
    }

    pub(crate) async fn image(&self, url: Url) -> Result<Vec<u8>> {
        tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(url, ResponseKind::Image),
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    pub(crate) async fn html(&self, url: Url) -> Result<String> {
        let bytes =
            tokio::time::timeout(Duration::from_secs(45), self.fetch(url, ResponseKind::Html))
                .await
                .map_err(|_| Error::Timeout)??;
        String::from_utf8(bytes).map_err(|_| Error::InvalidResponse("HTML is not UTF-8"))
    }

    async fn fetch(&self, mut url: Url, kind: ResponseKind) -> Result<Vec<u8>> {
        let same_origin = !matches!(kind, ResponseKind::Image);
        let max_bytes = if matches!(kind, ResponseKind::Html) {
            MAX_HTML_BYTES
        } else {
            MAX_RESPONSE_BYTES
        };
        for redirects in 0..=MAX_REDIRECTS {
            validate_https_url(&url)?;
            // Catalog redirects stay on the fixed origin. Image descriptors may
            // point to a volunteer CDN; every redirect uses the guarded resolver.
            if same_origin && url.origin() != self.origin.origin() {
                return Err(Error::UnsafeAddress);
            }

            let mut response = self
                .client
                .get(url.clone())
                .header(
                    ACCEPT,
                    match kind {
                        ResponseKind::Json => "application/json",
                        ResponseKind::Html => "text/html",
                        ResponseKind::Image => "image/*",
                    },
                )
                .header(ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(network_error)?;

            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                if redirects == MAX_REDIRECTS {
                    return Err(Error::InvalidRedirect);
                }

                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(Error::InvalidRedirect)?;
                url = url.join(location).map_err(|_| Error::InvalidRedirect)?;
                continue;
            }

            if !response.status().is_success() {
                return Err(Error::HttpStatus(response.status().as_u16()));
            }
            let mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
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

            if bytes.is_empty() {
                return Err(Error::InvalidResponse("empty source response"));
            }
            return Ok(bytes);
        }

        Err(Error::InvalidRedirect)
    }
}

pub(crate) fn validate_https_url(url: &Url) -> Result<()> {
    let valid = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
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

struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses: Vec<SocketAddr> =
                tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty()
                || addresses
                    .iter()
                    .any(|address| !public_address(address.ip()))
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Sources cannot access local network addresses",
                )
                .into());
            }

            // These checked addresses are handed directly to the connector;
            // there is no second DNS lookup between validation and connection.
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
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
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::UnsafeAddress);
    }

    let host = url.host_str().ok_or(Error::UnsafeAddress)?;
    let addresses = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((
            host,
            url.port_or_known_default().ok_or(Error::UnsafeAddress)?,
        )),
    )
    .await
    .map_err(|_| Error::Timeout)?
    .map_err(|_| Error::UnsafeAddress)?;
    let mut count = 0;
    for address in addresses {
        if !public_address(address.ip()) {
            return Err(Error::UnsafeAddress);
        }
        count += 1;
    }
    if count == 0 {
        return Err(Error::UnsafeAddress);
    }

    Ok(url)
}
