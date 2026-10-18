//! Bounded live search, inspection and resolution in an isolated QA profile.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use qbutt_content::{Category, Error, Igruha, ResolvedContent, browser::BrowserChoice, query};
use tokio::time::{Instant, timeout_at};

#[tokio::main]
async fn main() -> ExitCode {
    let Some(app_dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: igruha-probe HELPER_DIRECTORY [QUERY]");
        return ExitCode::FAILURE;
    };
    let query = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "ready or not".into());
    let mut random = [0u8; 8];
    if getrandom::fill(&mut random).is_err() {
        eprintln!("probe_error=profile_identity");
        return ExitCode::FAILURE;
    }
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let profile = std::env::temp_dir().join(format!(
        "qbutt-igruha-probe-{}-{suffix}",
        std::process::id()
    ));
    if std::fs::create_dir(&profile).is_err() {
        eprintln!("probe_error=profile_create");
        return ExitCode::FAILURE;
    }

    let deadline = Instant::now() + Duration::from_secs(90);
    println!(
        "igruha_probe profile=isolated helper_present={} case_budget_ms=90000",
        app_dir.join("qbutt-webview.exe").is_file()
    );
    let mut succeeded = true;
    for phase in 1..=2 {
        let source = match Igruha::new() {
            Ok(source) => source,
            Err(error) => {
                report_error(phase, &error, 0);
                succeeded = false;
                break;
            }
        };
        source.initialize_webview(profile.clone(), app_dir.clone());
        let started = Instant::now();
        let result = timeout_at(deadline, check_publication(&source, &query, phase)).await;
        source.close();
        drop(source);

        match result {
            Ok(Ok(matching)) => {
                println!(
                    "phase={phase} result=success matching_titles={matching} elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                if matching == 0 {
                    succeeded = false;
                    break;
                }
            }
            Ok(Err(error)) => {
                report_error(phase, &error, started.elapsed().as_millis());
                succeeded = false;
                break;
            }
            Err(_) => {
                println!(
                    "phase={phase} result=case_timeout elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                succeeded = false;
                break;
            }
        }
        // A successful check has awaited helper exit and gateway shutdown before
        // returning. Only that completed first phase permits the fresh provider.
    }

    let cleaned = cleanup(&profile).await;
    println!("cleanup={}", if cleaned { "complete" } else { "pending" });
    if succeeded && cleaned {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

async fn check_publication(source: &Igruha, query: &str, phase: u8) -> Result<usize, Error> {
    let works = source
        .search(query, Category::Games, None, BrowserChoice::Auto)
        .await?;
    let matching = works
        .iter()
        .filter(|work| query::matches(query, work))
        .count();
    println!("phase={phase} raw_results={}", works.len());
    for work in &works {
        println!(
            "phase={phase} publication={} title={:?}",
            work.reference.id, work.title
        );
    }

    let Some(publication) = works.iter().find(|work| query::matches(query, work)) else {
        return Ok(matching);
    };
    let work = source
        .inspect(&publication.reference, None, BrowserChoice::Auto)
        .await?;
    println!("phase={phase} inspected_variants={}", work.variants.len());
    for variant in &work.variants {
        println!(
            "phase={phase} variant={:?} size={:?} info_hash={:?}",
            variant.title, variant.size, variant.info_hash
        );
    }

    let selected = &work
        .variants
        .first()
        .ok_or(Error::VariantUnavailable)?
        .reference;
    let ResolvedContent::TorrentFile {
        reference,
        metadata,
    } = source.resolve(selected, None, BrowserChoice::Auto).await?
    else {
        return Err(Error::InvalidResponse("Igruha did not resolve a torrent"));
    };
    if reference != *selected {
        return Err(Error::InvalidResponse(
            "Igruha resolved a different variant",
        ));
    }
    println!(
        "phase={phase} exact_resolution=true metadata_bytes={}",
        metadata.bytes().len()
    );
    Ok(matching)
}

fn report_error(phase: u8, error: &Error, elapsed_ms: u128) {
    match error {
        Error::SourceCheck(error) => {
            println!("phase={phase} result=source_check subtype={error:?} elapsed_ms={elapsed_ms}")
        }
        Error::HttpStatus(status) => {
            println!("phase={phase} result=http_status status={status} elapsed_ms={elapsed_ms}")
        }
        error => {
            println!("phase={phase} result=source_error message={error} elapsed_ms={elapsed_ms}")
        }
    }
}

async fn cleanup(profile: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(profile) else {
        return false;
    };
    let (Ok(resolved), Ok(temporary)) =
        (profile.canonicalize(), std::env::temp_dir().canonicalize())
    else {
        return false;
    };
    if metadata.file_type().is_symlink() || resolved.parent() != Some(temporary.as_path()) {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    for _ in 0..20 {
        match std::fs::remove_dir_all(profile) {
            Ok(()) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    false
}
