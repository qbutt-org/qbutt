use std::path::{Path, PathBuf};

use reqwest::Response;
use tokio::fs;
use url::Url;

use super::{DownloadError, Materializer, audio_title_matches};
use crate::PlaybackAudioTrack;

impl Materializer<'_> {
    pub(super) async fn hls(
        &mut self,
        response: Response,
        url: Url,
        selected_height: Option<u16>,
        selected_audio: Option<&PlaybackAudioTrack>,
        external_audio: bool,
    ) -> Result<(PathBuf, Option<PathBuf>), DownloadError> {
        let body = Self::manifest(response).await?;
        let lines = hls_lines(&body)?;
        if lines
            .iter()
            .any(|line| line.starts_with("#EXT-X-SESSION-KEY:"))
        {
            return Err(DownloadError::UnsupportedMedia);
        }
        let variants = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                line.strip_prefix("#EXT-X-STREAM-INF:")
                    .and_then(|attributes| lines.get(index + 1).map(|next| (attributes, *next)))
            })
            .collect::<Vec<_>>();
        if variants.is_empty() {
            return Ok((self.hls_media(lines, &url).await?, None));
        }
        let (attributes, target) = variants
            .into_iter()
            .filter(|(attributes, _)| {
                selected_height.is_none_or(|height| {
                    hls_attribute(attributes, "RESOLUTION")
                        .and_then(|resolution| resolution.split_once('x'))
                        .and_then(|(_, height)| height.parse::<u16>().ok())
                        == Some(height)
                })
            })
            .max_by_key(|(attributes, _)| {
                hls_attribute(attributes, "BANDWIDTH")
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0)
            })
            .ok_or(DownloadError::Format)?;
        let group = hls_attribute(attributes, "AUDIO");
        let choices = lines
            .iter()
            .filter_map(|line| line.strip_prefix("#EXT-X-MEDIA:"))
            .filter(|attributes| hls_attribute(attributes, "TYPE") == Some("AUDIO"))
            .filter(|attributes| {
                group.is_none_or(|group| hls_attribute(attributes, "GROUP-ID") == Some(group))
            })
            .collect::<Vec<_>>();
        let chosen = if external_audio {
            None
        } else if let Some(track) = selected_audio {
            let matches = choices
                .iter()
                .filter(|attributes| {
                    audio_title_matches(hls_attribute(attributes, "NAME"), track)
                        && track.language.as_deref().is_none_or(|language| {
                            hls_attribute(attributes, "LANGUAGE") == Some(language)
                        })
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(DownloadError::UnsupportedMedia);
            }
            matches.into_iter().next().copied()
        } else {
            choices
                .iter()
                .find(|attributes| hls_attribute(attributes, "DEFAULT") == Some("YES"))
                .or_else(|| choices.first())
                .copied()
        };
        let video_url = url.join(target).map_err(|_| DownloadError::Format)?;
        let (video_response, video_url) = self.get(video_url).await?;
        let video_body = Self::manifest(video_response).await?;
        let video_lines = hls_lines(&video_body)?;
        let video = self.hls_media(video_lines, &video_url).await?;
        let audio = if let Some(attributes) = chosen {
            if let Some(audio_url) = hls_attribute(attributes, "URI") {
                let audio_url = url.join(audio_url).map_err(|_| DownloadError::Format)?;
                let (audio_response, audio_url) = self.get(audio_url).await?;
                let audio_body = Self::manifest(audio_response).await?;
                let audio_lines = hls_lines(&audio_body)?;
                Some(self.hls_media(audio_lines, &audio_url).await?)
            } else {
                None
            }
        } else {
            None
        };
        Ok((video, audio))
    }

    pub(super) async fn hls_media(
        &mut self,
        lines: Vec<&str>,
        base: &Url,
    ) -> Result<PathBuf, DownloadError> {
        if !lines.iter().any(|line| *line == "#EXT-X-ENDLIST")
            || lines
                .iter()
                .any(|line| line.contains("BYTERANGE") || line.starts_with("#EXT-X-PART"))
        {
            return Err(DownloadError::UnsupportedMedia);
        }
        let mut rewritten = Vec::with_capacity(lines.len());
        for line in lines {
            if line.starts_with("#EXT-X-KEY:") {
                if !line.contains("METHOD=NONE") || line.contains("URI=") {
                    return Err(DownloadError::UnsupportedMedia);
                }
            } else if line.starts_with("#EXT-X-SESSION-KEY:") {
                return Err(DownloadError::UnsupportedMedia);
            }
            if !line.starts_with('#') {
                let target = base.join(line).map_err(|_| DownloadError::Format)?;
                let (response, target) = self.get(target).await?;
                let path = self.save(response, &target).await?;
                rewritten.push(local_name(&path)?);
            } else if line.contains("URI=\"") {
                let start = line.find("URI=\"").ok_or(DownloadError::Format)? + 5;
                let end = line[start..].find('"').ok_or(DownloadError::Format)? + start;
                let target = base
                    .join(&line[start..end])
                    .map_err(|_| DownloadError::Format)?;
                let (response, target) = self.get(target).await?;
                let path = self.save(response, &target).await?;
                rewritten.push(format!(
                    "{}{}{}",
                    &line[..start],
                    local_name(&path)?,
                    &line[end..]
                ));
            } else {
                rewritten.push(line.to_owned());
            }
        }
        let path = self.directory.join(format!("playlist-{}.m3u8", self.next));
        self.next += 1;
        fs::write(&path, format!("{}\n", rewritten.join("\n"))).await?;
        Ok(path)
    }
}

pub(super) fn hls_lines(body: &str) -> Result<Vec<&str>, DownloadError> {
    let lines = body
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.first() != Some(&"#EXTM3U") {
        return Err(DownloadError::Format);
    }
    Ok(lines)
}

fn hls_attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let mut quoted = false;
    let mut start = 0;
    for (index, byte) in attributes.bytes().enumerate() {
        if byte == b'"' {
            quoted = !quoted;
        }
        if byte != b',' || quoted {
            continue;
        }
        let field = &attributes[start..index];
        let (key, value) = field.split_once('=')?;
        if key == name {
            return Some(value.trim_matches('"'));
        }
        start = index + 1;
    }
    let (key, value) = attributes[start..].split_once('=')?;
    (key == name).then(|| value.trim_matches('"'))
}

fn local_name(path: &Path) -> Result<String, DownloadError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or(DownloadError::Format)
}
