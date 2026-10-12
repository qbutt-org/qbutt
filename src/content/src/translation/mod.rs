//! Translation jobs use Codex as an installed, isolated inference provider.
//! The caller owns cancellation, caching, and committing the accepted result.

pub mod context;
pub mod context_tools;
pub mod manga;
pub mod text;
