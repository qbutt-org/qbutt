//! Read-only live Online-Fix acceptance. It fetches small `.torrent` metadata only.

use qbutt_content::{Category, OnlineFix, ResolvedContent, Transport, VariantSelector};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let source = OnlineFix::new()?;
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Stardew Valley".into());
    let found = source.search(&query, Category::Games).await?;
    let publication = found.first().ok_or("no Online-Fix game results")?;
    assert!(publication.needs_inspection && publication.variants.is_empty());

    let work = source.inspect(&publication.reference).await?;
    assert_eq!(work.reference, publication.reference);
    assert_eq!(work.category, Category::Games);
    assert!(!work.needs_inspection && !work.variants.is_empty());
    let description = work
        .description
        .as_deref()
        .ok_or("missing publication instructions")?;
    assert!(description.contains("Как запускать:"));
    if query.eq_ignore_ascii_case("ready or not") {
        for expected in [
            "Файлы для игры:",
            "Версия игры:",
            "ReadyOrNot.exe",
            "Epic Games Store",
            "Принимаем приглашение от друга.",
            "Multiplayer → Host Friends Only",
            "КООПЕРАТИВ: 5",
        ] {
            assert!(
                description.contains(expected),
                "missing current Ready or Not instruction"
            );
        }
        assert!(!description.contains("КОММЕНТАРИЕВ:") && !description.contains("<iframe"));
    }

    for variant in &work.variants {
        assert_eq!(variant.transport, Transport::Torrent);
        let title = variant
            .title
            .as_deref()
            .ok_or("missing Online-Fix variant title")?;
        assert!(title.starts_with("Full game:"));
        let VariantSelector::TorrentHash { info_hash } = &variant.reference.selector else {
            return Err("Online-Fix variant has no exact torrent hash".into());
        };
        let ResolvedContent::TorrentFile {
            reference,
            metadata,
        } = source.resolve(&variant.reference).await?
        else {
            return Err("Online-Fix returned a non-torrent variant".into());
        };
        assert_eq!(reference, variant.reference);
        println!(
            "variant={} infohash={} metadata_bytes={}",
            title,
            info_hash,
            metadata.bytes().len()
        );
    }

    println!(
        "PASS onlinefix_live search={} publication={} variants={} description_bytes={} launch_instructions=present exact_full_game_torrent=ok game_payload=none fix_archive=none",
        found.len(),
        work.reference.id,
        work.variants.len(),
        description.len()
    );
    Ok(())
}
