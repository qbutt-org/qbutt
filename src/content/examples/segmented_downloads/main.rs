//! Generated clear VOD integration for the native queue and qplay media tools.
//! Pass a fresh output directory, generated HLS/DASH media directory, and the
//! verified ffmpeg/ffprobe paths from one qplay runtime.

mod audio;

use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qbutt_content::{
    DirectDownload, DownloadJob, DownloadQueue, DownloadStatus, DownloadStorage, Library,
    LibraryResult, MediaTools, PlaybackAudioTrack, PlaybackMedia, PublicationRef, Source,
    VariantRef, VariantSelector,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

#[derive(Clone)]
struct Store(Arc<Mutex<Library>>);

#[derive(Clone, Copy)]
enum FixtureTrack {
    None,
    Audio,
    Subtitle,
}

impl DownloadStorage for Store {
    async fn load(&self) -> LibraryResult<Vec<DownloadJob>> {
        let store = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || store.lock().unwrap().download_jobs())
            .await
            .expect("fixture storage thread")
    }

    async fn save(&self, job: DownloadJob) -> LibraryResult<()> {
        let store = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || store.lock().unwrap().save_download_job(&job))
            .await
            .expect("fixture storage thread")
    }
}

fn request(
    port: u16,
    path: &str,
    title: &str,
    tools: &MediaTools,
    track: FixtureTrack,
) -> DirectDownload {
    let url = Url::parse(&format!("http://fixture.example:{port}/{path}")).unwrap();
    let mut media = PlaybackMedia::direct(url);
    if path.ends_with(".mpd") {
        media = media.with_video_height(90);
    } else if path.ends_with("master.m3u8") {
        media = media
            .with_video_height(90)
            .with_audio_track(PlaybackAudioTrack {
                title: "English, stereo".into(),
                kind: None,
                language: Some("en".into()),
                codec: None,
                channels: None,
            });
    }
    media = match track {
        FixtureTrack::None => media,
        FixtureTrack::Audio => media.with_audio_url(
            Url::parse(&format!("http://fixture.example:{port}/audio.m4a")).unwrap(),
        ),
        FixtureTrack::Subtitle => media.with_subtitle(
            "Accent".into(),
            Url::parse(&format!("http://fixture.example:{port}/accent.ass")).unwrap(),
            Some("en".into()),
        ),
    };
    DirectDownload::segmented_catalog(
        VariantRef {
            publication: PublicationRef {
                source: Source::Rutube,
                id: "0123456789abcdef0123456789abcdef".into(),
            },
            selector: VariantSelector::RutubeVideo,
        },
        title.into(),
        media,
        MediaTools {
            ffmpeg: tools.ffmpeg.clone(),
            ffprobe: tools.ffprobe.clone(),
        },
    )
    .unwrap()
}

async fn wait_for(
    updates: &mut tokio::sync::watch::Receiver<Vec<DownloadJob>>,
    id: &str,
    condition: impl Fn(&DownloadJob) -> bool,
) -> DownloadJob {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(job) = updates.borrow().iter().find(|job| job.id == id)
                && condition(job)
            {
                return job.clone();
            }
            updates.changed().await.expect("queue updates remain live");
        }
    })
    .await
    .expect("download reached expected state")
}

