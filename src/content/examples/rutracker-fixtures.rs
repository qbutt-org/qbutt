//! Generated legal CP1251 traffic through the production provider and guarded
//! HTTPS transport. Only this executable can inject the fixture's TLS client.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
pub use qbutt_content::webview;
#[path = "../src/browser/mod.rs"]
pub mod browser;

#[path = "../src/limetorrents.rs"]
mod limetorrents;
#[path = "../src/torrent.rs"]
mod torrent;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[allow(dead_code)]
mod rutracker {
    include!("../src/rutracker.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> RuTracker {
        RuTracker {
            http,
            origin,
            webview: std::sync::OnceLock::new(),
            clearance: std::sync::Mutex::new(None),
        }
    }
}

#[path = "browser/fixture.rs"]
#[allow(dead_code)]
mod browser_fixture;
#[path = "rutracker/fixture.rs"]
mod fixture;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use browser::{BrowserBroker, BrowserChoice, BrowserError};
use serde_json::json;
use url::Url;

use fixture::{Fixture, HASH};

type CheckResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> CheckResult {
    if std::env::args().nth(1).as_deref() == Some("--stdio") {
        return webview_fixture();
    }
    if let Some(path) = std::env::args().nth(1) {
        let bytes = std::fs::read(path)?;
        let body = rutracker::document::decode(&bytes, Some("text/html; charset=Windows-1251"))?;
        let map = rutracker::taxonomy::categories(&body)?;
        assert!(map.len() > 1000);
        for (forum, category) in [
            (7, Category::Movies),
            (2100, Category::Tv),
            (1386, Category::Anime),
            (899, Category::Games),
            (1016, Category::Software),
            (2038, Category::Books),
            (862, Category::Manga),
        ] {
            assert_eq!(map.get(&forum), Some(&Some(category)));
        }
        println!("PASS live_public_map=parsed forums={}", map.len());
        return Ok(());
    }
    let fixture = Fixture::start().await?;
    let source = rutracker::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let choice = BrowserChoice::Auto;
    let results = source
        .search("Синтел & legal", Category::Anime, None, choice)
        .await?;
    assert_eq!(results.len(), 1);
    let found = &results[0];
    assert_eq!(found.title, "Синтел [версия 2] & legal");
    assert_eq!(found.reference.id, "42");
    assert!(found.variants.is_empty());
    assert!(found.needs_inspection);
    {
        let requests = fixture.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|target| target.contains("nm=%D1%E8%ED%F2%E5%EB+%26+legal"))
        );
    }

    let work = source.inspect(&found.reference, None, choice).await?;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.variants[0].info_hash.as_deref(), Some(HASH));
    assert_eq!(work.variants[0].size, Some(734_003_200));
    assert_eq!(work.variants[0].seeders, Some(12));
    assert_eq!(
        work.description.as_deref(),
        Some("Созданный fixture для общественного достояния")
    );
    assert!(!work.needs_inspection);
    match source
        .resolve(&work.variants[0].reference, None, choice)
        .await?
    {
        ResolvedContent::Torrent {
            reference, magnet, ..
        } => {
            assert_eq!(reference, work.variants[0].reference);
            assert!(
                magnet
                    .query_pairs()
                    .any(|(name, value)| name == "tr" && value == "https://tracker.example/fresh")
            );
        }
        _ => panic!("expected exact torrent"),
    }
    for category in [
        Category::Movies,
        Category::Tv,
        Category::Games,
        Category::Software,
        Category::Books,
        Category::Manga,
    ] {
        let rows = source.search("generated", category, None, choice).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].category, category);
    }
    let before = fixture.hits();
    for query in ["", "bad\nvalue", "😀", &"x".repeat(501)] {
        assert!(matches!(
            source.search(query, Category::Anime, None, choice).await,
            Err(Error::InvalidQuery)
        ));
    }
    assert_eq!(fixture.hits(), before);

    fixture.case("empty");
    assert!(
        source
            .search("generated", Category::Anime, None, choice)
            .await?
            .is_empty()
    );
    for case in [
        "missing-href",
        "missing-topic-id",
        "row-id",
        "duplicate",
        "foreign-topic",
        "unknown-forum",
        "malformed-empty",
        "mime",
        "encoding",
        "charset",
        "redirect",
    ] {
        fixture.case(case);
        assert!(
            source
                .search("generated", Category::Anime, None, choice)
                .await
                .is_err(),
            "accepted {case}"
        );
    }
    fixture.case("huge");
    assert!(matches!(
        source
            .search("generated", Category::Anime, None, choice)
            .await,
        Err(Error::ResponseTooLarge)
    ));
    fixture.case("delay");
    assert!(matches!(
        source
            .search("generated", Category::Anime, None, choice)
            .await,
        Err(Error::Timeout)
    ));
    fixture.case("cancel");
    let mut pending = Box::pin(source.search("generated", Category::Anime, None, choice));
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(2), fixture.started.notified()) => result?,
        _ = &mut pending => panic!("cancel fixture completed early"),
    }
    drop(pending);
    tokio::time::timeout(Duration::from_secs(2), fixture.closed.notified()).await?;

    for case in ["login", "challenge", "input-recaptcha", "utf8-challenge"] {
        fixture.case(case);
        let error = source
            .search("generated", Category::Anime, None, choice)
            .await
            .unwrap_err();
        assert!(matches!(
            (case, error),
            ("login", Error::LoginRequired)
                | (_, Error::SourceCheck(webview::WebViewError::Unavailable))
        ));
    }
    for case in [
        "changed-hash",
        "changed-topic",
        "changed-download",
        "bad-magnet",
        "bad-size",
        "bad-seeders",
        "torrent-only",
        "gone",
    ] {
        fixture.case(case);
        let error = source
            .resolve(&work.variants[0].reference, None, choice)
            .await
            .err()
            .unwrap();
        match case {
            "torrent-only" => assert!(matches!(error, Error::UnsupportedTorrent)),
            "bad-magnet" | "bad-size" | "bad-seeders" => {
                assert!(matches!(error, Error::InvalidResponse(_)))
            }
            _ => assert!(matches!(error, Error::VariantUnavailable)),
        }
    }
    browser_sessions(&fixture, &source, &work).await?;
    webview_sessions(&fixture).await?;
    library(&work)?;
    fixture.stop().await?;
    println!(
        "PASS rutracker_cp1251_request_response=ok source_ancestry=7 exact_topic=ok fresh_magnet=ok replaced_hash=rejected torrent_only=unsupported"
    );
    println!(
        "PASS malformed_metadata=rejected challenge_login=typed connected_session_retry=ok unavailable_session=original browser_choice=typed cancellation=closed sqlite=redacted"
    );
    Ok(())
}

