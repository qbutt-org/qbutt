use std::path::PathBuf;

use reqwest::Response;
use roxmltree::{Document, Node};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use url::Url;

use super::{DownloadError, MAX_RESOURCES, Materializer};
use crate::PlaybackAudioTrack;

struct DashTrack {
    kind: &'static str,
    bandwidth: u64,
    height: Option<u16>,
    language: Option<String>,
    title: Option<String>,
    codec: Option<String>,
    resources: Vec<Url>,
}

impl Materializer<'_> {
    pub(super) async fn dash_audio(
        &mut self,
        response: Response,
        url: Url,
    ) -> Result<PathBuf, DownloadError> {
        let body = Self::manifest(response).await?;
        let tracks = dash_tracks(&body, &url)?;
        let track = tracks
            .iter()
            .filter(|track| track.kind == "audio")
            .max_by_key(|track| track.bandwidth)
            .ok_or(DownloadError::UnsupportedMedia)?;
        self.dash_track(track).await
    }

    pub(super) async fn dash(
        &mut self,
        response: Response,
        url: Url,
        selected_height: Option<u16>,
        selected_audio: Option<&PlaybackAudioTrack>,
        external_audio: bool,
    ) -> Result<(PathBuf, Option<PathBuf>), DownloadError> {
        let body = Self::manifest(response).await?;
        let tracks = dash_tracks(&body, &url)?;
        let video = tracks
            .iter()
            .filter(|track| track.kind == "video")
            .filter(|track| selected_height.is_none_or(|height| track.height == Some(height)))
            .max_by_key(|track| track.bandwidth)
            .ok_or(DownloadError::Format)?;
        let audio = if external_audio {
            None
        } else if let Some(wanted) = selected_audio {
            let matches = tracks
                .iter()
                .filter(|track| {
                    track.kind == "audio"
                        && (wanted.title.is_empty()
                            || track.title.as_deref() == Some(wanted.title.as_str())
                            || (track.title.is_none()
                                && (wanted.language.is_some() || wanted.codec.is_some())))
                        && wanted
                            .language
                            .as_deref()
                            .is_none_or(|language| track.language.as_deref() == Some(language))
                        && wanted.codec.as_deref().is_none_or(|codec| {
                            track.codec.as_deref().is_some_and(|actual| {
                                actual.starts_with(codec)
                                    || (codec == "aac" && actual.starts_with("mp4a."))
                            })
                        })
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(DownloadError::UnsupportedMedia);
            }
            matches.first().copied()
        } else {
            tracks
                .iter()
                .filter(|track| track.kind == "audio")
                .max_by_key(|track| track.bandwidth)
        };
        let video = self.dash_track(video).await?;
        let audio = match audio {
            Some(audio) => Some(self.dash_track(audio).await?),
            None => None,
        };
        Ok((video, audio))
    }

    async fn dash_track(&mut self, track: &DashTrack) -> Result<PathBuf, DownloadError> {
        let output_path = self.directory.join(format!("track-{}.mp4", self.next));
        self.next += 1;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .await?;
        for url in &track.resources {
            let (response, final_url) = self.get(url.clone()).await?;
            let local = self.save(response, &final_url).await?;
            let mut input = fs::File::open(&local).await?;
            tokio::io::copy(&mut input, &mut output).await?;
            fs::remove_file(local).await?;
        }
        output.flush().await?;
        output.sync_all().await?;
        Ok(output_path)
    }
}

