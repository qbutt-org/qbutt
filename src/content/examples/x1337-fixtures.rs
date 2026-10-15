//! Generated HTTPS catalog traffic through the production 1337x provider.
//! No live profiles, external downloads or torrent payloads are accessed.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
#[allow(dead_code)]
#[path = "../src/sources.rs"]
mod sources;
pub use qbutt_content::webview;
pub use sources::SourceOperation;

#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[allow(dead_code)]
#[path = "../src/limetorrents.rs"]
mod limetorrents;

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
mod x1337 {
    include!("../src/x1337.rs");

    pub(super) fn fixture(http: SourceHttp, origin: Url) -> X1337 {
        X1337 {
            http,
            origin,
            webview: std::sync::OnceLock::new(),
            clearance: std::sync::Mutex::new(None),
        }
    }
}

#[path = "x1337/browser.rs"]
mod browser_checks;
#[allow(dead_code)]
#[path = "browser/fixture.rs"]
mod browser_fixture;
#[path = "x1337/check.rs"]
mod check;
#[path = "x1337/fixture.rs"]
mod fixture;

use std::sync::atomic::Ordering;
use std::time::Duration;

use browser::BrowserChoice;
use fixture::{Fixture, HASH, PUBLICATION};
use url::Url;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--stdio") {
        return check::helper();
    }
    let fixture = Fixture::start().await?;
    let source = x1337::fixture(
        http::fixture(fixture.client.clone(), fixture.origin.clone()),
        fixture.origin.clone(),
    );
    let found = source
        .search(
            "generated-&-legal",
            Category::Movies,
            None,
            BrowserChoice::Auto,
        )
        .await?;
    assert_eq!(
        found.len(),
        2,
        "excluded uploader must not remove its normal neighbors"
    );
    assert_eq!(found[0].reference.id, PUBLICATION);
    assert!(
        found
            .iter()
            .all(|work| work.needs_inspection && work.variants.is_empty())
    );
    assert_eq!(
        found[0].description.as_deref(),
        Some("Uploader: LegalFixture")
    );

    let work = source
        .inspect(&found[0].reference, None, BrowserChoice::Auto)
        .await?;
    assert_eq!(work.reference, found[0].reference);
    assert_eq!(work.variants.len(), 1);
    let selected = &work.variants[0].reference;
    assert_eq!(work.variants[0].info_hash.as_deref(), Some(HASH));
    assert_eq!(work.variants[0].size, Some(1_310_720));
    assert_eq!(work.variants[0].seeders, Some(4));
    assert_eq!(work.variants[0].languages, ["en"]);
    let ResolvedContent::Torrent {
        reference, magnet, ..
    } = source.resolve(selected, None, BrowserChoice::Auto).await?
    else {
        panic!("unexpected acquisition");
    };
    assert_eq!(reference, *selected);
    assert!(
        magnet
            .query_pairs()
            .any(|(key, value)| key == "xt" && value == format!("urn:btih:{HASH}"))
    );

    fixture.changed.store(true, Ordering::SeqCst);
    assert!(matches!(
        source.resolve(selected, None, BrowserChoice::Auto).await,
        Err(Error::VariantUnavailable)
    ));
    fixture.changed.store(false, Ordering::SeqCst);
    for id in ["404/Missing", "410/Gone", "66/Excluded", "45/External-Only"] {
        let missing = PublicationRef {
            source: Source::X1337,
            id: id.into(),
        };
        assert!(
            matches!(
                source.inspect(&missing, None, BrowserChoice::Auto).await,
                Err(Error::VariantUnavailable)
            ),
            "{id}"
        );
    }
    for id in ["44/Bad-Hash", "46/Not-Html", "47/Ambiguous-Metadata"] {
        let invalid = PublicationRef {
            source: Source::X1337,
            id: id.into(),
        };
        assert!(
            matches!(
                source.inspect(&invalid, None, BrowserChoice::Auto).await,
                Err(Error::InvalidResponse(_))
            ),
            "{id}"
        );
    }
    assert!(
        source
            .search("empty", Category::Movies, None, BrowserChoice::Auto)
            .await?
            .is_empty()
    );
    for query in [
        "duplicate",
        "foreign",
        "wrong query",
        "missing table",
        "bad session",
        "wrong redirect",
    ] {
        assert!(
            matches!(
                source
                    .search(query, Category::Movies, None, BrowserChoice::Auto)
                    .await,
                Err(Error::InvalidResponse(_) | Error::InvalidReference)
            ),
            "fault {query}"
        );
    }
    assert!(matches!(
        source
            .search("challenge", Category::Movies, None, BrowserChoice::Auto)
            .await,
        Err(Error::SourceCheck(webview::WebViewError::Unavailable))
    ));
    assert_eq!(
        fixture.cookie_faults.load(Ordering::SeqCst),
        0,
        "the source's form session was not retained"
    );

    let hits = fixture.hits.load(Ordering::SeqCst);
    for query in ["", "---", "bad\nquery", &"x".repeat(501)] {
        assert!(matches!(
            source
                .search(query, Category::Movies, None, BrowserChoice::Auto)
                .await,
            Err(Error::InvalidQuery)
        ));
    }
    assert!(
        source
            .search("generated", Category::Manga, None, BrowserChoice::Auto)
            .await?
            .is_empty()
    );
    for id in [
        "0/Legal",
        "042/Legal",
        "42/../Legal",
        "42/%2e%2e",
        "42/Legal?token=value",
        "42/Legal/extra",
    ] {
        let invalid = PublicationRef {
            source: Source::X1337,
            id: id.into(),
        };
        assert!(matches!(
            source.inspect(&invalid, None, BrowserChoice::Auto).await,
            Err(Error::InvalidReference)
        ));
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), hits);

    for category in [
        Category::Movies,
        Category::Tv,
        Category::Anime,
        Category::Games,
        Category::Software,
        Category::Books,
    ] {
        let rows = source
            .search("categories", category, None, BrowserChoice::Auto)
            .await?;
        assert_eq!(rows.len(), 2);
        let release = source
            .inspect(&rows[0].reference, None, BrowserChoice::Auto)
            .await?;
        assert_eq!(release.category, category);
        assert_eq!(release.variants[0].info_hash.as_deref(), Some(HASH));

        assert!(
            source
                .search("category empty", category, None, BrowserChoice::Auto)
                .await?
                .is_empty(),
            "empty {category:?} category after a successful general search"
        );
    }
    for query in ["missing heading", "missing message", "unknown message"] {
        for category in [Category::Anime, Category::Books] {
            let result = source
                .search(query, category, None, BrowserChoice::Auto)
                .await;
            assert!(matches!(result, Err(Error::InvalidResponse(_))));
        }
    }
    browser_checks::run(&source, &fixture.origin).await?;
    check::run(&fixture).await?;

    let mut pending =
        Box::pin(source.search("cancel", Category::Movies, None, BrowserChoice::Auto));
    tokio::select! {
        started = tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.cancelled.load(Ordering::SeqCst) != 1 { tokio::task::yield_now().await; }
        }) => started?,
        _ = &mut pending => panic!("cancel completed before request"),
    }
    drop(pending);
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.cancelled.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert_eq!(fixture.payload.load(Ordering::SeqCst), 0);
    fixture.stop().await;

    // Persist the generated publication through the actual Library schema.
    let profile = std::env::temp_dir().join(format!("qbutt-1337x-library-{}", std::process::id()));
    std::fs::create_dir(&profile)?;
    let mut persisted: qbutt_content::Work = serde_json::from_value(serde_json::to_value(&work)?)?;
    persisted.url = Url::parse(&format!("https://1337x.to/torrent/{PUBLICATION}/"))?;
    let group = qbutt_content::WorkGroup::single(persisted.clone());
    let mut library = qbutt_content::Library::open(&profile)?;
    library.set_favorite(&group, true)?;
    drop(library);
    let library = qbutt_content::Library::open(&profile)?;
    let entries = library.entries(qbutt_content::LibraryView::Favorites)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].group.publications[0], persisted);
    drop(library);
    std::fs::remove_dir_all(profile)?;
    println!(
        "PASS 1337x_fixtures generated_https=ok form_cookie_jar=ok categories=6 category_empty=6 category_faults=6 exact_magnet=ok stale=rejected uploader_exclusion=isolated malformed_foreign=rejected browser_fetch=ok browser_category_empty=2 browser_fault_cancel=ok empty=ok cancel=connection_closed sqlite=reopened payload=none"
    );
    Ok(())
}
