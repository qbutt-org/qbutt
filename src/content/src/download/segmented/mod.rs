//! Materialize one fresh VOD descriptor under the queue-owned staging directory.
//! FFmpeg receives local files only; remote requests use the playback HTTP guard.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::Client;
use reqwest::Response;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;
use url::Url;

use super::transfer::{StagedDownload, staged_local_file};
use super::{DownloadError, MediaTools};
use crate::media::MediaResources;
use crate::{PlaybackAudioTrack, PlaybackMedia};

const MAX_MANIFEST: usize = 2 * 1024 * 1024;
const MAX_RESOURCE: u64 = 32 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024 * 1024;
pub(super) const MAX_RESOURCES: u32 = 10_000;

mod dash;
mod hls;
mod mux;

use hls::hls_lines;
use mux::{mux, select_audio, verify_output};

/// Classify an exact fresh media URL by its response, including extensionless
/// manifests. The body is dropped after a small prefix; transfer refetches it.
pub(super) async fn requires_mux(media: &PlaybackMedia) -> Result<bool, DownloadError> {
    if media.audio_url.is_some() || media.audio_track.is_some() || !media.subtitles.is_empty() {
        return Ok(true);
    }
    let source = MediaResources::for_media(media).map_err(|_| DownloadError::Stream)?;
    let (mut response, url) =
        tokio::time::timeout(Duration::from_secs(45), source.get(media.url.clone()))
            .await
            .map_err(|_| DownloadError::Timeout)?
            .map_err(|_| DownloadError::Stream)?;
    if content_kind(&response, &url) != "binary" {
        return Ok(true);
    }
    let mut prefix = Vec::new();
    while prefix.len() < 16 {
        let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), response.chunk())
            .await
            .map_err(|_| DownloadError::Timeout)?
            .map_err(|error| DownloadError::Network(error.without_url()))?
        else {
            break;
        };
        prefix.extend_from_slice(&chunk[..chunk.len().min(1024 - prefix.len())]);
        if prefix.len() >= 1024 {
            break;
        }
    }
    if prefix.get(4..8) == Some(b"ftyp") {
        return Ok(false);
    }
    let text = String::from_utf8_lossy(&prefix);
    let text = text.trim_start_matches(['\u{feff}', ' ', '\r', '\n', '\t']);
    if text.starts_with("#EXTM3U") || text.starts_with("<MPD") || text.contains("<MPD") {
        Ok(true)
    } else {
        Err(DownloadError::Format)
    }
}

pub(super) struct Materializer<'a> {
    source: MediaResources,
    directory: &'a Path,
    next: u32,
    received: u64,
    progress: &'a mut (dyn FnMut(u64, Option<u64>) + Send),
}

pub(super) struct Inputs {
    pub(super) video: PathBuf,
    pub(super) audio: Option<PathBuf>,
    pub(super) subtitles: Vec<SubtitleInput>,
}

pub(super) struct SubtitleInput {
    pub(super) path: PathBuf,
    pub(super) title: String,
    pub(super) language: Option<String>,
}

enum AuxiliaryKind {
    Audio,
    Subtitle,
}

fn audio_title_matches(title: Option<&str>, wanted: &PlaybackAudioTrack) -> bool {
    (wanted.title.is_empty() || title == Some(wanted.title.as_str()))
        && wanted
            .kind
            .as_deref()
            .is_none_or(|kind| title.is_some_and(|title| title.contains(kind)))
}

pub(super) async fn stage(
    media: &PlaybackMedia,
    tools: &MediaTools,
    client: &Client,
    directory: &Path,
    partial: &Path,
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    mut cancel: watch::Receiver<bool>,
) -> Result<StagedDownload, DownloadError> {
    if *cancel.borrow() {
        return Err(DownloadError::Cancelled);
    }
    fs::create_dir(directory).await?;
    let mut materializer = Materializer {
        source: MediaResources::with_client(media, client.clone())
            .map_err(|_| DownloadError::Stream)?,
        directory,
        next: 0,
        received: 0,
        progress,
    };
    let inputs = tokio::select! {
        result = async {
            let mut inputs = materializer.video(media).await?;
            for subtitle in &media.subtitles {
                inputs.subtitles.push(SubtitleInput {
                    path: materializer
                        .resource(subtitle.url.clone(), AuxiliaryKind::Subtitle)
                        .await?,
                    title: subtitle.title.clone(),
                    language: subtitle.language.clone(),
                });
            }
            Ok::<_, DownloadError>(inputs)
        } => result?,
        _ = cancel.changed() => return Err(DownloadError::Cancelled),
    };

    let audio_input = inputs.audio.as_ref().unwrap_or(&inputs.video);
    let audio_stream = if let Some(track) = &media.audio_track {
        Some(
            select_audio(
                tools,
                audio_input,
                track,
                inputs.audio.is_some(),
                &mut cancel,
            )
            .await?,
        )
    } else {
        None
    };
    mux(tools, &inputs, audio_stream, partial, &mut cancel).await?;
    verify_output(tools, partial, &inputs, &mut cancel).await?;
    let staged = staged_local_file(partial).await?;
    (materializer.progress)(staged.size(), Some(staged.size()));
    Ok(staged)
}

