#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("The local book could not be read")]
    LocalBookIo(#[source] std::io::Error),
    #[error("{0}")]
    InvalidLocalBook(&'static str),
    #[error("The book file changed since import")]
    LocalBookChanged,
    #[error("Enter a search query of at most 500 characters")]
    InvalidQuery,
    #[error("The publication does not belong to this source")]
    InvalidReference,
    #[error("The selected variant is no longer available")]
    VariantUnavailable,
    #[error("{0}")]
    UnsupportedAcquisition(&'static str),
    #[error("Sign in to the source in the browser and retry this action")]
    LoginRequired,
    #[error("Complete the source check in the browser and retry this action")]
    Challenge,
    #[cfg(windows)]
    #[error("The source requires a manual check")]
    SourceCheckRequired,
    #[error("Choose a connected browser and retry this action")]
    BrowserChoiceRequired,
    #[cfg(windows)]
    #[error("The connected browser session could not be used: {0}")]
    BrowserSession(#[source] crate::browser::BrowserError),
    #[cfg(windows)]
    #[error("The source check could not be completed: {0}")]
    SourceCheck(#[source] crate::webview::WebViewError),
    #[error("Torrent file downloads from RuTracker are not supported yet")]
    UnsupportedTorrent,
    #[error("The source returned invalid metadata: {0}")]
    InvalidResponse(&'static str),
    #[error("The source returned HTTP {0}")]
    HttpStatus(u16),
    #[error("The site is temporarily unavailable")]
    ServiceUnavailable,
    #[error("The source response exceeds the size limit")]
    ResponseTooLarge,
    #[error("The source request timed out")]
    Timeout,
    #[error("The source returned an unsafe address")]
    UnsafeAddress,
    #[error("The source returned an unsupported redirect")]
    InvalidRedirect,
    #[error("The source could not be reached")]
    Network(#[source] reqwest::Error),
    #[error("The source returned invalid JSON")]
    Json(#[source] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