fn decode(ffmpeg: &Path, path: &Path) {
    let result = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .expect("qplay ffmpeg launches");
    assert!(
        result.status.success(),
        "decode {}: {}",
        path.display(),
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn serve(listener: TcpListener, media: PathBuf, requests: Arc<Mutex<Vec<String>>>) {
    loop {
        let (mut socket, _) = listener.accept().await.unwrap();
        let media = media.clone();
        let requests = Arc::clone(&requests);
        tokio::spawn(async move {
            let mut head = [0u8; 4096];
            let count = socket.read(&mut head).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&head[..count]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            requests.lock().unwrap().push(path.to_owned());
            let slow = path.starts_with("/slow/");
            let expired = path.starts_with("/expired/") && !path.ends_with("master.m3u8");
            let name = path.rsplit('/').next().unwrap_or("");
            let allowed = name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte));
            let bytes = if allowed {
                fs::read(media.join(name)).ok()
            } else {
                None
            };
            let status = if expired {
                "403 Forbidden"
            } else if bytes.is_some() {
                "200 OK"
            } else {
                "404 Not Found"
            };
            let body = if expired { None } else { bytes };
            let content_type = if name.ends_with(".m3u8") {
                "application/vnd.apple.mpegurl"
            } else if name.ends_with(".mpd") {
                "application/dash+xml"
            } else {
                "application/octet-stream"
            };
            let payload = body.as_deref().unwrap_or(b"");
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            if socket.write_all(header.as_bytes()).await.is_err() {
                return;
            }
            for chunk in payload.chunks(if slow { 512 } else { 64 * 1024 }) {
                if socket.write_all(chunk).await.is_err() {
                    break;
                }
                if slow {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().ok_or("pass fresh output directory")?);
    let fixture = PathBuf::from(args.next().ok_or("pass generated media directory")?);
    let tools = MediaTools {
        ffmpeg: PathBuf::from(args.next().ok_or("pass qplay ffmpeg path")?),
        ffprobe: PathBuf::from(args.next().ok_or("pass qplay ffprobe path")?),
    };
    if root.exists() || !fixture.join("manifest.mpd").is_file() {
        return Err("output must be fresh and generated DASH fixture must exist".into());
    }
    fs::write(
        fixture.join("master.m3u8"),
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"English, stereo\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=320x180,AUDIO=\"audio\"\nmissing.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=160x90,AUDIO=\"audio\"\nvideo.m3u8\n",
    )?;
    fs::write(
        fixture.join("live.m3u8"),
        "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nvideo-000.ts\n",
    )?;
    fs::write(
        fixture.join("drm.m3u8"),
        "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"key\"\n#EXTINF:1,\nvideo-000.ts\n#EXT-X-ENDLIST\n",
    )?;
    let dash = fs::read_to_string(fixture.join("manifest.mpd"))?;
    fs::write(
        fixture.join("dynamic.mpd"),
        dash.replace("type=\"static\"", "type=\"dynamic\""),
    )?;
    fs::write(
        fixture.join("protected.mpd"),
        dash.replacen(
            "<Representation",
            "<ContentProtection schemeIdUri=\"urn:uuid:test\" />\n<Representation",
            1,
        ),
    )?;
    fs::write(
        fixture.join("accent.ass"),
        "[Script Info]\nTitle: Generated subtitle fixture\nScriptType: v4.00+\nPlayResX: 160\nPlayResY: 90\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Accent,Arial,18,&H0000FFFF,&H0000FFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:03.00,Accent,,0,0,0,,Generated subtitle\n",
    )?;
    let profile = root.join("profile");
    let destination = root.join("downloads");
    fs::create_dir_all(&destination)?;
    let marker = destination.join("user-file.mkv");
    fs::write(&marker, b"unrelated user bytes")?;
    fs::write(destination.join("HLS.mkv"), b"existing user payload")?;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address: SocketAddr = listener.local_addr()?;
    audio::prepare(&fixture, &tools)?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn(serve(listener, fixture.clone(), Arc::clone(&requests)));
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve("fixture.example", address)
        .build()?;
    let store = Store(Arc::new(Mutex::new(Library::open(&profile)?)));
    let queue =
        DownloadQueue::open_fixture(store.clone(), destination.clone(), client.clone()).await?;
    let mut updates = queue.subscribe();

    for (name, path, track) in [
        ("HLS", "master.m3u8", FixtureTrack::None),
        ("DASH", "manifest.mpd", FixtureTrack::None),
        ("External", "video.mp4", FixtureTrack::Audio),
        ("Subtitles", "video.mp4", FixtureTrack::Subtitle),
    ] {
        let job = queue
            .enqueue(request(address.port(), path, name, &tools, track))
            .await?;
        let done = wait_for(&mut updates, &job.id, |job| {
            matches!(job.status, DownloadStatus::Complete | DownloadStatus::Error)
        })
        .await;
        assert_eq!(
            done.status,
            DownloadStatus::Complete,
            "{name}: {:?}",
            done.error
        );
        let path = done.path.expect("complete output");
        if name == "HLS" {
            assert_eq!(path, destination.join("HLS (2).mkv"));
        }
        decode(&tools.ffmpeg, &path);
        if name == "Subtitles" {
            let probe = Command::new(&tools.ffprobe)
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "s:0",
                    "-show_entries",
                    "stream=codec_name:stream_tags=title,language",
                    "-of",
                    "json",
                ])
                .arg(&path)
                .output()?;
            assert!(probe.status.success());
            let streams: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
            assert_eq!(streams["streams"][0]["codec_name"], "ass");
            assert_eq!(streams["streams"][0]["tags"]["title"], "Accent");
            assert_eq!(streams["streams"][0]["tags"]["language"], "en");
            let extracted = root.join("extracted.ass");
            let result = Command::new(&tools.ffmpeg)
                .args(["-y", "-v", "error", "-i"])
                .arg(&path)
                .args(["-map", "0:s:0", "-c:s", "copy", "-f", "ass"])
                .arg(&extracted)
                .output()?;
            assert!(result.status.success());
            assert!(fs::read_to_string(extracted)?.contains("Style: Accent"));
        }
    }
    assert_eq!(
        fs::read(destination.join("HLS.mkv"))?,
        b"existing user payload"
    );

    audio::verify(&queue, address.port(), &fixture, &tools).await?;
    assert!(
        !requests.lock().unwrap().iter().any(|path| {
            ["/voice-0", "/voice-2", "/voice-3"]
                .iter()
                .any(|prefix| path.starts_with(prefix))
        }),
        "Unselected HLS voices were downloaded"
    );

    for name in ["live.m3u8", "drm.m3u8", "dynamic.mpd", "protected.mpd"] {
        let job = queue
            .enqueue(request(
                address.port(),
                name,
                name,
                &tools,
                FixtureTrack::None,
            ))
            .await?;
        let failed = wait_for(&mut updates, &job.id, |job| {
            job.status == DownloadStatus::Error
        })
        .await;
        assert!(failed.path.is_none(), "{name} unexpectedly published");
    }

    let expired = queue
        .enqueue(request(
            address.port(),
            "expired/master.m3u8",
            "Expired",
            &tools,
            FixtureTrack::None,
        ))
        .await?;
    let failed = wait_for(&mut updates, &expired.id, |job| {
        job.status == DownloadStatus::Error
    })
    .await;
    assert!(failed.path.is_none());
    assert!(!destination.join("Expired.mkv").exists());
    queue
        .retry(
            &expired.id,
            request(
                address.port(),
                "master.m3u8",
                "Expired",
                &tools,
                FixtureTrack::None,
            ),
        )
        .await?;
    let retried = wait_for(&mut updates, &expired.id, |job| {
        job.status == DownloadStatus::Complete
    })
    .await;
    decode(&tools.ffmpeg, retried.path.as_deref().unwrap());

    let stopped = queue
        .enqueue(request(
            address.port(),
            "slow/master.m3u8",
            "Stopped",
            &tools,
            FixtureTrack::None,
        ))
        .await?;
    wait_for(&mut updates, &stopped.id, |job| job.bytes > 0).await;
    queue.cancel(&stopped.id).await?;
    wait_for(&mut updates, &stopped.id, |job| {
        job.status == DownloadStatus::Cancelled
    })
    .await;
    assert!(!destination.join("Stopped.mkv").exists());
    assert!(
        !destination
            .join(format!(".qbutt-staging/{}", stopped.id))
            .exists()
    );

    let interrupted = queue
        .enqueue(request(
            address.port(),
            "slow/master.m3u8",
            "Interrupted",
            &tools,
            FixtureTrack::None,
        ))
        .await?;
    wait_for(&mut updates, &interrupted.id, |job| job.bytes > 0).await;
    queue.close().await;
    let reopened = DownloadQueue::open_fixture(store, destination.clone(), client).await?;
    let mut updates = reopened.subscribe();
    let restored = updates
        .borrow()
        .iter()
        .find(|job| job.id == interrupted.id)
        .cloned()
        .unwrap();
    assert_eq!(restored.status, DownloadStatus::Error);
    assert!(
        !destination
            .join(format!(".qbutt-staging/{}", interrupted.id))
            .exists()
    );
    reopened
        .retry(
            &interrupted.id,
            request(
                address.port(),
                "master.m3u8",
                "Interrupted",
                &tools,
                FixtureTrack::None,
            ),
        )
        .await?;
    let completed = wait_for(&mut updates, &interrupted.id, |job| {
        job.status == DownloadStatus::Complete
    })
    .await;
    decode(&tools.ffmpeg, completed.path.as_deref().unwrap());
    assert_eq!(fs::read(marker)?, b"unrelated user bytes");
    reopened.close().await;
    server.abort();

    let database = fs::read(profile.join("content-library.sqlite"))?;
    assert!(
        !database
            .windows(b"fixture.example".len())
            .any(|bytes| bytes == b"fixture.example")
    );
    println!(
        "PASS native_segmented hls=1 dash=1 external_audio=1 ass_style=1 mux_decode=6 collision=1 expired_retry=1 cancel=1 restart_retry=1 live_rejected=2 drm_rejected=2 descriptor_urls_persisted=0"
    );
    Ok(())
}
