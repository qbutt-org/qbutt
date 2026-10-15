//! Generated legal publications and unchanged production HTTP/parser code.
//! Fixture torrent acquisition does not establish acceptance of the real hosts.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[cfg(windows)]
mod webview {
    pub use qbutt_content::webview::{WebViewError, WebViewSession};
}

#[allow(dead_code)]
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
mod dodi {
    include!("../src/dodi.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> Dodi {
        Dodi { http, origin }
    }

    pub(super) async fn host_fixture(
        http: &SourceHttp,
        url: Url,
        id: &str,
    ) -> Result<crate::TorrentMetadata> {
        let mut session = SourceSession::anonymous(url.clone())?;
        #[cfg(windows)]
        session.set_bearer(Some("generated-fixture-authorization".into()))?;
        host::download(http, url, &mut session, id).await
    }
}

#[path = "dodi-fixtures/https.rs"]
mod https;

use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use https::Fixture;
use url::Url;

const FIRST_ID: &str = "aaaaaaaabbbb";
const SECOND_ID: &str = "ccccccccdddd";
const METADATA: &[u8] =
    b"d4:infod6:lengthi0e4:name19:generated-legal.txt12:piece lengthi16384e6:pieces0:ee";
const TITLE: &str = "42- Generated Game: Deluxe Edition (v1.2.3 + MULTi2) [DODI Repack]";

type AcceptanceResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> AcceptanceResult<()> {
    let fixture = Fixture::start().await?;
    assert!(qbutt_content::Source::ALL.contains(&qbutt_content::Source::Dodi));
    let provider = dodi::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let works = provider.search("generated & legal").await?;
    assert_eq!(works.len(), 1);
    assert_eq!(works[0].title, TITLE);
    assert_eq!(
        works[0].aliases,
        ["Generated Game: Deluxe Edition (v1.2.3 + MULTi2) [DODI Repack]"]
    );
    assert_eq!(works[0].category, Category::Games);
    assert!(works[0].needs_inspection && works[0].variants.is_empty());
    let publication = &works[0].reference;
    assert_eq!(publication.id, "42--generated-game");
    let work = provider.inspect(publication).await?;
    assert_eq!(work.reference, *publication);
    assert!(!work.needs_inspection);
    assert_eq!(work.variants.len(), 2);
    assert!(
        work.variants[1]
            .title
            .as_deref()
            .unwrap()
            .contains("file-me.top")
    );
    assert!(work.variants.iter().all(|variant| {
        variant.size.is_none()
            && variant.seeders.is_none()
            && variant.languages.is_empty()
            && variant.info_hash.is_none()
    }));
    let selected = &work.variants[1].reference;
    fixture.changed.store(true, Ordering::SeqCst);
    assert!(matches!(
        provider.resolve(selected).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.changed.store(false, Ordering::SeqCst);
    assert!(provider.search("empty").await?.is_empty());
    assert_eq!(provider.search("redirect").await?.len(), 1);
    for (query, expected) in [
        ("challenge", "metadata"),
        ("wrong-query", "metadata"),
        ("json", "metadata"),
        ("utf8", "metadata"),
        ("huge-length", "limit"),
        ("foreign", "unsafe"),
        ("local", "unsafe"),
        ("downgrade", "unsafe"),
        ("loop", "redirect"),
        ("delay", "timeout"),
    ] {
        let error = match provider.search(query).await {
            Err(error) => error,
            Ok(_) => panic!("HTTP fault {query} succeeded"),
        };
        let actual = match error {
            Error::InvalidResponse(_) => "metadata",
            Error::ResponseTooLarge => "limit",
            Error::UnsafeAddress => "unsafe",
            Error::InvalidRedirect => "redirect",
            Error::Timeout => "timeout",
            _ => panic!("unexpected HTTP fault {query}: {error}"),
        };
        assert_eq!(actual, expected, "HTTP fault {query}");
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), provider.search("cancel"))
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert_eq!(provider.search("after-cancel").await?.len(), 1);
    for slug in ["missing", "gone"] {
        let missing = PublicationRef {
            source: Source::Dodi,
            id: format!("42--{slug}"),
        };
        assert!(matches!(
            provider.inspect(&missing).await,
            Err(Error::VariantUnavailable)
        ));
    }
    parser_faults(&fixture.origin, publication)?;
    host_forms(&fixture).await?;
    library_roundtrip(&work)?;
    fixture.stop().await;
    println!(
        "PASS dodi-fixtures search=lazy identity=exact nonfirst_mirror=preserved stale=rejected empty=ok CDN_redirect=ok cookie_auth_scope=ok POST_replay=rejected malformed=rejected oversize=rejected cancellation=ok SQLite=ok invented_hash=absent"
    );
    Ok(())
}

fn parser_faults(origin: &Url, publication: &PublicationRef) -> AcceptanceResult<()> {
    use dodi::response;

    let detail = detail_page(origin, false);
    for body in [
        detail.replace("post-42", "post-43"),
        detail.replace("/generated-game/", "/replacement-game/"),
    ] {
        assert!(matches!(
            response::release(&body, origin, publication),
            Err(Error::VariantUnavailable)
        ));
    }
    for body in [
        detail.replace("rel='canonical'", "rel='alternate'"),
        detail.replace("Download Links", "Gameplay"),
        detail.replace(FIRST_ID, "invalid-id"),
        detail.replace(
            "<h1 class='entry-title'>",
            "<h1 class='entry-title'></h1><h1 class='entry-title'>",
        ),
        detail.replace(
            "<link rel='canonical'",
            "<link rel='canonical' href='https://127.0.0.1/generated-game/'><link rel='canonical'",
        ),
        detail.replace("post-42", "post-042"),
    ] {
        assert!(response::release(&body, origin, publication).is_err());
    }
    let unsupported = detail
        .replace("file-me.top", "unsupported.example")
        .replace("www.up-4ever.net", "other.example");
    assert!(matches!(
        response::release(&unsupported, origin, publication),
        Err(Error::UnsupportedAcquisition(_))
    ));
    for id in [
        "42",
        "0--game",
        "042--game",
        "42--",
        "42--../game",
        "42--game?token=private",
        "42--Game",
        "18446744073709551616--game",
    ] {
        assert!(
            PublicationRef {
                source: Source::Dodi,
                id: id.into()
            }
            .validate()
            .is_err()
        );
    }
    let search = search_page(origin, "legal");
    let row = search
        .split("<main id='main'>")
        .nth(1)
        .unwrap()
        .split("</main>")
        .next()
        .unwrap();
    for body in [
        search.replace("body class='search'", "body"),
        search.replace("post-42", "post-0"),
        search.replace("value='legal'", "value='other'"),
        search.replace("</main>", &format!("{row}</main>")),
        search.replace("class='entry-title'", "class='changed'"),
    ] {
        assert!(response::search(&body, origin, "legal").is_err());
    }
    let news = "
        <article class='post' id='post-99'>
            <div class='entry-meta'>
                <a rel='bookmark' href='/upcoming-repacks/'>Date</a>
            </div>
        </article>";
    assert_eq!(
        response::search(
            &search.replace("</main>", &format!("{news}</main>")),
            origin,
            "legal"
        )?
        .len(),
        1
    );
    Ok(())
}

async fn host_forms(fixture: &Fixture) -> AcceptanceResult<()> {
    let http = http::fixture(fixture.client.clone(), fixture.origin.clone());
    for id in [FIRST_ID, SECOND_ID] {
        let path = format!("/{id}.html");
        let acquired = dodi::host_fixture(&http, fixture.origin.join(&path)?, id).await?;
        assert_eq!(acquired.bytes(), METADATA);
    }
    let redirect = fixture
        .origin
        .join(&format!("/{FIRST_ID}.html?case=redirect"))?;
    let metadata = dodi::host_fixture(&http, redirect, FIRST_ID).await?;
    assert_eq!(metadata.bytes(), METADATA);

    for (case, expected) in [
        ("redirect-foreign", "unsafe"),
        ("redirect-private", "unsafe"),
        ("redirect-http", "unsafe"),
        ("redirect-replay", "unsafe"),
        ("redirect-loop", "redirect"),
        ("malformed", "metadata"),
        ("trailing", "metadata"),
        ("empty", "metadata"),
        ("large", "limit"),
    ] {
        let url = fixture
            .origin
            .join(&format!("/{FIRST_ID}.html?case={case}"))?;
        let error = match dodi::host_fixture(&http, url, FIRST_ID).await {
            Err(error) => error,
            Ok(_) => panic!("torrent acquisition fault {case} succeeded"),
        };
        let actual = match error {
            Error::UnsafeAddress => "unsafe",
            Error::InvalidResponse(_) | Error::UnsupportedAcquisition(_) => "metadata",
            Error::ResponseTooLarge => "limit",
            Error::InvalidRedirect => "redirect",
            _ => panic!("unexpected acquisition fault {case}: {error}"),
        };
        assert_eq!(actual, expected, "acquisition fault {case}");
    }
    for (case, message) in [
        ("gate", "DODI torrent host requires browser verification"),
        ("rate", "DODI torrent host download rate limit"),
        (
            "unsupported",
            "torrent host returned HTML instead of metadata",
        ),
    ] {
        let url = fixture
            .origin
            .join(&format!("/{FIRST_ID}.html?case={case}"))?;
        assert!(
            matches!(dodi::host_fixture(&http, url, FIRST_ID).await, Err(Error::InvalidResponse(actual)) if actual == message)
        );
    }
    for (case, expected) in [
        ("wrong-id", "identity"),
        ("external-action", "unsafe"),
        ("duplicate-op", "metadata"),
    ] {
        let url = fixture
            .origin
            .join(&format!("/{FIRST_ID}.html?case={case}"))?;
        let error = match dodi::host_fixture(&http, url, FIRST_ID).await {
            Err(error) => error,
            Ok(_) => panic!("host form fault {case} succeeded"),
        };
        let actual = match error {
            Error::VariantUnavailable => "identity",
            Error::UnsafeAddress => "unsafe",
            Error::InvalidResponse(_) => "metadata",
            _ => panic!("unexpected host form fault {case}: {error}"),
        };
        assert_eq!(actual, expected);
    }
    let cancelled_before = fixture.cancelled.load(Ordering::SeqCst);
    let url = fixture
        .origin
        .join(&format!("/{FIRST_ID}.html?case=cancel"))?;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            dodi::host_fixture(&http, url, FIRST_ID)
        )
        .await
        .is_err()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) == cancelled_before {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    Ok(())
}

