#![cfg(windows)]

#[path = "browser/fixture.rs"]
mod fixture;

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::browser::{BrowserBroker, BrowserChoice, BrowserError};
use qbutt_content::{
    Category, Error, Library, LibraryView, PublicationRef, ResolvedContent, SharedWork, Source,
    VariantPreferences, VkVideo, WorkAssociation, WorkGroup,
};
use serde_json::{Value, json};

use fixture::{extension, read, wait_status, write};

type FixtureResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> FixtureResult {
    let directory = std::env::temp_dir().join(format!(
        "qbutt-vkvideo-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&directory)?;
    let discovery = directory.join("browser-bridge.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let browser = broker.handle();
    let provider = VkVideo;

    assert!(matches!(
        provider
            .search(" bad ", Category::Video, &browser, BrowserChoice::Edge)
            .await,
        Err(Error::InvalidQuery)
    ));
    let mut edge = extension(&discovery, "edge", "generated VK fixture", true).await?;
    wait_status(&browser, |status| status.connections.len() == 1).await?;

    let query = "студсовет косяки";
    let pending = tokio::spawn({
        let browser = browser.clone();
        async move {
            VkVideo
                .search(query, Category::Video, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    assert_eq!(command["method"], "vk-search");
    assert_eq!(command["query"], query);
    assert_eq!(
        command["url"],
        "https://vkvideo.ru/?q=%D1%81%D1%82%D1%83%D0%B4%D1%81%D0%BE%D0%B2%D0%B5%D1%82+%D0%BA%D0%BE%D1%81%D1%8F%D0%BA%D0%B8"
    );
    respond(
        &mut edge,
        &command,
        json!({
            "query": query,
            "videos": [
                video("-900001_1001", "Даже у студсовета есть косяки! 1 серия"),
                video("-900001_1002", "Даже у студсовета есть косяки! 2 серия"),
                video("-900002_1003", "Студсовет косяки! Трейлер"),
                video("-900003_1004", "Нерелевантная комедия")
            ]
        }),
    )
    .await?;
    let works = pending.await??;
    assert_eq!(works.len(), 2);
    assert!(works.iter().all(|work| work.category == Category::Video));
    assert!(works.iter().all(|work| work.cover.is_none()));
    assert_eq!(works[0].reference.id, "-900001_1001");
    assert_eq!(works[1].reference.id, "-900001_1002");
    assert_ne!(works[0].reference, works[1].reference);
    assert_eq!(
        works[0].variants[0].reference.publication,
        works[0].reference
    );
    let mut association = WorkAssociation::default();
    for work in &works {
        association.update(work.clone());
    }
    assert_eq!(association.groups().len(), 2);

    let publication = PublicationRef {
        source: Source::VkVideo,
        id: "-900001_1001".into(),
    };
    let inspect = tokio::spawn({
        let browser = browser.clone();
        let publication = publication.clone();
        async move {
            VkVideo
                .inspect(&publication, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    assert_eq!(command["method"], "resolve");
    assert_eq!(command["url"], "https://vkvideo.ru/video-900001_1001");
    respond(
        &mut edge,
        &command,
        json!({
            "kind": "video", "resolver": "direct", "url": "https://cdn.example/fixture.mp4",
            "sourceUrl": "https://vkvideo.ru/video-900001_1001", "headers": {},
            "selection": {}, "title": "Студсовет. Серия 1"
        }),
    )
    .await?;
    let inspected = inspect.await??;
    assert_eq!(inspected.reference, publication);
    assert_eq!(inspected.title, "Студсовет. Серия 1");
    assert_eq!(inspected.category, Category::Video);

    let selected = works[0].variants[0].reference.clone();
    assert!(Source::ALL.contains(&Source::VkVideo));
    assert_eq!(Source::from_id("vkvideo"), Some(Source::VkVideo));
    let shared = SharedWork::new(vec![publication.clone()], Some(selected.clone()))?;
    let link = shared.to_url()?;
    assert_eq!(SharedWork::from_url(&link)?.preferred(), Some(&selected));
    let profile = directory.join("profile");
    fs::create_dir(&profile)?;
    let mut library = Library::open(&profile)?;
    library.set_favorite(&WorkGroup::single(works[0].clone()), true)?;
    let group = WorkGroup::single(inspected);
    library.record_opened(&group, selected.clone(), VariantPreferences::default())?;
    drop(library);
    let library = Library::open(&profile)?;
    let history = library.entries(LibraryView::History)?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].last_opened.as_ref().unwrap().variant, selected);
    let stored = serde_json::to_string(&history)?;
    assert!(!stored.contains("cdn.example") && !stored.contains("headers"));
    drop(library);
    let resolved = tokio::spawn({
        let browser = browser.clone();
        let selected = selected.clone();
        async move {
            VkVideo
                .resolve(&selected, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    assert_eq!(command["method"], "resolve");
    assert_eq!(command["url"], "https://vkvideo.ru/video-900001_1001");
    respond(
        &mut edge,
        &command,
        json!({
            "kind": "video", "resolver": "direct", "url": "https://cdn.example/fixture.mp4",
            "sourceUrl": "https://vkvideo.ru/video-900001_1001", "headers": {},
            "selection": {}, "title": "Студсовет. Серия 1"
        }),
    )
    .await?;
    assert!(matches!(
        resolved.await??,
        ResolvedContent::BrowserVideo { reference, title, .. }
            if reference == selected && title == "Студсовет. Серия 1"
    ));

    let stale = tokio::spawn({
        let browser = browser.clone();
        let selected = selected.clone();
        async move {
            VkVideo
                .resolve(&selected, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    respond(
        &mut edge,
        &command,
        json!({
            "kind": "video", "resolver": "direct", "url": "https://cdn.example/fixture.mp4",
            "sourceUrl": "https://vkvideo.ru/video-900001_1002", "headers": {},
            "selection": {}, "title": "Another episode"
        }),
    )
    .await?;
    assert!(matches!(stale.await?, Err(Error::VariantUnavailable)));

    let wrong_query = tokio::spawn({
        let browser = browser.clone();
        async move {
            VkVideo
                .search(query, Category::Video, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    respond(&mut edge, &command, json!({"query": "other", "videos": []})).await?;
    assert!(matches!(
        wrong_query.await?,
        Err(Error::BrowserSession(BrowserError::InvalidResponse))
    ));

    let unsafe_cover = tokio::spawn({
        let browser = browser.clone();
        async move {
            VkVideo
                .search(query, Category::Video, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    let mut unsafe_video = video("-900001_1001", "Студсовет косяки");
    unsafe_video["cover"] = json!("http://127.0.0.1/secret");
    respond(
        &mut edge,
        &command,
        json!({"query": query, "videos": [unsafe_video]}),
    )
    .await?;
    assert!(matches!(
        unsafe_cover.await?,
        Err(Error::BrowserSession(BrowserError::InvalidResponse))
    ));

    let challenge = tokio::spawn({
        let browser = browser.clone();
        async move {
            VkVideo
                .search(query, Category::Video, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    write(
        &mut edge,
        &json!({
            "id": command["id"],
            "error": {"code": "browser_check_required", "message": "generated check"}
        }),
    )
    .await?;
    assert!(matches!(challenge.await?, Err(Error::Challenge)));

    let timed_out = tokio::spawn({
        let browser = browser.clone();
        async move {
            VkVideo
                .search(query, Category::Video, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    write(
        &mut edge,
        &json!({
            "id": command["id"],
            "error": {"code": "browser_timeout", "message": "generated deadline"}
        }),
    )
    .await?;
    assert!(matches!(timed_out.await?, Err(Error::Timeout)));

    let chrome = extension(&discovery, "chrome", "generated old browser", false).await?;
    wait_status(&browser, |status| status.connections.len() == 2).await?;
    assert!(matches!(
        provider
            .search(query, Category::Video, &browser, BrowserChoice::Auto)
            .await,
        Err(Error::BrowserChoiceRequired)
    ));
    assert!(matches!(
        provider
            .search(query, Category::Video, &browser, BrowserChoice::Chrome)
            .await,
        Err(Error::BrowserSession(BrowserError::UpdateRequired))
    ));

    let canceled = tokio::spawn({
        let browser = browser.clone();
        async move {
            VkVideo
                .search(query, Category::Video, &browser, BrowserChoice::Edge)
                .await
        }
    });
    let command = read(&mut edge).await?;
    canceled.abort();
    assert!(canceled.await.is_err());
    let cancel = read(&mut edge).await?;
    assert_eq!(cancel["method"], "cancel");
    assert_eq!(cancel["id"], command["id"]);

    drop(chrome);
    drop(edge);
    broker.shutdown().await?;
    assert!(!discovery.exists());
    fs::remove_file(profile.join("content-library.sqlite"))?;
    fs::remove_dir(&profile)?;
    fs::remove_dir(&directory)?;
    println!(
        "PASS vkvideo_search=ok exact_identity=ok inspect=ok response_validation=ok browser_choice=ok cancellation=ok"
    );
    Ok(())
}

fn video(id: &str, title: &str) -> Value {
    json!({
        "id": id,
        "title": title,
        "url": format!("https://vkvideo.ru/video{id}"),
        "cover": "https://thumb.example/frame.jpg?quality=96&size=320x180"
    })
}

async fn respond(
    peer: &mut tokio::net::windows::named_pipe::NamedPipeClient,
    command: &Value,
    result: Value,
) -> FixtureResult {
    write(peer, &json!({"id": command["id"], "result": result})).await
}