fn webview_fixture() -> CheckResult {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let request: serde_json::Value = serde_json::from_str(&line)?;
    assert!(request["mode"] == "silent" || request["mode"] == "interactive");
    let profile = Path::new(
        request["profileDirectory"]
            .as_str()
            .expect("fixture profile"),
    );
    assert_eq!(
        profile.parent().unwrap().file_name().unwrap().to_str(),
        Some("web-session")
    );
    let url = request["url"].as_str().expect("fixture URL");
    let address = Url::parse(url)?;
    if request["mode"] == "interactive" {
        assert_eq!(address.path(), "/forum/index.php");
    }
    let query = address.query_pairs().find(|(name, _)| name == "nm");
    let mode = query.as_ref().map(|(_, value)| value.as_ref());
    let response = match mode {
        Some("manual" | "login-required") => json!({
            "id":request["id"], "error":{"code":"browser_check_required"}
        }),
        Some("fault") => json!({
            "id":request["id"], "error":{"code":"webview_failed"}
        }),
        _ => json!({
            "id":request["id"],
            "session":{
                "url":url,
                "userAgent":"fixture-webview",
                "cookies":[{
                    "name":"clearance", "value":"fixture-pass", "domain":"fixture.example",
                    "path":"/", "secure":true, "httpOnly":true, "hostOnly":true
                }]
            }
        }),
    };
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

async fn webview_sessions(fixture: &Fixture) -> CheckResult {
    let directory = std::env::temp_dir().join(format!(
        "qbutt-rutracker-webview-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let app_dir = directory.join("app");
    std::fs::create_dir_all(&app_dir)?;
    std::fs::hard_link(std::env::current_exe()?, app_dir.join("qbutt-webview.exe"))?;
    fixture.case("auto-challenge");

    for query in ["generated", "persistent", "manual", "fault"] {
        let source = rutracker::fixture(
            http::fixture(fixture.client.clone(), fixture.origin.clone()),
            fixture.origin.clone(),
        );
        source.initialize_webview(directory.clone(), app_dir.clone());
        let before = fixture.requests.lock().unwrap().len();
        let result = source
            .search(query, Category::Anime, None, BrowserChoice::Auto)
            .await;
        match query {
            "generated" => {
                let works = result?;
                assert_eq!(works.len(), 1);
                assert_eq!(works[0].title, "Синтел [версия 2] & legal");
            }
            "persistent" => assert!(matches!(result, Err(Error::SourceCheckRequired))),
            "manual" => {
                assert!(matches!(result, Err(Error::SourceCheckRequired)));
                source.complete_check().await?;
                let works = source
                    .search("manual", Category::Anime, None, BrowserChoice::Auto)
                    .await?;
                assert_eq!(works.len(), 1);
            }
            "fault" => assert!(matches!(
                result,
                Err(Error::SourceCheck(webview::WebViewError::Failed))
            )),
            _ => unreachable!(),
        }
        let requests = fixture.requests.lock().unwrap();
        let recent = &requests[before..];
        assert_eq!(
            recent.len(),
            if matches!(query, "generated" | "persistent") {
                3
            } else if query == "manual" {
                4
            } else {
                2
            }
        );
        if matches!(query, "generated" | "persistent") {
            assert_eq!(recent[1], recent[2]);
        }
        drop(requests);
        if query == "generated" {
            let before = fixture.requests.lock().unwrap().len();
            let works = source
                .search("cached", Category::Anime, None, BrowserChoice::Auto)
                .await?;
            assert_eq!(works[0].title, "Синтел [версия 2] & legal");
            assert_eq!(fixture.requests.lock().unwrap().len() - before, 2);
        }
        source.close();
    }

    fixture.case("login-until-cookie");
    let source = rutracker::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    source.initialize_webview(directory.clone(), app_dir.clone());
    let before = fixture.requests.lock().unwrap().len();
    assert_eq!(
        source
            .search("generated", Category::Anime, None, BrowserChoice::Auto)
            .await?
            .len(),
        1
    );
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len() - before, 3);
    assert_eq!(requests[before + 1], requests[before + 2]);
    drop(requests);
    source.close();

    let source = rutracker::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    source.initialize_webview(directory.clone(), app_dir.clone());
    assert!(matches!(
        source
            .search("login-required", Category::Anime, None, BrowserChoice::Auto)
            .await,
        Err(Error::LoginRequired)
    ));
    source.complete_check().await?;
    assert_eq!(
        source
            .search("login-required", Category::Anime, None, BrowserChoice::Auto)
            .await?
            .len(),
        1
    );
    source.close();

    std::fs::remove_file(app_dir.join("qbutt-webview.exe"))?;
    std::fs::remove_dir(app_dir)?;
    std::fs::remove_dir(directory)?;
    Ok(())
}

async fn browser_sessions(
    fixture: &Fixture,
    source: &rutracker::RuTracker,
    work: &Work,
) -> CheckResult {
    let directory = std::env::temp_dir().join(format!(
        "qbutt-rutracker-browser-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir(&directory)?;
    let discovery = directory.join("browser-bridge.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let handle = broker.handle();
    fixture.case("login");
    assert!(matches!(
        source
            .search(
                "generated",
                Category::Anime,
                Some(&handle),
                BrowserChoice::Auto
            )
            .await,
        Err(Error::LoginRequired)
    ));
    fixture.case("challenge");
    assert!(matches!(
        source
            .search(
                "generated",
                Category::Anime,
                Some(&handle),
                BrowserChoice::Auto
            )
            .await,
        Err(Error::SourceCheck(webview::WebViewError::Unavailable))
    ));

    let mut edge = browser_fixture::extension(&discovery, "edge", "fixture-browser", false).await?;
    wait_browser(&handle, 1).await?;
    fixture.case("session");
    let operation = source.search(
        "generated",
        Category::Anime,
        Some(&handle),
        BrowserChoice::Edge,
    );
    let response = answer_session(&mut edge, fixture, false);
    let (result, reply) = tokio::join!(operation, response);
    reply?;
    let found = result?.remove(0);
    assert!(found.variants.is_empty());
    for resolve in [false, true] {
        let operation = async {
            if resolve {
                source
                    .resolve(
                        &work.variants[0].reference,
                        Some(&handle),
                        BrowserChoice::Edge,
                    )
                    .await
                    .map(|_| ())
            } else {
                source
                    .inspect(&found.reference, Some(&handle), BrowserChoice::Edge)
                    .await
                    .map(|_| ())
            }
        };
        let (result, reply) = tokio::join!(operation, answer_session(&mut edge, fixture, false));
        reply?;
        result?;
    }
    let operation = source.search(
        "generated",
        Category::Anime,
        Some(&handle),
        BrowserChoice::Edge,
    );
    let (result, reply) = tokio::join!(operation, answer_session(&mut edge, fixture, true));
    reply?;
    assert!(matches!(result, Err(Error::LoginRequired)));

    let mut chrome =
        browser_fixture::extension(&discovery, "chrome", "fixture-chrome", false).await?;
    wait_browser(&handle, 2).await?;
    assert!(matches!(
        source
            .search(
                "generated",
                Category::Anime,
                Some(&handle),
                BrowserChoice::Auto
            )
            .await,
        Err(Error::BrowserChoiceRequired)
    ));
    let operation = source.search(
        "generated",
        Category::Anime,
        Some(&handle),
        BrowserChoice::Chrome,
    );
    let response = async {
        let wire = browser_fixture::read(&mut chrome).await?;
        browser_fixture::write(&mut chrome, &json!({"id":wire["id"],"result":[]})).await
    };
    let (result, reply) = tokio::join!(operation, response);
    reply?;
    assert!(matches!(result, Err(Error::LoginRequired)));
    let operation = source.search(
        "generated",
        Category::Anime,
        Some(&handle),
        BrowserChoice::Chrome,
    );
    let response = async {
        let wire = browser_fixture::read(&mut chrome).await?;
        browser_fixture::write(
            &mut chrome,
            &json!({"id":wire["id"],"error":{"code":"browser_busy","message":"generated private text"}}),
        )
        .await
    };
    let (result, reply) = tokio::join!(operation, response);
    reply?;
    assert!(matches!(
        result,
        Err(Error::BrowserSession(BrowserError::Busy))
    ));
    fixture.case("challenge");
    let operation = source.search(
        "generated",
        Category::Anime,
        Some(&handle),
        BrowserChoice::Edge,
    );
    let disconnect = async {
        browser_fixture::read(&mut edge).await?;
        drop(edge);
        Ok::<_, Box<dyn std::error::Error>>(())
    };
    let (result, disconnected) = tokio::join!(operation, disconnect);
    disconnected?;
    assert!(matches!(
        result,
        Err(Error::SourceCheck(webview::WebViewError::Unavailable))
    ));
    drop(chrome);
    broker.shutdown().await?;
    assert!(!discovery.exists());
    std::fs::remove_dir(directory)?;
    Ok(())
}

async fn wait_browser(handle: &browser::BrowserHandle, connections: usize) -> CheckResult {
    let mut status = handle.subscribe();
    tokio::time::timeout(Duration::from_secs(3), async {
        while status.borrow().connections.len() != connections {
            status.changed().await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await??;
    Ok(())
}

async fn answer_session(
    client: &mut tokio::net::windows::named_pipe::NamedPipeClient,
    fixture: &Fixture,
    expired: bool,
) -> CheckResult {
    let wire = browser_fixture::read(client).await?;
    assert_eq!(wire["method"], "cookies");
    assert_eq!(
        wire["url"],
        fixture.origin.join("/forum/tracker.php")?.as_str()
    );
    let mut cookie = json!({
        "name":"bbsession", "value":"fixture-session", "domain":"fixture.example",
        "path":"/forum/", "secure":true, "hostOnly":true
    });
    if expired {
        cookie["expirationDate"] = json!(1.0);
    }
    browser_fixture::write(client, &json!({"id":wire["id"], "result":[cookie]})).await
}

fn library(work: &Work) -> CheckResult {
    let directory = std::env::temp_dir().join(format!(
        "qbutt-rutracker-library-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir(&directory)?;
    let result = library_profile(&directory, work);
    std::fs::remove_dir_all(directory)?;
    result
}

fn library_profile(directory: &Path, work: &Work) -> CheckResult {
    use qbutt_content::{
        Error as StoredError, Library, LibraryView, SourceFailure, SourceFailureReason,
        SourceOperation,
    };

    let mut work: qbutt_content::Work = serde_json::from_value(serde_json::to_value(work)?)?;
    work.url = "https://rutracker.org/forum/viewtopic.php?t=42".parse()?;
    let operation = SourceOperation::Resolve(work.variants[0].reference.clone());
    let mut library = Library::open(directory)?;
    let settings = library.source_settings()?;
    assert_eq!(
        settings
            .iter()
            .filter(|row| row.source == qbutt_content::Source::RuTracker)
            .count(),
        1
    );
    assert!(
        settings
            .iter()
            .find(|row| row.source == qbutt_content::Source::RuTracker)
            .unwrap()
            .enabled
    );
    library.set_favorite(&qbutt_content::WorkGroup::single(work.clone()), true)?;
    for (error, reason) in [
        (
            StoredError::LoginRequired,
            SourceFailureReason::LoginRequired,
        ),
        (StoredError::Challenge, SourceFailureReason::Challenge),
        (
            StoredError::SourceCheckRequired,
            SourceFailureReason::SourceCheckRequired,
        ),
        (
            StoredError::SourceCheck(webview::WebViewError::Unavailable),
            SourceFailureReason::SourceCheck,
        ),
        (
            StoredError::BrowserChoiceRequired,
            SourceFailureReason::BrowserChoiceRequired,
        ),
        (
            StoredError::BrowserSession(qbutt_content::browser::BrowserError::InvalidResponse),
            SourceFailureReason::BrowserSession,
        ),
        (
            StoredError::UnsupportedTorrent,
            SourceFailureReason::UnsupportedTorrent,
        ),
    ] {
        assert_eq!(SourceFailureReason::from_error(&error), Some(reason));
        library.record_source_failure(SourceFailure {
            operation: operation.clone(),
            reason,
        })?;
    }
    for url in [
        "https://rutracker.ru/forum/viewtopic.php?t=42",
        "https://rutracker.org/forum/viewtopic.php?t=43",
        "https://rutracker.org/forum/viewtopic.php?t=42&token=private",
        "https://rutracker.org/forum/dl.php?t=42",
    ] {
        let mut invalid = work.clone();
        invalid.url = url.parse()?;
        assert!(
            library
                .set_favorite(&qbutt_content::WorkGroup::single(invalid.clone()), true)
                .is_err()
        );
    }
    drop(library);
    let library = Library::open(directory)?;
    assert_eq!(
        library.entries(LibraryView::Favorites)?[0]
            .group
            .anchor_work()
            .unwrap(),
        &work
    );
    let failure = library
        .source_settings()?
        .into_iter()
        .find(|row| row.source == qbutt_content::Source::RuTracker)
        .unwrap()
        .failure
        .unwrap();
    assert_eq!(failure.operation, operation);
    let persisted = serde_json::to_string(&failure)?;
    for secret in [
        "fixture-session",
        "fixture-browser",
        "magnet:",
        "tracker.example",
        "token=",
    ] {
        assert!(!persisted.contains(secret));
    }
    assert!(library.entries(LibraryView::History)?.is_empty());
    Ok(())
}
