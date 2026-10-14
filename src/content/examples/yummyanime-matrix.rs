//! Resolve every exact video variant of one live episode through the native provider.
//! Public evidence excludes acquisition URLs; private descriptors stay in the QA directory.

use std::path::PathBuf;

use qbutt_content::{PublicationRef, ResolvedContent, Source, YummyAnime};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 3 {
        return Err("Usage: yummyanime-matrix <publication id> <episode> <QA directory>".into());
    }
    let output = PathBuf::from(&arguments[2]);
    std::fs::create_dir_all(&output)?;
    let catalog = YummyAnime::new()?;
    let work = catalog
        .inspect(&PublicationRef {
            source: Source::YummyAnime,
            id: arguments[0].clone(),
        })
        .await?;
    std::fs::write(output.join("work.json"), serde_json::to_vec_pretty(&work)?)?;

    let variants: Vec<_> = work
        .variants
        .iter()
        .filter(|variant| variant.episode.as_deref() == Some(arguments[1].as_str()))
        .collect();
    if variants.is_empty() {
        return Err("The selected episode has no current variants".into());
    }

    let mut evidence = Vec::new();
    let mut descriptors = Vec::new();
    let mut failed = false;
    for variant in variants {
        let started = std::time::Instant::now();
        let resolution = catalog.resolve(&variant.reference).await;
        let (status, quality, error) = match resolution {
            Ok(ResolvedContent::DirectVideo {
                reference,
                url,
                quality,
                ..
            }) => {
                assert_eq!(reference, variant.reference);
                descriptors.push(json!({ "reference": reference, "url": url, "quality": quality }));
                ("direct", Some(quality.height()), None)
            }
            Ok(ResolvedContent::ExternalVideo { reference, .. }) => {
                assert_eq!(reference, variant.reference);
                ("browser_required", None, None)
            }
            Ok(_) => return Err("The selected video changed transport".into()),
            Err(error) => {
                failed = true;
                ("failed", None, Some(error.to_string()))
            }
        };
        let result = json!({
            "reference": variant.reference,
            "player": variant.player,
            "dubbing": variant.translation,
            "status": status,
            "quality": quality,
            "error": error,
            "elapsedMs": started.elapsed().as_millis(),
        });
        println!("{result}");
        evidence.push(result);
    }
    std::fs::write(
        output.join("resolve.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    std::fs::write(
        output.join("private-descriptors.json"),
        serde_json::to_vec_pretty(&descriptors)?,
    )?;
    if failed {
        return Err("Some exact episode variants did not resolve to native playback".into());
    }
    Ok(())
}
