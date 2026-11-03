//! Native browser-bridge acceptance with generated peers or a private JSONL driver.
//! Default mode exercises lifetime and optional qcatch native-host relay.
//! --driver <discovery> accepts typed public commands for a real browser fixture.

#[path = "browser/mod.rs"]
mod browser;
#[path = "browser/driver.rs"]
mod driver;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--driver") {
        let path = std::env::args_os()
            .nth(2)
            .ok_or("Missing isolated discovery path")?;
        if std::env::args_os().nth(3).is_some() {
            return Err("Unexpected browser fixture driver argument".into());
        }
        return driver::run(path.into()).await;
    }
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
