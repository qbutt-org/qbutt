//! Generated pipe + HTTPS integration for the operation-scoped Lib transport.
//! The fixture supplies TLS/DNS only; no product endpoints or profile are changed.

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
#[cfg(windows)]
pub use qbutt_content::webview;
pub use qbutt_content::{Source, TorrentMetadata};

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client) -> SourceHttp {
        SourceHttp {
            client,
            origin: "https://api.cdnlibs.org/".parse().unwrap(),
        }
    }
}

#[allow(dead_code)]
mod lib_api {
    include!("../src/lib_api.rs");

    pub(super) fn fixture(client: reqwest::Client) -> LibHttp {
        LibHttp {
            source: Source::RanobeLib,
            http: crate::http::fixture(client),
            origin: "https://api.cdnlibs.org/".parse().unwrap(),
        }
    }
}

#[cfg(windows)]
#[path = "lib_session/mod.rs"]
mod acceptance;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    acceptance::run().await?;
    #[cfg(not(windows))]
    return Err("Lib browser integration requires Windows".into());
    Ok(())
}
