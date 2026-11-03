//! Generated named-pipe and HTTPS integration acceptance. Production source
//! files remain unchanged; only the fixture supplies its own TLS client/DNS.

#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
pub use qbutt_content::TorrentMetadata;

#[allow(dead_code, unused_imports)]
#[path = "../src/browser/mod.rs"]
mod browser;

mod webview {
    pub use qbutt_content::webview::{WebViewError, WebViewSession};
}

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[path = "session/mod.rs"]
mod session;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    session::run().await?;
    Ok(())
}
