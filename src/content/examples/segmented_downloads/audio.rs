use std::fs;
use std::path::Path;
use std::process::Command;

use qbutt_content::{
    DirectDownload, DownloadQueue, DownloadStatus, MediaTools, PlaybackAudioTrack, PlaybackMedia,
    PublicationRef, Source, VariantRef, VariantSelector,
};
use url::Url;

use super::{Store, decode, wait_for};

const TITLES: [&str; 4] = [
    "Многоголосый. Studio Alpha",
    "Многоголосый. Studio Beta",
    "Многоголосый. Ambiguous",
    "Многоголосый. Ambiguous",
];

fn run(command: &mut Command) -> Result<String, Box<dyn std::error::Error>> {
    let output = command.output()?;
    assert!(
        output.status.success(),
        "audio fixture tool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

pub(super) fn prepare(
    fixture: &Path,
    tools: &MediaTools,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::new(&tools.ffmpeg);
    command.current_dir(fixture).args([
        "-y",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=blue:s=160x90:r=24:d=2",
    ]);
    for frequency in [440, 880, 660, 550] {
        command.args([
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={frequency}:duration=2"),
        ]);
    }
    command.args(["-map", "0:v"]);
    for index in 1..=4 {
        command.args(["-map", &format!("{index}:a")]);
    }
    command.args(["-c:v", "libx264", "-g", "24", "-c:a", "aac"]);
    for (index, title) in TITLES.iter().enumerate() {
        command.args([
            &format!("-metadata:s:a:{index}"),
            &format!("title={title}"),
            &format!("-metadata:s:a:{index}"),
            "language=rus",
        ]);
    }
    run(command.arg("voices.mkv"))?;
    run(Command::new(&tools.ffmpeg).current_dir(fixture).args([
        "-y",
        "-v",
        "error",
        "-i",
        "voices.mkv",
        "-map",
        "0:v",
        "-c",
        "copy",
        "-hls_time",
        "1",
        "-hls_list_size",
        "0",
        "voices-video.m3u8",
    ]))?;
    for index in 0..TITLES.len() {
        run(Command::new(&tools.ffmpeg).current_dir(fixture).args([
            "-y",
            "-v",
            "error",
            "-i",
            "voices.mkv",
            "-map",
            &format!("0:a:{index}"),
            "-c",
            "copy",
            "-hls_time",
            "1",
            "-hls_list_size",
            "0",
            &format!("voice-{index}.m3u8"),
        ]))?;
    }
    let mut manifest = String::from("#EXTM3U\n");
    for (index, title) in TITLES.iter().enumerate() {
        manifest.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"voices\",NAME=\"{title}\",LANGUAGE=\"ru\",DEFAULT={},URI=\"voice-{index}.m3u8\"\n",
            if index == 0 { "YES" } else { "NO" }
        ));
    }
    manifest.push_str("#EXT-X-STREAM-INF:BANDWIDTH=100000,AUDIO=\"voices\"\nvoices-video.m3u8\n");
    fs::write(fixture.join("voices-master.m3u8"), manifest)?;
    Ok(())
}

fn request(port: u16, path: &str, track: PlaybackAudioTrack, tools: &MediaTools) -> DirectDownload {
    let media = PlaybackMedia::direct(
        Url::parse(&format!("http://fixture.example:{port}/{path}")).unwrap(),
    )
    .with_audio_track(track);
    DirectDownload::segmented_catalog(
        VariantRef {
            publication: PublicationRef {
                source: Source::Rutube,
                id: "0123456789abcdef0123456789abcdef".into(),
            },
            selector: VariantSelector::RutubeVideo,
        },
        "Selected voice".into(),
        media,
        MediaTools {
            ffmpeg: tools.ffmpeg.clone(),
            ffprobe: tools.ffprobe.clone(),
        },
    )
    .unwrap()
}

fn audio_hash(
    tools: &MediaTools,
    path: &Path,
    stream: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    run(Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", stream, "-c", "copy", "-f", "hash", "-hash", "sha256", "-",
        ]))
}

pub(super) async fn verify(
    queue: &DownloadQueue<Store>,
    port: u16,
    fixture: &Path,
    tools: &MediaTools,
) -> Result<(), Box<dyn std::error::Error>> {
    let beta = PlaybackAudioTrack {
        title: TITLES[1].into(),
        kind: Some("Многоголосый".into()),
        language: Some("rus".into()),
        codec: Some("aac".into()),
        channels: Some(1),
    };
    let expected = audio_hash(tools, &fixture.join("voices.mkv"), "0:a:1")?;
    let mut updates = queue.subscribe();
    for path in ["voices.mkv", "voices-master.m3u8"] {
        let mut selection = beta.clone();
        if path.ends_with("m3u8") {
            selection.language = Some("ru".into());
        }
        let job = queue
            .enqueue(request(port, path, selection.clone(), tools))
            .await?;
        let done = wait_for(&mut updates, &job.id, |job| {
            matches!(job.status, DownloadStatus::Complete | DownloadStatus::Error)
        })
        .await;
        assert_eq!(
            done.status,
            DownloadStatus::Complete,
            "{path}: {:?}",
            done.error
        );
        let output = done.path.unwrap();
        let probe: serde_json::Value = serde_json::from_str(&run(Command::new(&tools.ffprobe)
            .args(["-v", "error", "-show_streams", "-of", "json"])
            .arg(&output))?)?;
        let audio = probe["streams"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|stream| stream["codec_type"] == "audio")
            .collect::<Vec<_>>();
        assert_eq!(
            audio.len(),
            1,
            "Only the explicitly selected voice may be saved"
        );
        assert_eq!(audio[0]["disposition"]["default"], 1);
        assert_eq!(
            audio_hash(tools, &output, "0:a:0")?,
            expected,
            "{path}: default Alpha replaced selected Beta"
        );
        decode(&tools.ffmpeg, &output);

        let mut ambiguous = selection.clone();
        ambiguous.title = TITLES[2].into();
        let mut missing = selection.clone();
        missing.title = "Missing".into();
        let mut wrong_kind = selection.clone();
        wrong_kind.kind = Some("Дублированный".into());
        let mut wrong_channels = selection.clone();
        wrong_channels.channels = Some(2);
        let mut wrong_codec = selection;
        wrong_codec.codec = Some("ac3".into());
        for (case, track) in [
            ("ambiguous", ambiguous),
            ("missing", missing),
            ("kind", wrong_kind),
            ("channels", wrong_channels),
            ("codec", wrong_codec),
        ] {
            let job = queue.enqueue(request(port, path, track, tools)).await?;
            let failed = wait_for(&mut updates, &job.id, |job| {
                matches!(job.status, DownloadStatus::Complete | DownloadStatus::Error)
            })
            .await;
            assert_eq!(
                failed.status,
                DownloadStatus::Error,
                "{path}: incorrect {case} was accepted"
            );
            assert!(failed.path.is_none());
        }
    }
    println!(
        "PASS native_download_audio selected_beta_hash=2 default=2 ambiguous_missing_kind_channels_codec_rejected=10"
    );
    Ok(())
}
