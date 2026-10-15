//! Session-scoped byte resources and rewritten, immutable clear HLS playlists.

use std::io;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, Response, StatusCode};
use subtle::ConstantTimeEq;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use url::{Origin, Url};

use super::{HEAD_TIMEOUT, IDLE_TIMEOUT, MAX_HEADERS, MediaGatewayError, hop_header, reject};
use crate::http::{media_client, validate_media_url};
use crate::{PlaybackAudioTrack, PlaybackMedia};

pub struct PreparedPlayback {
    pub url: Url,
    pub audio_url: Option<Url>,
    pub audio_track: Option<PlaybackAudioTrack>,
    pub subtitles: Vec<PreparedSubtitle>,
}

impl PreparedPlayback {
    /// A direct download can save this exact file when no separate audio is
    /// required. The URL remains valid only while its gateway is owned.
    pub fn muxed_mp4_url(&self) -> Option<Url> {
        (self.url.path().ends_with(".mp4")
            && self.audio_url.is_none()
            && self.audio_track.is_none())
        .then(|| self.url.clone())
    }
}

pub struct PreparedSubtitle {
    pub title: String,
    pub url: Url,
    pub language: Option<String>,
}

pub(crate) struct Resources {
    client: Client,
    origin: Origin,
    headers: HeaderMap,
    entries: Vec<Resource>,
}

struct Resource {
    target: Url,
    path: String,
    payload: Payload,
}

enum Payload {
    Remote { mp4: bool },
    Playlist(Vec<u8>),
}

impl Resources {
    /// Use the same source-scoped headers and redirect policy as playback.
    /// Download descriptors and credentials exist only for the active job.
    pub(crate) fn for_media(media: &PlaybackMedia) -> Result<Self, MediaGatewayError> {
        Self::with_client(
            media,
            media_client().map_err(|_| MediaGatewayError::Source)?,
        )
    }

    pub(crate) fn with_client(
        media: &PlaybackMedia,
        client: Client,
    ) -> Result<Self, MediaGatewayError> {
        Ok(Self {
            client,
            origin: media.url.origin(),
            headers: playback_headers(media.headers.clone())?,
            entries: Vec::new(),
        })
    }

    pub(crate) async fn get(&self, url: Url) -> Result<(Response, Url), MediaGatewayError> {
        self.fetch(url, Method::GET, None).await
    }

    pub(super) async fn prepare(
        media: PlaybackMedia,
        proxy: &Url,
    ) -> Result<(Self, PreparedPlayback), MediaGatewayError> {
        let mut resources = Self::for_media(&media)?;
        let muxed_mp4 = !hls_url(&media.url);
        let url = resources.add_media(media.url, proxy, muxed_mp4).await?;
        let audio_url = if let Some(url) = media.audio_url {
            Some(resources.add_media(url, proxy, false).await?)
        } else {
            None
        };
        let mut subtitles = Vec::new();
        for subtitle in media.subtitles {
            subtitles.push(PreparedSubtitle {
                title: subtitle.title,
                url: resources.add(subtitle.url, proxy, false)?,
                language: subtitle.language,
            });
        }
        if muxed_mp4 {
            // A small range proves an ordinary file is a seekable MP4 before
            // the caller launches a player or offers a direct download.
            let (mut response, _) = tokio::time::timeout(
                Duration::from_secs(45),
                resources.fetch(
                    resources.entries[0].target.clone(),
                    Method::GET,
                    Some("bytes=0-1023"),
                ),
            )
            .await
            .map_err(|_| MediaGatewayError::Source)??;
            if !valid_range_response(&response, Some("bytes=0-1023")) {
                return Err(MediaGatewayError::InvalidDescriptor);
            }
            let prefix = response_prefix(&mut response).await?;
            validate_bytes(&response, &prefix)?;
            if prefix.get(4..8) != Some(b"ftyp") || !mp4_content_type(&response) {
                return Err(MediaGatewayError::InvalidDescriptor);
            }
        }
        Ok((
            resources,
            PreparedPlayback {
                url,
                audio_url,
                audio_track: media.audio_track,
                subtitles,
            },
        ))
    }

