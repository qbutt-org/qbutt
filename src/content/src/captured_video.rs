use url::Url;

use crate::PlaybackMedia;
use crate::browser::{
    BrowserChoice, BrowserCommand, BrowserError, BrowserHandle, BrowserReply, BrowserResolver,
    ResolveKind, Selection,
};

#[derive(Debug, thiserror::Error)]
pub enum CapturedVideoError {
    #[error(transparent)]
    Browser(#[from] BrowserError),
    #[error("This video cannot be saved as one MP4 file")]
    Unsupported,
}

/// Resolve one qcatch selection against the currently connected browser session.
/// The capture ID is ephemeral and never becomes download-job provenance.
pub async fn resolve_captured_video(
    browser: &BrowserHandle,
    page: Url,
    capture_id: String,
) -> Result<PlaybackMedia, CapturedVideoError> {
    let reply = browser
        .request_connected(
            BrowserChoice::Auto,
            BrowserCommand::Resolve {
                url: page.clone(),
                selection: Selection {
                    capture_id: Some(capture_id),
                    ..Selection::default()
                },
                kind: ResolveKind::Video,
                referrer: None,
            },
        )
        .await?;
    let BrowserReply::Resolved(media) = reply else {
        return Err(BrowserError::InvalidResponse.into());
    };
    if !matches!(media.resolver, BrowserResolver::Direct) {
        return Err(CapturedVideoError::Unsupported);
    }
    if media
        .source_url
        .as_ref()
        .is_some_and(|source| source != &page)
    {
        return Err(BrowserError::SelectionChanged.into());
    }
    Ok((*media).into())
}
