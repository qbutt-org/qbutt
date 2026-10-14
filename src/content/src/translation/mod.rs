//! Native translation shares context validation and accepted cache ownership.
//! The caller owns cancellation, caching, and committing the accepted result.

pub mod context;
pub mod context_tools;
pub mod manga_imagegen;
pub mod openrouter;
pub mod page;
pub mod text;