    async fn add_media(
        &mut self,
        target: Url,
        proxy: &Url,
        mp4: bool,
    ) -> Result<Url, MediaGatewayError> {
        if hls_url(&target) {
            self.add_hls(target, proxy).await
        } else {
            self.add(target, proxy, mp4)
        }
    }

    async fn add_hls(&mut self, target: Url, proxy: &Url) -> Result<Url, MediaGatewayError> {
        const MAX_MANIFEST: usize = 512 * 1024;
        let (mut response, base) = tokio::time::timeout(Duration::from_secs(15), self.get(target))
            .await
            .map_err(|_| MediaGatewayError::Source)??;
        if response.status() != StatusCode::OK
            || response
                .content_length()
                .is_some_and(|size| size > MAX_MANIFEST as u64)
        {
            return Err(MediaGatewayError::InvalidDescriptor);
        }
        let body = tokio::time::timeout(Duration::from_secs(15), async {
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| MediaGatewayError::Source)?
            {
                if chunk.len() > MAX_MANIFEST - body.len() {
                    return Err(MediaGatewayError::InvalidDescriptor);
                }
                body.extend_from_slice(&chunk);
            }
            String::from_utf8(body).map_err(|_| MediaGatewayError::InvalidDescriptor)
        })
        .await
        .map_err(|_| MediaGatewayError::Source)??;
        let lines = super::hls::lines(&body)
            .and_then(super::hls::media)
            .map_err(|error| match error {
                super::hls::Error::Format => MediaGatewayError::InvalidDescriptor,
                super::hls::Error::Unsupported => MediaGatewayError::UnsupportedManifest,
            })?;
        let fragmented = lines.iter().any(
            |line| matches!(line, super::hls::Line::Resource { prefix, .. } if !prefix.is_empty()),
        );
        let mut rewritten = String::new();
        for line in lines {
            match line {
                super::hls::Line::Tag(line) => rewritten.push_str(line),
                super::hls::Line::Resource {
                    prefix,
                    uri,
                    suffix,
                } => {
                    let target = base
                        .join(uri)
                        .map_err(|_| MediaGatewayError::InvalidDescriptor)?;
                    // Local capability names describe the HLS role; CDN URLs
                    // often have no media extension or use an opaque suffix.
                    let extension = if !prefix.is_empty() {
                        "mp4"
                    } else if fragmented {
                        "m4s"
                    } else {
                        "ts"
                    };
                    let local =
                        self.insert(target, proxy, extension, Payload::Remote { mp4: false })?;
                    rewritten.push_str(prefix);
                    rewritten.push_str(local.as_str());
                    rewritten.push_str(suffix);
                }
            }
            rewritten.push('\n');
        }
        self.insert(
            base,
            proxy,
            "m3u8",
            Payload::Playlist(rewritten.into_bytes()),
        )
    }

    fn add(&mut self, target: Url, proxy: &Url, mp4: bool) -> Result<Url, MediaGatewayError> {
        if matches!(
            target
                .path()
                .rsplit('.')
                .next()
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("m3u8" | "m3u" | "mpd")
        ) {
            return Err(MediaGatewayError::UnsupportedManifest);
        }
        let suffix = if mp4 {
            "mp4"
        } else {
            target
                .path()
                .rsplit_once('.')
                .map(|(_, suffix)| suffix)
                .filter(|suffix| {
                    suffix.len() <= 8 && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
                .unwrap_or("bin")
        };
        let suffix = suffix.to_owned();
        self.insert(target, proxy, &suffix, Payload::Remote { mp4 })
    }

    fn insert(
        &mut self,
        target: Url,
        proxy: &Url,
        suffix: &str,
        payload: Payload,
    ) -> Result<Url, MediaGatewayError> {
        validate_media_url(&target).map_err(|_| MediaGatewayError::InvalidDescriptor)?;
        if self.entries.len() >= super::hls::MAX_RESOURCES + 64 {
            return Err(MediaGatewayError::InvalidDescriptor);
        }
        let path = format!(
            "/media/{}/{}/resource.{suffix}",
            proxy.password().expect("gateway password"),
            self.entries.len()
        );
        let mut local = proxy.clone();
        local.set_username("").expect("local HTTP URL");
        local.set_password(None).expect("local HTTP URL");
        local.set_path(&path);
        self.entries.push(Resource {
            target,
            path,
            payload,
        });
        Ok(local)
    }

    async fn fetch(
        &self,
        mut target: Url,
        method: Method,
        range: Option<&str>,
    ) -> Result<(Response, Url), MediaGatewayError> {
        for redirects in 0..=5 {
            validate_media_url(&target).map_err(|_| MediaGatewayError::InvalidDescriptor)?;
            let mut request = self.client.request(method.clone(), target.clone());
            for (name, value) in &self.headers {
                if target.origin() == self.origin || public_header(name.as_str()) {
                    request = request.header(name, value);
                }
            }
            request = request.header("Accept-Encoding", "identity");
            if let Some(range) = range {
                request = request.header("Range", range);
            }
            let response = request
                .send()
                .await
                .map_err(|_| MediaGatewayError::Source)?;
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                if redirects == 5 {
                    return Err(MediaGatewayError::Source);
                }
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .ok_or(MediaGatewayError::Source)?;
                let next = target
                    .join(location)
                    .map_err(|_| MediaGatewayError::Source)?;
                if target.scheme() == "https" && next.scheme() != "https" {
                    return Err(MediaGatewayError::InvalidDescriptor);
                }
                target = next;
                continue;
            }
            if !response.status().is_success()
                && response.status() != StatusCode::RANGE_NOT_SATISFIABLE
            {
                return Err(MediaGatewayError::Source);
            }
            return Ok((response, target));
        }
        unreachable!("redirect loop has a terminal bound")
    }

    pub(super) async fn handle(
        &self,
        mut client: TcpStream,
        head: &[u8],
        remainder: &[u8],
    ) -> io::Result<()> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        if !matches!(request.parse(head), Ok(httparse::Status::Complete(_)))
            || !remainder.is_empty()
            || request.headers.iter().any(|header| {
                header.name.eq_ignore_ascii_case("transfer-encoding")
                    || header.name.eq_ignore_ascii_case("content-length") && header.value != b"0"
            })
        {
            return reject(&mut client, 400).await;
        }
        let method = match request.method {
            Some("GET") => Method::GET,
            Some("HEAD") => Method::HEAD,
            _ => return reject(&mut client, 405).await,
        };
        let Some(index) = self.entries.iter().position(|resource| {
            request
                .path
                .is_some_and(|supplied| resource.path.as_bytes().ct_eq(supplied.as_bytes()).into())
        }) else {
            return reject(&mut client, 403).await;
        };
        let ranges: Vec<_> = request
            .headers
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case("range"))
            .collect();
        let range = match ranges.as_slice() {
            [] => None,
            [header] => match std::str::from_utf8(header.value) {
                Ok(value) if byte_range(value) => Some(value),
                _ => return reject(&mut client, 400).await,
            },
            _ => return reject(&mut client, 400).await,
        };
        let mp4 = match &self.entries[index].payload {
            Payload::Remote { mp4 } => *mp4,
            Payload::Playlist(body) => {
                let (status, content_range, bytes) = if let Some(range) = range {
                    let (first, last) = range
                        .strip_prefix("bytes=")
                        .unwrap()
                        .split_once('-')
                        .unwrap();
                    let (start, end) = if first.is_empty() {
                        (
                            body.len()
                                .saturating_sub(last.parse::<usize>().unwrap_or(usize::MAX)),
                            body.len() - 1,
                        )
                    } else {
                        (
                            first.parse::<usize>().unwrap_or(usize::MAX),
                            last.parse::<usize>()
                                .unwrap_or(usize::MAX)
                                .min(body.len() - 1),
                        )
                    };
                    if start >= body.len() {
                        let outgoing = format!(
                            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        tokio::time::timeout(HEAD_TIMEOUT, client.write_all(outgoing.as_bytes()))
                            .await??;
                        return client.shutdown().await;
                    }
                    (
                        "206 Partial Content",
                        format!("Content-Range: bytes {start}-{end}/{}\r\n", body.len()),
                        &body[start..=end],
                    )
                } else {
                    ("200 OK", String::new(), body.as_slice())
                };
                let outgoing = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\n{content_range}Cache-Control: no-store\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                tokio::time::timeout(HEAD_TIMEOUT, client.write_all(outgoing.as_bytes())).await??;
                if method == Method::GET {
                    tokio::time::timeout(IDLE_TIMEOUT, client.write_all(bytes)).await??;
                }
                return client.shutdown().await;
            }
        };
        let mut response = match tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(self.entries[index].target.clone(), method.clone(), range),
        )
        .await
        {
            Ok(Ok((response, _))) => response,
            _ => return reject(&mut client, 502).await,
        };
        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
            if range.is_none() {
                return reject(&mut client, 502).await;
            }
            let Some(content_range) = response
                .headers()
                .get("content-range")
                .and_then(|value| value.to_str().ok())
                .filter(|value| {
                    value
                        .strip_prefix("bytes */")
                        .is_some_and(|size| size.parse::<u64>().is_ok())
                })
            else {
                return reject(&mut client, 502).await;
            };
            let outgoing = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\nConnection: close\r\nContent-Range: {content_range}\r\nContent-Length: 0\r\n\r\n"
            );
            tokio::time::timeout(HEAD_TIMEOUT, client.write_all(outgoing.as_bytes())).await??;
            return client.shutdown().await;
        }
        if !valid_range_response(&response, range) {
            return reject(&mut client, 502).await;
        }
        let prefix = if method == Method::HEAD {
            Vec::new()
        } else {
            match response_prefix(&mut response).await {
                Ok(prefix) => prefix,
                Err(_) => return reject(&mut client, 502).await,
            }
        };
        if validate_bytes(&response, &prefix).is_err()
            || mp4 && !mp4_content_type(&response)
            || mp4
                && method != Method::HEAD
                && (range.is_none() || range.is_some_and(|range| range.starts_with("bytes=0-")))
                && prefix.get(4..8) != Some(b"ftyp")
        {
            return reject(&mut client, 502).await;
        }
        let mut outgoing = format!(
            "HTTP/1.1 {} {}\r\nConnection: close\r\n",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("OK")
        );
        for name in [
            "content-type",
            "content-length",
            "content-range",
            "accept-ranges",
            "etag",
            "last-modified",
        ] {
            if mp4 && name == "content-type" {
                outgoing.push_str("content-type: video/mp4\r\n");
                continue;
            }
            if let Some(value) = response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
            {
                outgoing.push_str(&format!("{name}: {value}\r\n"));
            }
        }
        outgoing.push_str("\r\n");
        tokio::time::timeout(HEAD_TIMEOUT, client.write_all(outgoing.as_bytes())).await??;
        if method == Method::HEAD {
            return client.shutdown().await;
        }
        tokio::time::timeout(IDLE_TIMEOUT, client.write_all(&prefix)).await??;
        while let Some(chunk) = response.chunk().await.map_err(io::Error::other)? {
            tokio::time::timeout(IDLE_TIMEOUT, client.write_all(&chunk)).await??;
        }
        client.shutdown().await
    }
}

