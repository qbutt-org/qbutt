//! Read-only live acceptance through an explicitly supplied qcatch discovery.
//! The native host must already be configured for that isolated broker path.

use std::path::PathBuf;
use std::time::Duration;

use qbutt_content::browser::{BrowserBroker, BrowserChoice, BrowserError, BrowserHandle};
use qbutt_content::{Category, Error, ResolvedContent, VariantSelector, X1337};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let discovery = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("pass the isolated qcatch discovery path")?,
    );
    if !discovery.is_absolute() {
        return Err("the discovery path must be absolute".into());
    }
    let broker = BrowserBroker::start(discovery).await?;
    let handle = broker.handle();
    let result = exercise(&handle).await;
    broker.shutdown().await?;
    result
}

async fn exercise(browser: &BrowserHandle) -> Result<(), Box<dyn std::error::Error>> {
    let mut status = browser.subscribe();
    tokio::time::timeout(Duration::from_secs(30), async {
        while status.borrow().connections.is_empty() {
            status.changed().await.map_err(|_| BrowserError::Stopped)?;
        }
        Ok::<_, BrowserError>(())
    })
    .await??;

    let source = X1337::new()?;
    let choice = BrowserChoice::Edge;
    let results = source
        .search("Sintel", Category::Movies, Some(browser), choice)
        .await?;
    let found = results
        .iter()
        .find(|work| work.reference.id.starts_with("1010994/"))
        .ok_or("known Blender open-movie publication was not returned")?;
    assert!(found.needs_inspection && found.variants.is_empty());
    let work = source
        .inspect(&found.reference, Some(browser), choice)
        .await?;
    assert_eq!(work.reference, found.reference);
    assert_eq!(work.category, Category::Movies);
    assert_eq!(work.variants.len(), 1);
    let selected = &work.variants[0].reference;
    assert_eq!(
        work.variants[0].info_hash.as_deref(),
        Some("64877b5490208c3015c0f5121287949d62622e54")
    );
    let ResolvedContent::Torrent {
        reference, magnet, ..
    } = source.resolve(selected, Some(browser), choice).await?
    else {
        return Err("unexpected acquisition".into());
    };
    assert_eq!(reference, *selected);
    assert!(magnet.query_pairs().any(|(name, value)| {
        name == "xt"
            && value.eq_ignore_ascii_case("urn:btih:64877b5490208c3015c0f5121287949d62622e54")
    }));
    let mut stale = selected.clone();
    stale.selector = VariantSelector::TorrentHash {
        info_hash: "0".repeat(40),
    };
    assert!(matches!(
        source.resolve(&stale, Some(browser), choice).await,
        Err(Error::VariantUnavailable)
    ));
    assert!(
        source
            .search(
                "qbutt legal fixture 907376",
                Category::Movies,
                Some(browser),
                choice
            )
            .await?
            .is_empty()
    );
    println!(
        "PASS 1337x_live search={} publication={} exact_magnet=ok stale=rejected empty=ok payload=none native_add_dialog=unverified",
        results.len(),
        work.reference.id
    );
    Ok(())
}
