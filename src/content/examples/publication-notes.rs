//! Read-only native extraction from two exact public game publications.

use qbutt_content::{FitGirl, OnlineFix, PublicationRef, Source};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let online_fix = OnlineFix::new()?
        .inspect(&PublicationRef {
            source: Source::OnlineFix,
            id: "games/officialservers/16889-ready-or-not-po-seti.html".into(),
        })
        .await?;
    assert!(
        !online_fix.notes.is_empty(),
        "no public Online-Fix comments were extracted"
    );
    assert!(
        online_fix
            .description
            .as_deref()
            .is_some_and(|text| text.contains("Как запускать:"))
    );
    let linked = online_fix
        .notes
        .iter()
        .filter(|note| note.source_url(&online_fix.url).is_some())
        .count();
    let authored = online_fix
        .notes
        .iter()
        .filter(|note| note.author.is_some())
        .count();
    assert!(linked > 0 && authored > 0);
    assert!(
        online_fix
            .notes
            .iter()
            .all(|note| !note.text.contains("<script") && !note.text.contains("<iframe"))
    );
    println!(
        "PASS onlinefix_publication_notes publication={} comments={} authors={} published_links={} text_bytes={}",
        online_fix.reference.id,
        online_fix.notes.len(),
        authored,
        linked,
        online_fix
            .notes
            .iter()
            .map(|note| note.text.len())
            .sum::<usize>()
    );

    let fitgirl = FitGirl::new()?
        .inspect(&PublicationRef {
            source: Source::FitGirl,
            id: "cyberpunk-2077".into(),
        })
        .await?;
    let description = fitgirl
        .description
        .as_deref()
        .ok_or("missing FitGirl description")?;
    assert!(
        description.contains("Repack Features") && description.contains("Backwards Compatibility")
    );
    println!(
        "PASS fitgirl_publication_details publication={} description_bytes={} embedded_comments={} external_widget=not_acquired",
        fitgirl.reference.id,
        description.len(),
        fitgirl.notes.len()
    );
    Ok(())
}
