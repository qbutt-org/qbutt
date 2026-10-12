//! Bounded public metadata check. No episode payload is downloaded.

use qbutt_content::{AniTokyo, ResolvedContent, VariantSelector};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let source = AniTokyo::new()?;
    let search = source.search("Psyren").await?;
    let publication = search
        .iter()
        .find(|work| work.reference.id == "9981")
        .ok_or("Psyren publication missing from AniTokyo search")?;
    let work = source.inspect(&publication.reference).await?;
    let selected = work
        .variants
        .iter()
        .find(|variant| {
            matches!(
                &variant.reference.selector,
                VariantSelector::AniTokyoVideo {
                    episode,
                    dubbing,
                    ..
                } if episode == "1" && dubbing == "Дублированный"
            )
        })
        .ok_or("selected AniTokyo episode and dubbing missing")?;
    let ResolvedContent::DirectVideo {
        reference,
        url,
        quality,
        ..
    } = source.resolve(&selected.reference).await?
    else {
        return Err("AniTokyo episode did not resolve to direct video".into());
    };
    assert_eq!(reference, selected.reference);
    assert_eq!(url.scheme(), "https");

    let subtitled = work
        .variants
        .iter()
        .find(|variant| {
            matches!(
                &variant.reference.selector,
                VariantSelector::AniTokyoVideo {
                    episode,
                    dubbing,
                    ..
                } if episode == "1" && dubbing == "Субтитры „Crunchyroll.Subtitles“"
            )
        })
        .ok_or("selected AniTokyo subtitled episode missing")?;
    let ResolvedContent::DirectVideo {
        reference,
        url,
        quality: subtitle_quality,
        ..
    } = source.resolve(&subtitled.reference).await?
    else {
        return Err("AniTokyo subtitles did not resolve to direct video".into());
    };
    assert_eq!(reference, subtitled.reference);
    assert_eq!(url.scheme(), "https");
    println!(
        "PASS anitokyo live search={} variants={} selected_episode=1 dubbing=exact {}p subtitles=exact {}p",
        search.len(),
        work.variants.len(),
        quality.height(),
        subtitle_quality.height()
    );
    Ok(())
}
