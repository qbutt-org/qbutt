//! Generated protected HLS, decoder/seek and cancellation acceptance.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use url::Url;

use super::media::{MediaGateway, MediaGatewayError};
use super::{AcceptanceResult, PlaybackMedia, local_get};

type RequestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub(super) async fn run(directory: &Path, ffmpeg: &Path) -> AcceptanceResult<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let origin: Url =
        format!("http://fixture.example:{}/", listener.local_addr()?.port()).parse()?;
    let playlist = std::fs::read_to_string(directory.join("playlist.m3u8"))?;
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("segment") && name.ends_with(".ts")
            || name.starts_with("fragmented") && name.ends_with(".m4s")
            || name == "init.mp4"
        {
            files.insert(format!("/{name}"), std::fs::read(entry.path())?);
        }
    }
    assert!(files.len() > 1, "Expected generated VOD segments");
    files.insert("/selected.m3u8".into(), playlist.as_bytes().to_vec());
    files.insert(
        "/fragmented.m3u8".into(),
        std::fs::read(directory.join("fragmented.m3u8"))?,
    );
    files.insert("/large.m3u8".into(), vec![b'x'; 512 * 1024 + 1]);
    files.insert(
        "/private.m3u8".into(),
        b"#EXTM3U\n#EXTINF:1,\nhttp://127.0.0.1/forbidden.ts\n#EXT-X-ENDLIST\n".to_vec(),
    );
    files.insert(
        "/master.m3u8".into(),
        b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nselected.m3u8\n".to_vec(),
    );
    for (name, body) in [
        ("live", playlist.replace("#EXT-X-ENDLIST", "")),
        (
            "encrypted",
            playlist.replace(
                "#EXTM3U",
                "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"",
            ),
        ),
        (
            "ranged",
            playlist.replace("#EXTINF:", "#EXT-X-BYTERANGE:10@0\n#EXTINF:"),
        ),
        (
            "malformed",
            playlist.replace("#EXTINF:1.000000,", "#EXTINF:NaN,"),
        ),
        (
            "redirect",
            playlist.replace("segment000.ts", "redirect-segment"),
        ),
        (
            "private-redirect",
            playlist.replace("segment000.ts", "private-redirect-segment"),
        ),
        (
            "pending",
            playlist.replace("segment000.ts", "stall-segment"),
        ),
        (
            "map-ranged",
            playlist.replace(
                "#EXTM3U",
                "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"10@0\"",
            ),
        ),
    ] {
        files.insert(format!("/{name}.m3u8"), body.into_bytes());
    }
    let files = Arc::new(files);
    let source_hits = Arc::new(AtomicUsize::new(0));
    let stalls = Arc::new(AtomicUsize::new(0));
    let (stop, mut stopped) = oneshot::channel();
    let server = tokio::spawn({
        let (files, origin, hits, stalls) =
            (files, origin.clone(), source_hits.clone(), stalls.clone());
        async move {
            let mut requests: JoinSet<RequestResult> = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    completed = requests.join_next(), if !requests.is_empty() => {
                        completed.unwrap().unwrap().unwrap();
                    }
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        requests.spawn(respond(socket, origin.clone(), files.clone(), hits.clone(), stalls.clone()));
                    }
                }
            }
            requests.shutdown().await;
        }
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()?;
    let descriptor = |path: &str| PlaybackMedia {
        url: origin.join(path).unwrap(),
        headers: [
            ("Cookie".into(), "fixture=session".into()),
            ("Authorization".into(), "Bearer fixture".into()),
            (
                "Referer".into(),
                "https://alloha.yani.tv/?token=fixture".into(),
            ),
            ("Origin".into(), "https://alloha.yani.tv".into()),
        ]
        .into(),
        audio_url: None,
        audio_track: None,
        video_height: None,
        subtitles: Vec::new(),
    };
    for path in ["selected.m3u8", "redirect.m3u8", "fragmented.m3u8"] {
        let (gateway, prepared) = MediaGateway::prepare(descriptor(path)).await?;
        assert!(
            prepared.muxed_mp4_url().is_none(),
            "HLS was offered as an MP4 download"
        );
        let response = local_get(&client, &prepared.url, None).await?;
        assert_eq!(response.status(), 200);
        let rewritten = response.text().await?;
        assert!(!rewritten.contains("fixture.example"));
        assert!(!rewritten.contains("child.example"));
        let children =
            super::media::hls::media(super::media::hls::lines(&rewritten).unwrap()).unwrap();
        for line in children {
            if let super::media::hls::Line::Resource { uri, .. } = line {
                let child: Url = uri.parse()?;
                assert_eq!(child.origin(), prepared.url.origin());
                let response = local_get(&client, &child, Some("bytes=1-3")).await?;
                assert_eq!(response.status(), 206);
                assert!(
                    response.headers()["content-range"]
                        .to_str()?
                        .starts_with("bytes 1-3/")
                );
                assert_eq!(response.bytes().await?.len(), 3);
                break;
            }
        }
        assert_eq!(
            local_get(&client, &prepared.url, Some("bytes=0-"))
                .await?
                .status(),
            206
        );
        let tail = local_get(&client, &prepared.url, Some("bytes=-8")).await?;
        assert_eq!(tail.status(), 206);
        assert_eq!(
            tail.bytes().await?.as_ref(),
            &rewritten.as_bytes()[rewritten.len() - 8..]
        );
        assert_eq!(
            local_get(&client, &prepared.url, Some("bytes=18446744073709551615-"))
                .await?
                .status(),
            416
        );
        decode(ffmpeg, &prepared.url, false).await?;
        decode(ffmpeg, &prepared.url, true).await?;
        gateway.close().await;
        assert!(client.get(prepared.url).send().await.is_err());
    }
    for (path, expected) in [
        ("master.m3u8", "unsupported"),
        ("live.m3u8", "unsupported"),
        ("encrypted.m3u8", "unsupported"),
        ("ranged.m3u8", "unsupported"),
        ("map-ranged.m3u8", "unsupported"),
        ("private.m3u8", "invalid"),
        ("large.m3u8", "invalid"),
        ("malformed.m3u8", "invalid"),
    ] {
        let before = source_hits.load(Ordering::SeqCst);
        let result = MediaGateway::prepare(descriptor(path)).await;
        assert!(
            matches!(
                (result, expected),
                (Err(MediaGatewayError::UnsupportedManifest), "unsupported")
                    | (Err(MediaGatewayError::InvalidDescriptor), "invalid")
            ),
            "Incorrect HLS refusal for {path}"
        );
        assert_eq!(
            source_hits.load(Ordering::SeqCst),
            before,
            "Rejected manifest acquired payload"
        );
    }
    let (gateway, prepared) = MediaGateway::prepare(descriptor("private-redirect.m3u8")).await?;
    let body = client
        .get(prepared.url.clone())
        .send()
        .await?
        .text()
        .await?;
    let child: Url = body
        .lines()
        .find(|line| !line.starts_with('#'))
        .unwrap()
        .parse()?;
    assert_eq!(client.get(child).send().await?.status(), 502);
    gateway.close().await;

    let mut preparing = Box::pin(MediaGateway::prepare(descriptor("stall.m3u8")));
    tokio::select! {
        _ = &mut preparing => panic!("Stalled manifest unexpectedly completed"),
        _ = wait_stall(&stalls, 1) => {}
    }
    drop(preparing);
    wait_stall(&stalls, 0).await;
    let (gateway, prepared) = MediaGateway::prepare(descriptor("pending.m3u8")).await?;
    let body = client
        .get(prepared.url.clone())
        .send()
        .await?
        .text()
        .await?;
    let child: Url = body
        .lines()
        .find(|line| !line.starts_with('#'))
        .unwrap()
        .parse()?;
    let request = tokio::spawn({
        let client = client.clone();
        async move { client.get(child).send().await }
    });
    wait_stall(&stalls, 1).await;
    gateway.close().await;
    assert!(request.await?.is_err());
    wait_stall(&stalls, 0).await;
    let _ = stop.send(());
    server.await?;
    println!(
        "PASS media-gateway-hls clear_vod_ts_fmp4=decoded audio=decoded seek=decoded map=decoded range=verified headers=scoped redirects=guarded master_live_encrypted_ranged_private_large_malformed=rejected prepare_cancel=closed playback_cancel=drained retirement=closed"
    );
    Ok(())
}