fn hls_url(url: &Url) -> bool {
    url.path().rsplit('.').next().is_some_and(|suffix| {
        suffix.eq_ignore_ascii_case("m3u8") || suffix.eq_ignore_ascii_case("m3u")
    })
}

fn playback_headers(
    headers: std::collections::BTreeMap<String, String>,
) -> Result<HeaderMap, MediaGatewayError> {
    let mut result = HeaderMap::new();
    for (name, mut value) in headers {
        if hop_header(&name)
            || ["host", "accept-encoding", "content-length", "range"]
                .iter()
                .any(|excluded| name.eq_ignore_ascii_case(excluded))
        {
            continue;
        }
        if name.eq_ignore_ascii_case("referer") || name.eq_ignore_ascii_case("origin") {
            let mut url = Url::parse(&value).map_err(|_| MediaGatewayError::InvalidDescriptor)?;
            validate_media_url(&url).map_err(|_| MediaGatewayError::InvalidDescriptor)?;
            url.set_query(None);
            url.set_fragment(None);
            value = if name.eq_ignore_ascii_case("origin") {
                url.origin().ascii_serialization()
            } else {
                url.to_string()
            };
        }
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| MediaGatewayError::InvalidDescriptor)?;
        let value =
            HeaderValue::from_str(&value).map_err(|_| MediaGatewayError::InvalidDescriptor)?;
        if result.insert(name, value).is_some() {
            return Err(MediaGatewayError::InvalidDescriptor);
        }
    }
    Ok(result)
}

