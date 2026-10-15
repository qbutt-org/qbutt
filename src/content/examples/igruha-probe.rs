//! One bounded Ready or Not search case using an isolated, owned QA profile.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use qbutt_content::{Category, Error, Igruha, SourceFailureReason, browser::BrowserChoice, query};
use tokio::time::{Instant, timeout_at};

const QUERY: &str = "ready or not";

#[tokio::main]
async fn main() -> ExitCode {
    let Some(app_dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: igruha-probe HELPER_DIRECTORY");
        return ExitCode::FAILURE;
    };
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
        let result = timeout_at(
            deadline,
            source.search(QUERY, Category::Games, None, BrowserChoice::Auto),
        )
        .await;
        source.close();
        drop(source);

        match result {
            Ok(Ok(works)) => {
                let matching = works
                    .iter()
                    .filter(|work| query::matches(QUERY, work))
                    .count();
                println!(
                    "phase={phase} result=success raw_results={} matching_titles={matching} elapsed_ms={}",
                    works.len(),
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

fn report_error(phase: u8, error: &Error, elapsed_ms: u128) {
    match error {
        Error::SourceCheck(error) => {
            println!("phase={phase} result=source_check subtype={error:?} elapsed_ms={elapsed_ms}")
        }
        Error::HttpStatus(status) => {
            println!("phase={phase} result=http_status status={status} elapsed_ms={elapsed_ms}")
        }
        error => println!(
            "phase={phase} result=source_error reason={:?} elapsed_ms={elapsed_ms}",
            SourceFailureReason::from_error(error)
        ),
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