async fn wait_stall(stalls: &AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while stalls.load(Ordering::SeqCst) != expected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Source socket did not reach its expected state");
}

async fn decode(ffmpeg: &Path, url: &Url, seek: bool) -> AcceptanceResult<()> {
    let mut command = tokio::process::Command::new(ffmpeg);
    command.args([
        "-nostdin",
        "-v",
        "error",
        "-protocol_whitelist",
        "file,http,tcp",
    ]);
    if seek {
        command.args(["-ss", "2"]);
    }
    command.args([
        "-i",
        url.as_str(),
        "-map",
        "0:v:0",
        "-map",
        "0:a:0",
        "-progress",
        "pipe:1",
        "-f",
        "null",
        "-",
    ]);
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output()).await??;
    assert!(
        output.status.success(),
        "Generated HLS decode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let progress = String::from_utf8_lossy(&output.stdout);
    assert!(progress.contains("progress=end"));
    let frames = progress
        .lines()
        .filter_map(|line| line.strip_prefix("frame="))
        .filter_map(|value| value.trim().parse::<u64>().ok())
        .last()
        .unwrap();
    let duration = progress
        .lines()
        .filter_map(|line| line.strip_prefix("out_time_us="))
        .filter_map(|value| value.parse::<u64>().ok())
        .last()
        .unwrap();
    assert!(frames > 0);
    assert!(if seek {
        (1_000_000..3_000_000).contains(&duration)
    } else {
        duration >= 3_000_000
    });
    println!(
        "PASS prepared-hls-decode seek_seconds={} frames={frames} output_microseconds={duration}",
        if seek { 2 } else { 0 }
    );
    Ok(())
}

