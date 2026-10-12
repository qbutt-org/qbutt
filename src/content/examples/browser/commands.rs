use qbutt_content::browser::{BrowserChoice, BrowserCommand, BrowserError, BrowserHandle};
use serde_json::{Value, json};
use tokio::net::windows::named_pipe::NamedPipeClient;

use super::fixture::{FixtureResult, cookies, exchange, read, request, write};

pub(super) async fn command_replies(
    handle: &BrowserHandle,
    edge: &mut NamedPipeClient,
) -> FixtureResult {
    let cookie = json!({
        "name": "session",
        "value": "ephemeral-fixture-secret",
        "domain": ".fixture.example",
        "path": "/",
        "secure": true,
        "hostOnly": false,
        "httpOnly": true,
        "sameSite": "lax",
        "expirationDate": 2000000000,
    });
    let reply = exchange(
        handle,
        edge,
        json!({"method":"cookies","url":"https://fixture.example/"}),
        json!([cookie.clone()]),
        None,
    )
    .await?;
    assert_eq!(reply[0]["value"], "ephemeral-fixture-secret");
    let mut foreign_cookie = cookie.clone();
    foreign_cookie["domain"] = json!("other.example");
    exchange(
        handle,
        edge,
        json!({"method":"cookies","url":"https://fixture.example/"}),
        json!([foreign_cookie]),
        Some(BrowserError::InvalidResponse),
    )
    .await?;
    let mut foreign_path = cookie.clone();
    foreign_path["path"] = json!("/another-publication");
    exchange(
        handle,
        edge,
        json!({"method":"cookies","url":"https://fixture.example/"}),
        json!([foreign_path]),
        Some(BrowserError::InvalidResponse),
    )
    .await?;
    exchange(
        handle,
        edge,
        json!({"method":"cookies","url":"https://fixture.example/"}),
        Value::Array(vec![cookie; 513]),
        Some(BrowserError::InvalidResponse),
    )
    .await?;
    let fetch = json!({
        "method": "fetch",
        "url": "https://fixture.example/api",
        "init": {
            "method": "POST",
            "headers": {"Content-Type": "application/json"},
            "body": "{}",
        },
    });
    let response = json!({
        "status": 200,
        "url": "https://fixture.example/api",
        "headers": {"content-type": "application/json"},
        "body": "{\"ok\":true}",
    });
    exchange(handle, edge, fetch.clone(), response.clone(), None).await?;
    for (field, value) in [
        ("status", json!(999)),
        ("url", json!("http://192.168.1.1/")),
        ("body", json!("x".repeat(512 * 1024 + 1))),
        ("headers", json!({"x-test":"value\r\nInjected: true"})),
    ] {
        let mut invalid = response.clone();
        invalid[field] = value;
        exchange(
            handle,
            edge,
            fetch.clone(),
            invalid,
            Some(BrowserError::InvalidResponse),
        )
        .await?;
    }
    let resolve = json!({
        "method": "resolve",
        "url": "https://fixture.example/player",
        "kind": "video",
        "referrer": "https://fixture.example/title",
        "selection": {
            "episode": "7",
            "season": "2",
            "translationId": "13",
            "player": "VK",
        },
    });
    let media = json!({
        "kind": "video",
        "resolver": "direct",
        "url": "https://media.example/video.m3u8?key=ephemeral-fixture-secret",
        "selection": resolve["selection"],
        "headers": {"Cookie": "session=ephemeral-fixture-secret"},
        "audioUrl": "https://media.example/audio.m3u8",
        "audioTrack": {"title": "Original", "language": "ja", "channels": 2},
        "subtitles": [{
            "title": "Russian",
            "url": "https://media.example/ru.vtt",
            "language": "ru",
        }],
    });
    let reply = exchange(handle, edge, resolve.clone(), media.clone(), None).await?;
    assert_eq!(reply["audioTrack"]["channels"], 2);
    assert_eq!(reply["subtitles"][0]["language"], "ru");
    let mut wrong_selection = media.clone();
    wrong_selection["selection"]["episode"] = json!("8");
    exchange(
        handle,
        edge,
        resolve.clone(),
        wrong_selection,
        Some(BrowserError::SelectionChanged),
    )
    .await?;
    for (field, value) in [
        ("kind", json!("file")),
        ("resolver", json!("yt-dlp")),
        ("audioUrl", json!("http://10.0.0.1/audio")),
        (
            "subtitles",
            json!([{"title":"local","url":"file:///C:/Windows/file.vtt"}]),
        ),
    ] {
        let mut invalid = media.clone();
        invalid[field] = value;
        exchange(
            handle,
            edge,
            resolve.clone(),
            invalid,
            Some(BrowserError::InvalidResponse),
        )
        .await?;
    }
    let capture = json!({"captureId":"00000000-0000-4000-8000-000000000001"});
    exchange(
        handle,
        edge,
        json!({
            "method": "resolve",
            "url": "https://fixture.example/player",
            "selection": capture,
        }),
        json!({
            "kind": "video",
            "resolver": "yt-dlp",
            "url": "https://fixture.example/player",
            "selection": capture,
        }),
        None,
    )
    .await?;
    exchange(
        handle,
        edge,
        json!({"method":"resolve","url":"https://fixture.example/download","kind":"file"}),
        json!({"kind":"torrent","resolver":"direct","url":"https://fixture.example/file.torrent"}),
        None,
    )
    .await?;
    let search = serde_json::to_value(BrowserCommand::vk_search("fixture anime".into())?)?;
    let videos = json!({
        "query": "fixture anime",
        "videos": [{
            "id": "-123_456",
            "title": "Episode 7",
            "url": "https://vkvideo.ru/video-123_456",
            "author": "Fixture",
            "duration": 1440,
            "cover": "https://cover.example/image.jpg",
        }],
    });
    exchange(handle, edge, search.clone(), videos.clone(), None).await?;
    let mut wrong_query = videos.clone();
    wrong_query["query"] = json!("other");
    exchange(
        handle,
        edge,
        search.clone(),
        wrong_query,
        Some(BrowserError::InvalidResponse),
    )
    .await?;
    let mut wrong_video = videos;
    wrong_video["videos"][0]["url"] = json!("https://vkvideo.ru/video-123_999");
    exchange(
        handle,
        edge,
        search,
        wrong_video,
        Some(BrowserError::InvalidResponse),
    )
    .await?;
    let pending = request(handle, BrowserChoice::Edge, cookies());
    let wire = read(edge).await?;
    write(
        edge,
        &json!({
            "id": wire["id"],
            "error": {
                "code": "verification_required",
                "message": "ephemeral-fixture-secret https://fixture.example/?token=secret",
                "url": "https://fixture.example/?token=secret",
            },
        }),
    )
    .await?;
    let error = match pending.await? {
        Err(error) => error,
        Ok(_) => panic!("Expected remote failure"),
    };
    assert_eq!(error, BrowserError::VerificationRequired);
    assert!(!format!("{error:?} {error}").contains("secret"));
    Ok(())
}