fn dash_tracks(body: &str, manifest: &Url) -> Result<Vec<DashTrack>, DownloadError> {
    if body.contains("<!DOCTYPE") || body.contains("<!ENTITY") {
        return Err(DownloadError::UnsupportedMedia);
    }
    let document = Document::parse(body).map_err(|_| DownloadError::Format)?;
    let root = document.root_element();
    if root.tag_name().name() != "MPD" || root.attribute("type") != Some("static") {
        return Err(DownloadError::UnsupportedMedia);
    }
    if document.descendants().any(|node| {
        node.is_element()
            && (node.tag_name().name() == "ContentProtection"
                || node
                    .attributes()
                    .any(|attribute| attribute.name() == "href" && attribute.namespace().is_some()))
    }) {
        return Err(DownloadError::UnsupportedMedia);
    }
    let periods = root
        .children()
        .filter(|node| node.has_tag_name("Period"))
        .collect::<Vec<_>>();
    if periods.len() != 1 {
        return Err(DownloadError::UnsupportedMedia);
    }
    let period = periods[0];
    let duration = root
        .attribute("mediaPresentationDuration")
        .or_else(|| period.attribute("duration"))
        .and_then(parse_iso_duration);
    let mut tracks = Vec::new();
    for representation in period
        .descendants()
        .filter(|node| node.has_tag_name("Representation"))
    {
        let adaptation = representation
            .ancestors()
            .find(|node| node.has_tag_name("AdaptationSet"))
            .ok_or(DownloadError::Format)?;
        let mime = representation
            .attribute("mimeType")
            .or_else(|| adaptation.attribute("mimeType"))
            .unwrap_or("");
        let kind = if mime.starts_with("video/")
            || adaptation.attribute("contentType") == Some("video")
        {
            "video"
        } else if mime.starts_with("audio/") || adaptation.attribute("contentType") == Some("audio")
        {
            "audio"
        } else {
            continue;
        };
        let base = dash_base(manifest, representation)?;
        let template = inherited_child(representation, "SegmentTemplate");
        let list = inherited_child(representation, "SegmentList");
        let resources = if let Some(template) = template {
            dash_template_urls(template, representation, &base, duration)?
        } else if let Some(list) = list {
            dash_list_urls(list, &base)?
        } else {
            return Err(DownloadError::UnsupportedMedia);
        };
        if resources.is_empty() || resources.len() > MAX_RESOURCES as usize {
            return Err(DownloadError::Format);
        }
        let title = representation
            .children()
            .find(|node| node.has_tag_name("Label"))
            .or_else(|| {
                adaptation
                    .children()
                    .find(|node| node.has_tag_name("Label"))
            })
            .and_then(|node| node.text())
            .map(str::to_owned);
        tracks.push(DashTrack {
            kind,
            bandwidth: representation
                .attribute("bandwidth")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            height: representation
                .attribute("height")
                .and_then(|value| value.parse().ok()),
            language: representation
                .attribute("lang")
                .or_else(|| adaptation.attribute("lang"))
                .map(str::to_owned),
            title,
            codec: representation
                .attribute("codecs")
                .or_else(|| adaptation.attribute("codecs"))
                .map(str::to_owned),
            resources,
        });
    }
    if tracks.is_empty() {
        return Err(DownloadError::Format);
    }
    Ok(tracks)
}

fn inherited_child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.ancestors().find_map(|ancestor| {
        ancestor
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == name)
    })
}

fn dash_base(manifest: &Url, representation: Node<'_, '_>) -> Result<Url, DownloadError> {
    let mut base = manifest.clone();
    let mut ancestors = representation
        .ancestors()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        if let Some(value) = ancestor.attribute(("http://www.w3.org/XML/1998/namespace", "base")) {
            base = base.join(value).map_err(|_| DownloadError::Format)?;
        }
        if let Some(declared) = ancestor
            .children()
            .find(|child| child.has_tag_name("BaseURL"))
        {
            if let Some(value) =
                declared.attribute(("http://www.w3.org/XML/1998/namespace", "base"))
            {
                base = base.join(value).map_err(|_| DownloadError::Format)?;
            }
            base = base
                .join(declared.text().unwrap_or("").trim())
                .map_err(|_| DownloadError::Format)?;
        }
    }
    Ok(base)
}

fn dash_list_urls(list: Node<'_, '_>, base: &Url) -> Result<Vec<Url>, DownloadError> {
    let mut urls = Vec::new();
    if let Some(initial) = list
        .children()
        .find(|node| node.has_tag_name("Initialization"))
    {
        let address = initial
            .attribute("sourceURL")
            .ok_or(DownloadError::Format)?;
        if initial.attribute("range").is_some() {
            return Err(DownloadError::UnsupportedMedia);
        }
        urls.push(base.join(address).map_err(|_| DownloadError::Format)?);
    }
    for segment in list
        .children()
        .filter(|node| node.has_tag_name("SegmentURL"))
    {
        let address = segment.attribute("media").ok_or(DownloadError::Format)?;
        if segment.attribute("mediaRange").is_some() {
            return Err(DownloadError::UnsupportedMedia);
        }
        urls.push(base.join(address).map_err(|_| DownloadError::Format)?);
    }
    Ok(urls)
}