fn public_header(name: &str) -> bool {
    matches!(
        name,
        "user-agent" | "referer" | "origin" | "accept" | "accept-language"
    )
}

fn byte_range(value: &str) -> bool {
    let Some((start, end)) = value
        .strip_prefix("bytes=")
        .and_then(|value| value.split_once('-'))
    else {
        return false;
    };
    (!start.is_empty() || end.parse::<u64>().is_ok_and(|count| count > 0))
        && [start, end]
            .into_iter()
            .all(|number| number.is_empty() || number.len() <= 20 && number.parse::<u64>().is_ok())
        && (start.is_empty()
            || end.is_empty()
            || start.parse::<u64>().unwrap() <= end.parse::<u64>().unwrap())
}

fn valid_range_response(response: &Response, requested: Option<&str>) -> bool {
    let Some(requested) = requested else {
        return response.status() == StatusCode::OK;
    };
    // A complete response also fulfills an open range beginning at zero.
    if response.status() == StatusCode::OK {
        return requested == "bytes=0-" && !response.headers().contains_key("content-range");
    }
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return false;
    }
    let Some((start, end, total)) = response
        .headers()
        .get("content-range")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes "))
        .and_then(|value| value.split_once('/'))
        .and_then(|(span, total)| {
            let (start, end) = span.split_once('-')?;
            Some((
                start.parse::<u64>().ok()?,
                end.parse::<u64>().ok()?,
                total.parse::<u64>().ok()?,
            ))
        })
    else {
        return false;
    };
    if total == 0
        || start > end
        || end >= total
        || response
            .content_length()
            .is_some_and(|length| length != end - start + 1)
    {
        return false;
    }
    let Some((first, last)) = requested
        .strip_prefix("bytes=")
        .and_then(|range| range.split_once('-'))
    else {
        return false;
    };
    if first.is_empty() {
        return last.parse::<u64>().is_ok_and(|count| {
            count > 0 && end == total - 1 && end - start + 1 == count.min(total)
        });
    }
    first.parse::<u64>().is_ok_and(|first| first == start)
        && (last.is_empty() && end == total - 1
            || last
                .parse::<u64>()
                .is_ok_and(|last| end == last.min(total - 1)))
}

fn mp4_content_type(response: &Response) -> bool {
    response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .is_some_and(|mime| {
            matches!(
                mime.as_str(),
                "video/mp4"
                    | "application/mp4"
                    | "application/octet-stream"
                    | "binary/octet-stream"
            )
        })
}

async fn response_prefix(response: &mut Response) -> Result<Vec<u8>, MediaGatewayError> {
    let mut prefix = Vec::new();
    while prefix.len() < 1024 {
        let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MediaGatewayError::Source)?
        else {
            break;
        };
        if prefix.len() + chunk.len() > 64 * 1024 {
            return Err(MediaGatewayError::Source);
        }
        prefix.extend_from_slice(&chunk);
    }
    Ok(prefix)
}

fn validate_bytes(response: &Response, prefix: &[u8]) -> Result<(), MediaGatewayError> {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let text = String::from_utf8_lossy(&prefix[..prefix.len().min(1024)]);
    let text = text.trim_start_matches(['\u{feff}', ' ', '\r', '\n', '\t']);
    if content_type.contains("mpegurl")
        || content_type.contains("dash+xml")
        || text.starts_with("#EXTM3U")
        || text.contains("<MPD")
    {
        return Err(MediaGatewayError::UnsupportedManifest);
    }
    Ok(())
}