fn library_roundtrip(work: &Work) -> AcceptanceResult<()> {
    use qbutt_content::{Library, LibraryView, VariantPreferences, WorkGroup};

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile =
        std::env::temp_dir().join(format!("qbutt-dodi-fixture-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&profile)?;
    let result = (|| -> AcceptanceResult<()> {
        let work: qbutt_content::Work = serde_json::from_value(serde_json::to_value(work)?)?;
        let preferences = VariantPreferences {
            selected: Some(work.variants[1].reference.clone()),
            ..Default::default()
        };
        let group = WorkGroup::single(work.clone());
        let mut library = Library::open(&profile)?;
        library.set_favorite(&group, true)?;
        library.save_preferences(&group, preferences.clone())?;
        let confirmed = library.get(&work.reference)?.unwrap();
        let mut invented_hash = work.clone();
        invented_hash.variants[0].info_hash = Some("0".repeat(40));
        assert!(
            library
                .set_favorite(&WorkGroup::single(invented_hash), false)
                .is_err()
        );
        assert_eq!(library.get(&work.reference)?.unwrap(), confirmed);
        drop(library);
        let library = Library::open(&profile)?;
        let reopened = library
            .get(&work.reference)?
            .ok_or("missing saved DODI work")?;
        assert_eq!(reopened.group, WorkGroup::single(work));
        assert_eq!(reopened.preferences, preferences);
        assert!(library.entries(LibraryView::History)?.is_empty());
        Ok(())
    })();
    std::fs::remove_dir_all(&profile)?;
    result
}

fn search_page(origin: &Url, query: &str) -> String {
    let query = query.replace('&', "&amp;").replace('\'', "&#39;");
    if query == "empty" {
        return format!(
            "<body class='search search-no-results'>
                <main id='main'></main>
                <input name='s' value='{query}'>
            </body>"
        );
    }
    format!(
        "<body class='search'>
            <main id='main'>
                <article id='post-42' class='post'>
                    <h2 class='entry-title'>
                        <a rel='bookmark' href='{origin}generated-game/'>{TITLE}</a>
                    </h2>
                </article>
            </main>
            <input name='s' value='{query}'>
        </body>"
    )
}

fn detail_page(origin: &Url, changed: bool) -> String {
    let secondary = if changed { FIRST_ID } else { SECOND_ID };
    format!(
        "<link rel='canonical' href='{origin}generated-game/'>
        <main id='main'>
            <article id='post-42' class='post'>
                <h1 class='entry-title'>{TITLE}</h1>
                <div class='entry-content'>
                    <p>Gameplay <a href='https://www.up-4ever.net/ignoredabcde'>ignore</a></p>
                    <p><strong>Download Links</strong></p>
                    <p>
                        Torrent
                        <a href='http://file-me.top/{FIRST_ID}.html'>Click Here</a>
                        <a href='https://file-me.top/{secondary}.html'>Click Here</a>
                    </p>
                    <p>Soundtrack <a href='https://www.up-4ever.net/ignoredabcde'>Click Here</a></p>
                </div>
            </article>
        </main>"
    )
}