fn dash_template_urls(
    template: Node<'_, '_>,
    representation: Node<'_, '_>,
    base: &Url,
    duration_seconds: Option<f64>,
) -> Result<Vec<Url>, DownloadError> {
    let id = representation
        .attribute("id")
        .ok_or(DownloadError::Format)?;
    let bandwidth = representation
        .attribute("bandwidth")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(DownloadError::Format)?;
    let media = template.attribute("media").ok_or(DownloadError::Format)?;
    let start = template
        .attribute("startNumber")
        .unwrap_or("1")
        .parse::<u64>()
        .map_err(|_| DownloadError::Format)?;
    let timescale = template
        .attribute("timescale")
        .unwrap_or("1")
        .parse::<u64>()
        .map_err(|_| DownloadError::Format)?;
    if timescale == 0 {
        return Err(DownloadError::Format);
    }
    let segments = if let Some(timeline) = template
        .children()
        .find(|node| node.has_tag_name("SegmentTimeline"))
    {
        let mut items = Vec::new();
        let mut time = 0u64;
        let entries = timeline
            .children()
            .filter(|node| node.has_tag_name("S"))
            .collect::<Vec<_>>();
        for (index, entry) in entries.iter().enumerate() {
            let duration = entry
                .attribute("d")
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(DownloadError::Format)?;
            time = entry
                .attribute("t")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(time);
            let repeat = entry
                .attribute("r")
                .unwrap_or("0")
                .parse::<i64>()
                .map_err(|_| DownloadError::Format)?;
            if duration == 0 || !(-1..=MAX_RESOURCES as i64).contains(&repeat) {
                return Err(DownloadError::Format);
            }
            let count = if repeat == -1 {
                let end = entries
                    .get(index + 1)
                    .and_then(|next| next.attribute("t"))
                    .and_then(|value| value.parse::<u64>().ok())
                    .or_else(|| {
                        duration_seconds.map(|seconds| (seconds * timescale as f64).ceil() as u64)
                    })
                    .ok_or(DownloadError::UnsupportedMedia)?;
                end.saturating_sub(time).div_ceil(duration)
            } else {
                repeat as u64 + 1
            };
            if count == 0 || count > MAX_RESOURCES as u64 {
                return Err(DownloadError::Format);
            }
            for _ in 0..count {
                items.push(time);
                time = time.checked_add(duration).ok_or(DownloadError::Format)?;
            }
        }
        items
    } else {
        let duration_seconds = duration_seconds.ok_or(DownloadError::UnsupportedMedia)?;
        let duration = template
            .attribute("duration")
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(DownloadError::Format)?;
        if duration == 0 {
            return Err(DownloadError::Format);
        }
        let count = (duration_seconds * timescale as f64 / duration as f64).ceil() as usize;
        if count == 0 || count > MAX_RESOURCES as usize {
            return Err(DownloadError::Format);
        }
        (0..count).map(|index| index as u64 * duration).collect()
    };
    if segments.is_empty() || segments.len() > MAX_RESOURCES as usize {
        return Err(DownloadError::Format);
    }
    let mut urls = Vec::new();
    if let Some(initial) = template.attribute("initialization") {
        let address = substitute_template(initial, id, bandwidth, start, segments[0])?;
        urls.push(base.join(&address).map_err(|_| DownloadError::Format)?);
    }
    for (index, time) in segments.into_iter().enumerate() {
        let number = start
            .checked_add(index as u64)
            .ok_or(DownloadError::Format)?;
        let address = substitute_template(media, id, bandwidth, number, time)?;
        urls.push(base.join(&address).map_err(|_| DownloadError::Format)?);
    }
    Ok(urls)
}

fn substitute_template(
    template: &str,
    id: &str,
    bandwidth: u64,
    number: u64,
    time: u64,
) -> Result<String, DownloadError> {
    let mut output = String::new();
    let mut parts = template.split('$');
    output.push_str(parts.next().unwrap_or(""));
    loop {
        let Some(variable) = parts.next() else { break };
        if variable.is_empty() {
            output.push('$');
        } else if variable == "RepresentationID" {
            output.push_str(id);
        } else if variable == "Bandwidth" {
            output.push_str(&bandwidth.to_string());
        } else if variable == "Time" {
            output.push_str(&time.to_string());
        } else if variable == "Number" {
            output.push_str(&number.to_string());
        } else if let Some(width) = variable
            .strip_prefix("Number%0")
            .and_then(|value| value.strip_suffix('d'))
        {
            let width = width.parse::<usize>().map_err(|_| DownloadError::Format)?;
            if width > 12 {
                return Err(DownloadError::Format);
            }
            output.push_str(&format!("{number:0width$}"));
        } else {
            return Err(DownloadError::Format);
        }
        output.push_str(parts.next().ok_or(DownloadError::Format)?);
    }
    Ok(output)
}

fn parse_iso_duration(value: &str) -> Option<f64> {
    let mut rest = value.strip_prefix("PT")?;
    let mut seconds = 0.0;
    let mut last_unit = 4;
    while !rest.is_empty() {
        let end = rest.find(|character: char| !character.is_ascii_digit() && character != '.')?;
        let amount = rest[..end].parse::<f64>().ok()?;
        let (unit, multiplier) = match rest.as_bytes().get(end)? {
            b'H' => (3, 3600.0),
            b'M' => (2, 60.0),
            b'S' => (1, 1.0),
            _ => return None,
        };
        if unit >= last_unit {
            return None;
        }
        seconds += amount * multiplier;
        last_unit = unit;
        rest = &rest[end + 1..];
    }
    (seconds.is_finite() && seconds > 0.0 && seconds <= 24.0 * 60.0 * 60.0).then_some(seconds)
}
