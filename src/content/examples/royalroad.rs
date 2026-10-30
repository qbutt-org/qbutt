use qbutt_content::{PublicationRef, RoyalRoad, Source, Transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = RoyalRoad::new()?;
    let mut arguments = std::env::args_os().skip(1);
    let query = if let Some(app_dir) = arguments.next() {
        let profile = arguments
            .next()
            .ok_or("usage: royalroad [HELPER_DIRECTORY PROFILE_DIRECTORY [SEARCH_QUERY]]")?;
        source.initialize_webview(profile.into(), app_dir.into());
        arguments
            .next()
            .map(|query| query.into_string().map_err(|_| "query is not UTF-8"))
            .transpose()?
    } else {
        None
    };
    let result = exercise(&source, query.as_deref()).await;
    source.close();
    result
}

async fn exercise(
    source: &RoyalRoad,
    query: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let search = source.search(query.unwrap_or("Mother of Learning")).await?;
    if query.is_some() {
        println!(
            "PASS Royal Road live exact search: results={}",
            search.len()
        );
        return Ok(());
    }
    let reference = PublicationRef {
        source: Source::RoyalRoad,
        id: "21220".into(),
    };
    let listed = search
        .iter()
        .find(|work| work.reference == reference)
        .ok_or("known fiction missing from search")?;
    assert!(listed.needs_inspection);
    let work = source.inspect(&reference).await?;
    assert!(!work.needs_inspection);
    assert_eq!(work.variants[0].transport, Transport::Text);
    assert_eq!(work.variants[0].languages, ["en"]);
    let chapters = source.chapters(&work.variants[0].reference).await?;
    let selected = chapters.get(1).ok_or("nonfirst public chapter missing")?;
    assert_eq!(selected.reference.id, "21220/301781");
    let chapter = source.chapter_text(&selected.reference).await?;
    assert_eq!(chapter.reference, selected.reference);
    assert!(chapter.blocks.len() > 50);
    assert!(chapter.text_bytes() > 10_000);
    println!(
        "PASS Royal Road live search/inspect/public TOC/nonfirst chapter; fiction={} chapters={} selected={} blocks={} text_bytes={}",
        reference.id,
        chapters.len(),
        selected.reference.id,
        chapter.blocks.len(),
        chapter.text_bytes()
    );
    Ok(())
}