async fn respond(
    mut socket: TcpStream,
    origin: Url,
    files: Arc<BTreeMap<String, Vec<u8>>>,
    hits: Arc<AtomicUsize>,
    stalls: Arc<AtomicUsize>,
) -> RequestResult {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
        assert!(head.len() < 16 * 1024);
    }
    let head = String::from_utf8(head)?;
    let headers = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let child = headers
        .get("host")
        .is_some_and(|host| host.starts_with("child.example:"));
    assert_eq!(
        headers.get("cookie").map(String::as_str),
        (!child).then_some("fixture=session")
    );
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        (!child).then_some("Bearer fixture")
    );
    assert_eq!(
        headers.get("referer").map(String::as_str),
        Some("https://alloha.yani.tv/")
    );
    assert_eq!(
        headers.get("origin").map(String::as_str),
        Some("https://alloha.yani.tv")
    );
    let path = head.split_whitespace().nth(1).unwrap();
    if path == "/stall.m3u8" || path == "/stall-segment" {
        stalls.fetch_add(1, Ordering::SeqCst);
        let result = socket.read(&mut byte).await;
        stalls.fetch_sub(1, Ordering::SeqCst);
        result?;
        return Ok(());
    }
    if path == "/redirect-segment" || path == "/private-redirect-segment" {
        let mut target = origin.join("segment000.ts")?;
        target.set_host(Some(if path == "/redirect-segment" {
            "child.example"
        } else {
            "127.0.0.1"
        }))?;
        socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await?;
        return Ok(());
    }
    let body = files.get(path).ok_or("Unknown generated resource")?;
    let playlist = path.ends_with(".m3u8");
    if !playlist {
        hits.fetch_add(1, Ordering::SeqCst);
    }
    let (status, range, bytes) = if headers
        .get("range")
        .is_some_and(|range| range == "bytes=1-3")
    {
        (
            "206 Partial Content",
            format!("Content-Range: bytes 1-3/{}\r\n", body.len()),
            &body[1..4],
        )
    } else {
        ("200 OK", String::new(), body.as_slice())
    };
    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {}\r\n{range}Connection: close\r\n\r\n", bytes.len(), if playlist { "application/vnd.apple.mpegurl" } else { "video/mp2t" }).as_bytes()).await?;
    if !head.starts_with("HEAD ") {
        socket.write_all(bytes).await?;
    }
    Ok(())
}