impl Materializer<'_> {
    async fn video(&mut self, media: &PlaybackMedia) -> Result<Inputs, DownloadError> {
        let (response, url) = self.get(media.url.clone()).await?;
        let kind = content_kind(&response, &url);
        let (video, mut audio) = match kind {
            "hls" => {
                self.hls(
                    response,
                    url,
                    media.video_height,
                    media
                        .audio_track
                        .as_ref()
                        .filter(|_| media.audio_url.is_none()),
                    media.audio_url.is_some(),
                )
                .await?
            }
            "dash" => {
                self.dash(
                    response,
                    url,
                    media.video_height,
                    media
                        .audio_track
                        .as_ref()
                        .filter(|_| media.audio_url.is_none()),
                    media.audio_url.is_some(),
                )
                .await?
            }
            _ => (self.save(response, &url).await?, None),
        };
        if let Some(external) = &media.audio_url {
            audio = Some(
                self.resource(external.clone(), AuxiliaryKind::Audio)
                    .await?,
            );
        }
        Ok(Inputs {
            video,
            audio,
            subtitles: Vec::new(),
        })
    }

    async fn resource(&mut self, url: Url, kind: AuxiliaryKind) -> Result<PathBuf, DownloadError> {
        let (response, url) = self.get(url).await?;
        match content_kind(&response, &url) {
            "hls" => {
                let body = Self::manifest(response).await?;
                self.hls_media(hls_lines(&body)?, &url).await
            }
            "dash" if matches!(kind, AuxiliaryKind::Audio) => self.dash_audio(response, url).await,
            "dash" => Err(DownloadError::UnsupportedMedia),
            _ => self.save(response, &url).await,
        }
    }

    async fn get(&mut self, url: Url) -> Result<(Response, Url), DownloadError> {
        tokio::time::timeout(Duration::from_secs(45), self.source.get(url))
            .await
            .map_err(|_| DownloadError::Timeout)?
            .map_err(|_| DownloadError::Stream)
    }

    async fn manifest(mut response: Response) -> Result<String, DownloadError> {
        let mut body = Vec::new();
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), response.chunk())
            .await
            .map_err(|_| DownloadError::Timeout)?
            .map_err(|error| DownloadError::Network(error.without_url()))?
        {
            if body.len() + chunk.len() > MAX_MANIFEST {
                return Err(DownloadError::Format);
            }
            body.extend_from_slice(&chunk);
        }
        String::from_utf8(body).map_err(|_| DownloadError::Format)
    }

    async fn save(&mut self, mut response: Response, url: &Url) -> Result<PathBuf, DownloadError> {
        if self.next >= MAX_RESOURCES || response.status() != reqwest::StatusCode::OK {
            return Err(DownloadError::Format);
        }
        let extension = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
            .filter(|extension| {
                extension.len() <= 8 && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .unwrap_or("bin");
        let path = self
            .directory
            .join(format!("resource-{}.{extension}", self.next));
        self.next += 1;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await?;
        let expected = response.content_length();
        let mut size = 0u64;
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), response.chunk())
            .await
            .map_err(|_| DownloadError::Timeout)?
            .map_err(|error| DownloadError::Network(error.without_url()))?
        {
            size = size
                .checked_add(chunk.len() as u64)
                .ok_or(DownloadError::Incomplete)?;
            if size > MAX_RESOURCE || expected.is_some_and(|expected| size > expected) {
                return Err(DownloadError::Incomplete);
            }
            output.write_all(&chunk).await?;
            self.received = self
                .received
                .checked_add(chunk.len() as u64)
                .ok_or(DownloadError::Incomplete)?;
            if self.received > MAX_TOTAL {
                return Err(DownloadError::Incomplete);
            }
            (self.progress)(self.received, None);
        }
        if size == 0 || expected.is_some_and(|expected| expected != size) {
            return Err(DownloadError::Incomplete);
        }
        output.flush().await?;
        output.sync_all().await?;
        Ok(path)
    }
}

fn content_kind(response: &Response, url: &Url) -> &'static str {
    let path = url.path().to_ascii_lowercase();
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if path.ends_with(".m3u8") || path.ends_with(".m3u") || mime.contains("mpegurl") {
        "hls"
    } else if path.ends_with(".mpd") || mime.contains("dash+xml") {
        "dash"
    } else {
        "binary"
    }
}
