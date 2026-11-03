//! Native pipe/socket lifecycle acceptance with a simulated qcatch extension.
//! Optional arguments: an existing qcatch.exe and its pinned extension ID.
//! This proves transport and native-host relay, not a real browser media session.

#[path = "browser/mod.rs"]
mod browser;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    if std::env::args().nth(1).as_deref() == Some("--abandon-broker") {
        let path = std::env::args_os()
            .nth(2)
            .ok_or("Missing isolated discovery path")?;
        let _broker = qbutt_content::browser::BrowserBroker::start(path.into()).await?;
        std::future::pending::<()>().await;
    }
    browser::run().await
}
