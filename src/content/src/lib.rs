//! Native content services, independent of Qt and the application runtime.
//!
//! Provider operations borrow their service and do not spawn tasks. The caller
//! owns the async runtime, request generation and cancellation: drop an operation's
//! future to stop awaiting its result. Apply completed results to Qt models only on
//! the GUI thread and only while their generation remains current.
//! Library storage is synchronous and must be scheduled outside the GUI thread.

mod aniliberty;
mod http;
mod library;
mod mangadex;
mod model;
mod rutor;
mod sources;

pub use aniliberty::AniLiberty;
pub use library::{
    LastOpened, Library, LibraryEntry, LibraryError, LibraryResult, LibraryView, ReaderFit,
    ReaderMode, ReaderProgress, ReaderSettings, VariantFilters, VariantPreferences,
};
pub use mangadex::MangaDex;
pub use model::*;
pub use rutor::RuTor;
pub use sources::{SourceFailure, SourceFailureReason, SourceOperation, SourceSettings};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Enter a search query of at most 500 characters")]
    InvalidQuery,
    #[error("The publication does not belong to this source")]
    InvalidReference,
    #[error("The selected variant is no longer available")]
    VariantUnavailable,
    #[error("The source returned invalid metadata: {0}")]
    InvalidResponse(&'static str),
    #[error("The source returned HTTP {0}")]
    HttpStatus(u16),
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
