use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};
use qbutt_content::browser::{BrowserCommand, BrowserError, ResolveKind, Selection};
use qbutt_content::{CapturedVideoError, MediaGateway, MediaGatewayError, resolve_captured_video};
use url::Url;

use super::ffi;

impl ffi::ContentController {
    pub fn watch_captured(
        mut self: Pin<&mut Self>,
        page: QString,
        title: QString,
        capture_id: QString,
    ) -> bool {
        let Ok(page) = Url::parse(&page.to_string()) else {
            return false;
        };
        let title = title.to_string();
        if title.trim().is_empty()
            || title.encode_utf16().count() > 512
            || title.chars().any(char::is_control)
        {
            return false;
        }
        let capture_id = capture_id.to_string();
        let command = BrowserCommand::Resolve {
            url: page.clone(),
            selection: Selection {
                capture_id: Some(capture_id.clone()),
                ..Selection::default()
            },
            kind: ResolveKind::Video,
            referrer: None,
        };
        if command.validate().is_err() || !self.playback_capacity() {
            return false;
        }
        let Some(browser) = self
            .services
            .as_ref()
            .map(|services| services.browser.client())
        else {
            return false;
        };
        let qt_thread = self.qt_thread();
        let choice = browser.choice();
        self.as_mut().rust_mut().captured_watch.cancel();
        let generation = self.captured_watch.generation;
        let task = self.services.as_ref().unwrap().runtime.spawn(async move {
            let prepared = async {
                let handle = browser.handle().await.map_err(|_| "browser_required")?;
                let media = resolve_captured_video(&handle, choice, page, capture_id.clone())
                    .await
                    .map_err(|error| match error {
                        CapturedVideoError::Browser(
                            BrowserError::BrowserRequired
                            | BrowserError::ChoiceRequired
                            | BrowserError::Disconnected,
                        ) => "browser_required",
                        CapturedVideoError::Unsupported => "unsupported",
                        CapturedVideoError::Browser(
                            BrowserError::UnsupportedMultiAudio
                            | BrowserError::UnsupportedPrivateTracks,
                        ) => "unsupported",
                        CapturedVideoError::Browser(_) => "capture",
                    })?;
                MediaGateway::prepare(media).await.map_err(|error| {
                    if matches!(error, MediaGatewayError::UnsupportedManifest) {
                        "unsupported"
                    } else {
                        "capture"
                    }
                })
            }
            .await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.captured_watch.generation != generation {
                    if let Ok((gateway, _)) = prepared
                        && let Some(services) = controller.as_mut().rust_mut().services.as_mut()
                    {
                        services.retire_gateway(gateway);
                    }
                    return;
                }
                controller.as_mut().rust_mut().captured_watch.task = None;
                let (gateway, playback) = match prepared {
                    Ok(prepared) => prepared,
                    Err(reason) => {
                        controller.as_mut().captured_watch_finished(
                            QString::from(capture_id),
                            QString::from(reason),
                        );
                        return;
                    }
                };
                let url = QUrl::from(&QString::from(playback.url.as_str()));
                let accepted = controller.as_mut().accept_video(
                    gateway,
                    None,
                    Some(&capture_id),
                    url,
                    QString::from(title),
                    Some(playback),
                );
                controller.as_mut().captured_watch_finished(
                    QString::from(capture_id),
                    QString::from(if accepted { "" } else { "failed" }),
                );
            });
        });
        self.as_mut().rust_mut().captured_watch.task = Some(task);
        true
    }

    pub fn cancel_captured_watch(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().captured_watch.cancel();
    }
}
