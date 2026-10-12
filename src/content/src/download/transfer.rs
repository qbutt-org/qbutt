//! MP4 byte transfer, verification and atomic publication within one volume.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::{Client, StatusCode};
use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use url::Url;

use super::DownloadError;
use crate::http::{PublicResolver, validate_media_url};

const MAX_REDIRECTS: usize = 5;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AddressPolicy {
    Public,
    Gateway,
}

impl AddressPolicy {
    pub(super) fn valid(self, url: &Url) -> bool {
        match self {
            Self::Public => validate_media_url(url).is_ok(),
            Self::Gateway => {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port().is_some_and(|port| port != 0)
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.path().starts_with("/media/")
            }
        }
    }
}

pub(super) async fn verify_candidate(candidate: &PublishCandidate) -> Result<bool, DownloadError> {
    let mut file = match fs::File::open(&candidate.path).await {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if file.metadata().await?.len() != candidate.size {
        return Ok(false);
    }
    let mut hash = Context::new(&SHA256);
    let mut buffer = [0u8; 64 * 1024];
    use tokio::io::AsyncReadExt;
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let actual = hash
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(actual == candidate.sha256)
}

pub(super) async fn move_without_replace_async(
    source: &Path,
    destination: &Path,
) -> io::Result<()> {
    let source = source.to_owned();
    let destination = destination.to_owned();
    tokio::task::spawn_blocking(move || move_without_replace(&source, &destination))
        .await
        .map_err(|_| io::Error::other("download publication failed"))?
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PublishCandidate {
    pub(super) path: PathBuf,
    pub(super) size: u64,
    pub(super) sha256: String,
}

pub(super) struct StagedDownload {
    path: PathBuf,
    size: u64,
    sha256: String,
}

impl StagedDownload {
    pub(super) fn size(&self) -> u64 {
        self.size
    }

    pub(super) fn candidate(&self, path: PathBuf) -> PublishCandidate {
        PublishCandidate {
            path,
            size: self.size,
            sha256: self.sha256.clone(),
        }
    }

    pub(super) async fn discard(self) -> Result<(), DownloadError> {
        match fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

pub(super) fn public_client() -> Result<Client, DownloadError> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(PublicResolver))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("qbutt-content/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| DownloadError::Network(error.without_url()))
}

pub(super) fn local_client() -> Result<Client, DownloadError> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| DownloadError::Network(error.without_url()))
}

pub(super) async fn stage_direct(
    client: &Client,
    address: &Url,
    policy: AddressPolicy,
    partial: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<StagedDownload, DownloadError> {
    let mut url = address.clone();
    let mut response = None;
    for redirects in 0..=MAX_REDIRECTS {
        if !policy.valid(&url) {
            return Err(DownloadError::UnsafeAddress);
        }
        let received = tokio::time::timeout(
            IDLE_TIMEOUT,
            client
                .get(url.clone())
                .header(ACCEPT_ENCODING, "identity")
                .send(),
        )
        .await
        .map_err(|_| DownloadError::Timeout)?
        .map_err(|error| DownloadError::Network(error.without_url()))?;
        if received.status().is_redirection() {
            if policy == AddressPolicy::Gateway || redirects == MAX_REDIRECTS {
                return Err(DownloadError::Redirect);
            }
            let location = received
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(DownloadError::Redirect)?;
            url = url.join(location).map_err(|_| DownloadError::Redirect)?;
            continue;
        }
        response = Some(received);
        break;
    }
    let mut response = response.ok_or(DownloadError::Redirect)?;
    if !response.status().is_success() {
        return Err(DownloadError::Http(response.status().as_u16()));
    }
    if response.status() != StatusCode::OK {
        return Err(DownloadError::Format);
    }
    if response
        .headers()
        .get(CONTENT_ENCODING)
        .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(DownloadError::Format);
    }
    let mime = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if !mime.eq_ignore_ascii_case("video/mp4")
        && !mime.eq_ignore_ascii_case("application/octet-stream")
    {
        return Err(DownloadError::Format);
    }
    let total = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(partial)
        .await?;
    let mut digest = Context::new(&SHA256);
    let mut bytes = 0u64;
    let mut first = Vec::new();
    while let Some(chunk) = tokio::time::timeout(IDLE_TIMEOUT, response.chunk())
        .await
        .map_err(|_| DownloadError::Timeout)?
        .map_err(|error| DownloadError::Network(error.without_url()))?
    {
        if first.len() < 12 {
            first.extend_from_slice(&chunk[..chunk.len().min(12 - first.len())]);
        }
        output.write_all(&chunk).await?;
        digest.update(&chunk);
        bytes = bytes
            .checked_add(chunk.len() as u64)
            .ok_or(DownloadError::Incomplete)?;
        if total.is_some_and(|total| bytes > total) {
            return Err(DownloadError::Incomplete);
        }
        progress(bytes, total);
    }
    if bytes == 0 {
        return Err(DownloadError::Empty);
    }
    if total.is_some_and(|total| bytes != total) {
        return Err(DownloadError::Incomplete);
    }
    if first.len() < 12 || &first[4..8] != b"ftyp" {
        return Err(DownloadError::Format);
    }
    output.flush().await?;
    output.sync_all().await?;
    drop(output);
    Ok(StagedDownload {
        path: partial.to_owned(),
        size: bytes,
        sha256: digest
            .finish()
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    })
}

pub(super) fn safe_file_name(title: &str) -> String {
    let mut name = title
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    name = name
        .chars()
        .take(150)
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .to_owned();
    if name.is_empty()
        || matches!(
            name.split('.')
                .next()
                .unwrap_or("")
                .trim_end_matches(' ')
                .to_ascii_lowercase()
                .as_str(),
            "con"
                | "conin$"
                | "conout$"
                | "prn"
                | "aux"
                | "nul"
                | "com1"
                | "com2"
                | "com3"
                | "com4"
                | "com5"
                | "com6"
                | "com7"
                | "com8"
                | "com9"
                | "lpt1"
                | "lpt2"
                | "lpt3"
                | "lpt4"
                | "lpt5"
                | "lpt6"
                | "lpt7"
                | "lpt8"
                | "lpt9"
        )
    {
        name = "Download".into();
    }
    if !name.to_ascii_lowercase().ends_with(".mp4") {
        name.push_str(".mp4");
    }
    name
}

#[cfg(windows)]
fn move_without_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let from = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn move_without_replace(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::hard_link(source, destination)?;
    std::fs::remove_file(source)
}
