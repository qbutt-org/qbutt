//! Generated named-pipe and HTTPS integration acceptance. Production source
//! files remain unchanged; only the fixture supplies its own TLS client/DNS.

pub use qbutt_content::{Error, Result};

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[cfg(windows)]
#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[cfg(windows)]
#[path = "session/mod.rs"]
mod session;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    session::run().await?;
    #[cfg(not(windows))]
    return Err("The browser session fixture requires Windows".into());
    Ok(())
}
