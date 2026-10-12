//! Process acceptance against the installed WebView2 Runtime and the built
//! qbutt-webview helper. No credentials or site data are printed.

#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qbutt_content::webview::{WebViewCheck, WebViewError, WebViewSessions};
    use std::path::PathBuf;
    use url::Url;

    let app_dir = PathBuf::from(std::env::args().nth(1).ok_or("pass the helper directory")?);
    let address = Url::parse(
        &std::env::args()
            .nth(2)
            .ok_or("pass a public HTTPS check URL")?,
    )?;
    let mut random = [0u8; 8];
    getrandom::fill(&mut random)?;
    let profile = std::env::temp_dir().join(format!(
        "qbutt-webview-check-{}-{}",
        std::process::id(),
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ));
    let sessions = WebViewSessions::new(profile.clone(), app_dir);
    let (first, second) = tokio::join!(
        sessions.check(address.clone(), WebViewCheck::default()),
        sessions.check(address.clone(), WebViewCheck::default())
    );
    assert_eq!(first?.user_agent, second?.user_agent);
    assert!(matches!(
        sessions
            .check(
                address.clone(),
                WebViewCheck {
                    configured_session: Some("invalid".into()),
                    ..Default::default()
                }
            )
            .await,
        Err(WebViewError::InvalidRequest)
    ));
    sessions.close();
    assert!(matches!(
        sessions.check(address, WebViewCheck::default()).await,
        Err(WebViewError::Stopped)
    ));
    let mut cleanup = None;
    for _ in 0..20 {
        match std::fs::remove_dir_all(&profile) {
            Ok(()) => {
                cleanup = None;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                cleanup = None;
                break;
            }
            Err(error) => cleanup = Some(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    if let Some(error) = cleanup {
        return Err(error.into());
    }
    println!("PASS hidden WebView2 check, guarded session, coalescing and bounded faults");
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
