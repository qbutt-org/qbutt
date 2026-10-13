use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::watch;

use super::{DownloadError, Inputs, MediaTools, audio_title_matches};
use crate::PlaybackAudioTrack;

#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct VerifiedOutput {
    format: VerifiedFormat,
    streams: Vec<VerifiedStream>,
}

#[derive(Deserialize)]
struct VerifiedFormat {
    format_name: String,
}

#[derive(Deserialize)]
struct VerifiedStream {
    codec_type: String,
    nb_read_packets: Option<String>,
}

#[derive(Deserialize)]
struct Stream {
    index: usize,
    codec_type: String,
    codec_name: Option<String>,
    channels: Option<u16>,
    tags: Option<StreamTags>,
}

#[derive(Deserialize)]
struct StreamTags {
    title: Option<String>,
    language: Option<String>,
}

pub(super) async fn select_audio(
    tools: &MediaTools,
    input: &Path,
    wanted: &PlaybackAudioTrack,
    exact_input: bool,
    cancel: &mut watch::Receiver<bool>,
) -> Result<usize, DownloadError> {
    let mut args = vec!["-v".into(), "error".into()];
    input_options(&mut args, input);
    args.extend(["-show_streams".into(), "-of".into(), "json".into()]);
    args.push(input.to_string_lossy().into_owned());
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run_tool(&tools.ffprobe, &borrowed, Duration::from_secs(30), cancel).await?;
    let probe: Probe = serde_json::from_slice(&output).map_err(|_| DownloadError::Stream)?;
    let audio = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type == "audio")
        .collect::<Vec<_>>();
    let exact_track = exact_input && audio.len() == 1;
    let matches = audio
        .into_iter()
        .filter(|stream| {
            let labels_match = exact_track
                || (audio_title_matches(
                    stream.tags.as_ref().and_then(|tags| tags.title.as_deref()),
                    wanted,
                ) && wanted.language.as_deref().is_none_or(|language| {
                    stream
                        .tags
                        .as_ref()
                        .and_then(|tags| tags.language.as_deref())
                        == Some(language)
                }));
            labels_match
                && wanted
                    .codec
                    .as_deref()
                    .is_none_or(|codec| stream.codec_name.as_deref() == Some(codec))
                && wanted
                    .channels
                    .is_none_or(|channels| stream.channels == Some(channels))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(DownloadError::UnsupportedMedia);
    }
    Ok(matches[0].index)
}

pub(super) async fn mux(
    tools: &MediaTools,
    inputs: &Inputs,
    audio_stream: Option<usize>,
    output: &Path,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), DownloadError> {
    let mut args = vec![
        "-nostdin".to_owned(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
    ];
    for path in std::iter::once(&inputs.video)
        .chain(inputs.audio.iter())
        .chain(inputs.subtitles.iter().map(|subtitle| &subtitle.path))
    {
        input_options(&mut args, path);
        args.extend(["-i".into(), path.to_string_lossy().into_owned()]);
    }
    args.extend(["-map".into(), "0:v:0".into()]);
    match audio_stream {
        Some(stream) => args.extend([
            "-map".into(),
            format!("{}:{stream}", usize::from(inputs.audio.is_some())),
            "-disposition:a:0".into(),
            "default".into(),
        ]),
        None => args.extend([
            "-map".into(),
            if inputs.audio.is_some() {
                "1:a:0"
            } else {
                "0:a:0?"
            }
            .into(),
        ]),
    }
    for (index, _) in inputs.subtitles.iter().enumerate() {
        args.extend([
            "-map".into(),
            format!("{}:s:0", index + 1 + usize::from(inputs.audio.is_some())),
        ]);
    }
    for (index, subtitle) in inputs.subtitles.iter().enumerate() {
        args.extend([
            format!("-metadata:s:s:{index}"),
            format!("title={}", subtitle.title),
        ]);
        if let Some(language) = &subtitle.language {
            args.extend([
                format!("-metadata:s:s:{index}"),
                format!("language={language}"),
            ]);
        }
    }
    args.extend([
        "-c".into(),
        "copy".into(),
        "-f".into(),
        "matroska".into(),
        output.to_string_lossy().into_owned(),
    ]);
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_tool(&tools.ffmpeg, &borrowed, Duration::from_secs(7200), cancel).await?;
    Ok(())
}

fn input_options(args: &mut Vec<String>, path: &Path) {
    args.extend(["-protocol_whitelist".into(), "file,data".into()]);
    if path
        .extension()
        .is_some_and(|extension| extension == "m3u8")
    {
        args.extend([
            "-allowed_extensions".into(),
            "ALL".into(),
            "-allowed_segment_extensions".into(),
            "ALL".into(),
            "-extension_picky".into(),
            "0".into(),
        ]);
    }
}

pub(super) async fn verify_output(
    tools: &MediaTools,
    output: &Path,
    inputs: &Inputs,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), DownloadError> {
    let result = run_tool(
        &tools.ffprobe,
        &[
            "-v",
            "error",
            "-count_packets",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
            output.to_str().ok_or(DownloadError::Format)?,
        ],
        Duration::from_secs(7200),
        cancel,
    )
    .await?;
    let verified: VerifiedOutput =
        serde_json::from_slice(&result).map_err(|_| DownloadError::Stream)?;
    if !verified
        .format
        .format_name
        .split(',')
        .any(|name| name == "matroska")
    {
        return Err(DownloadError::Stream);
    }
    for (kind, expected) in [
        ("video", 1),
        ("audio", usize::from(inputs.audio.is_some())),
        ("subtitle", inputs.subtitles.len()),
    ] {
        let count = verified
            .streams
            .iter()
            .filter(|stream| {
                stream.codec_type == kind
                    && stream
                        .nb_read_packets
                        .as_deref()
                        .and_then(|count| count.parse::<u64>().ok())
                        .is_some_and(|count| count > 0)
            })
            .count();
        if count < expected {
            return Err(DownloadError::Stream);
        }
    }
    Ok(())
}

async fn run_tool(
    path: &Path,
    args: &[&str],
    limit: Duration,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Vec<u8>, DownloadError> {
    if *cancel.borrow() {
        return Err(DownloadError::Cancelled);
    }
    let mut command = Command::new(path);
    command
        .args(args)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|_| DownloadError::MediaTools)?;
    let stdout = child.stdout.take().ok_or(DownloadError::Stream)?;
    let stderr = child.stderr.take().ok_or(DownloadError::Stream)?;
    let result = tokio::select! {
        result = tokio::time::timeout(limit, async {
            tokio::try_join!(
                read_tool_output(stdout, 2 * 1024 * 1024),
                read_tool_output(stderr, 4 * 1024),
                child.wait(),
            )
        }) => match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                child.kill().await?;
                return Err(error.into());
            }
            Err(_) => {
                child.kill().await?;
                return Err(DownloadError::Timeout);
            }
        },
        _ = cancel.changed() => {
            child.kill().await?;
            return Err(DownloadError::Cancelled);
        }
    };
    let (stdout, _stderr, status) = result;
    if !status.success() || stdout.1 {
        #[cfg(feature = "download-fixture")]
        eprintln!(
            "media tool {} failed: {}",
            path.display(),
            String::from_utf8_lossy(&_stderr.0)
        );
        return Err(DownloadError::Stream);
    }
    Ok(stdout.0)
}

async fn read_tool_output<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut data = Vec::new();
    let mut buffer = [0; 8192];
    let mut overflow = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let retained = count.min(limit.saturating_sub(data.len()));
        data.extend_from_slice(&buffer[..retained]);
        overflow |= retained != count;
    }
    Ok((data, overflow))
}
