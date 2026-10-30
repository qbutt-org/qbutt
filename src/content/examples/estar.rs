use qbutt_content::{Estar, Source, Transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = Estar::new()?;
    if let Some(query) = std::env::args().nth(1) {
        let results = provider.search(&query).await?;
        println!("Estar search: {} results", results.len());
        return Ok(());
    }

    let results = provider.search("下の階の消防士").await?;
    println!("Estar search: {} results", results.len());
    let found = results
        .iter()
        .find(|work| work.reference.id == "26510966")
        .ok_or("known public Estar work was not found")?;
    assert_eq!(found.reference.source, Source::Estar);

    let work = provider.inspect(&found.reference).await?;
    println!("Estar inspect: {}", work.reference.id);
    assert_eq!(work.variants[0].transport, Transport::Text);
    assert_eq!(work.variants[0].languages, ["ja"]);

    let chapters = provider.chapters(&work.variants[0].reference).await?;
    println!("Estar chapters: {}", chapters.len());
    assert_eq!(chapters.len(), 31);
    assert_eq!(chapters[1].reference.id, "26510966/6");
    let text = provider.chapter_text(&chapters[1].reference).await?;
    assert!(text.blocks.len() >= 3);
    assert!(text.text_bytes() > 2_000);
    println!(
        "PASS Estar search, inspect, {} chapters and non-first chapter ({} blocks, {} text bytes)",
        chapters.len(),
        text.blocks.len(),
        text.text_bytes()
    );
    Ok(())
}
