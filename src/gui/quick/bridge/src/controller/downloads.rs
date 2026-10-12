use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::browser::{BrowserChoice, BrowserCommand, BrowserError, ResolveKind, Selection};
use qbutt_content::{
    CapturedVideoError, DirectDownload, DownloadQueue, MediaGateway, MediaGatewayError,
    ResolvedContent, SourceOperation, Transport, VariantSelector, resolve_captured_video,
    resolve_external_video,
};
use url::Url;

use super::catalog::SourceResponse;
use super::ffi;
use super::sources::SourceRequest;
use crate::download_model;

pub(super) struct CapturedIntent {
    page: Url,
    title: String,
    capture_id: String,
}

impl ffi::ContentController {
    pub fn downloads_model(&self) -> *mut download_model::ffi::DownloadModel {
        self.downloads.as_ptr().cast_mut()
    }

    pub fn init_downloads(mut self: Pin<&mut Self>, destination: QString) {
        if self
            .download_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        if self.services.is_none() {
            self.as_mut()
                .set_error(Some(ffi::ContentError::ServicesUnavailable));
            return;
        }
        let Some(worker) = &self.library_worker else {
            self.as_mut()
                .set_error(Some(ffi::ContentError::LibraryOpen));
            return;
        };

        let destination = PathBuf::from(destination.to_string());
        let storage = worker.download_store();
        let qt_thread = self.qt_thread();
        self.as_mut().rust_mut().downloads_state = QString::from("loading");
        self.as_mut().downloads_state_changed();
        let task = self.services.as_ref().unwrap().runtime.spawn(async move {
            let queue = match DownloadQueue::open(storage, destination).await {
                Ok(queue) => Arc::new(queue),
                Err(error) => {
                    let detail = error.to_string();
                    let _ = qt_thread.queue(move |mut controller| {
                        ffi::log_content_error(&QString::from("downloads"), &QString::from(detail));
                        controller.as_mut().rust_mut().downloads_state = QString::from("failed");
                        controller.as_mut().downloads_state_changed();
                        if let Some(intent) = controller.as_mut().rust_mut().captured_intent.take()
                        {
                            controller.as_mut().captured_download_finished(
                                QString::from(intent.capture_id),
                                QString::from("failed"),
                            );
                        }
                        controller
                            .as_mut()
                            .set_error(Some(ffi::ContentError::Download));
                    });
                    return;
                }
            };

            let mut updates = queue.subscribe();
            let initial = updates.borrow().clone();
            let owner = Arc::clone(&queue);
            if qt_thread
                .queue(move |mut controller| {
                    controller.as_mut().rust_mut().download_queue = Some(owner);
                    controller.as_mut().rust_mut().downloads_state = QString::from("ready");
                    controller.as_mut().downloads_state_changed();
                    controller
                        .as_mut()
                        .rust_mut()
                        .downloads
                        .pin_mut()
                        .apply(initial);
                    if let Some(intent) = controller.as_mut().rust_mut().captured_intent.take() {
                        controller.as_mut().download_captured(
                            QString::from(intent.page.as_str()),
                            QString::from(intent.title),
                            QString::from(intent.capture_id),
                        );
                    }
                })
                .is_err()
            {
                queue.close().await;
                return;
            }

            while updates.changed().await.is_ok() {
                let jobs = updates.borrow().clone();
                if qt_thread
                    .queue(move |mut controller| {
                        controller
                            .as_mut()
                            .rust_mut()
                            .downloads
                            .pin_mut()
                            .apply(jobs);
                    })
                    .is_err()
                {
                    break;
                }
            }
            queue.close().await;
        });
        self.as_mut().rust_mut().download_task = Some(task);
    }

    pub fn download_selected(mut self: Pin<&mut Self>, id: QString) -> bool {
        if id != self.selected_variant_id {
            return false;
        }
        let Some(variant) = self.variants.find(&id.to_string()) else {
            return false;
        };
        if variant.transport != Transport::Video {
            return false;
        }
        if !self.selected_variant_saveable() {
            return false;
        }
        let Some(queue) = self.download_queue.as_ref().map(Arc::clone) else {
            self.as_mut().set_error(Some(ffi::ContentError::Download));
            return false;
        };
        let reference = variant.reference.clone();
        let Some(request) = self.source_request(SourceOperation::Resolve(reference)) else {
            return false;
        };
        self.as_mut().resolve_download(request, queue, None)
    }

    pub fn download_playback(mut self: Pin<&mut Self>, launch_token: u64) -> bool {
        let Some(reference) = self.playback.reference_for(launch_token) else {
            return false;
        };
        if !matches!(&reference.selector, VariantSelector::YummyAnimeVideo { player, .. } if player == "Плеер VK")
        {
            return false;
        }
        let Some(queue) = self.download_queue.as_ref().map(Arc::clone) else {
            return false;
        };
        let Some(request) = self.source_request(SourceOperation::Resolve(reference)) else {
            return false;
        };
        self.as_mut().resolve_download(request, queue, None)
    }

    pub fn download_captured(
        mut self: Pin<&mut Self>,
        page: QString,
        title: QString,
        capture_id: QString,
    ) -> bool {
        let Ok(page) = Url::parse(&page.to_string()) else {
            return false;
        };
        let Some(source_host) = page.host_str().map(str::to_owned) else {
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
        if command.validate().is_err() {
            return false;
        }
        self.as_mut().rust_mut().captured_download.cancel();
        let Some(queue) = self.download_queue.as_ref().map(Arc::clone) else {
            if self.downloads_state == QString::from("loading") {
                self.as_mut().rust_mut().captured_intent = Some(CapturedIntent {
                    page,
                    title,
                    capture_id,
                });
                return true;
            }
            return false;
        };
        let browser = match &self.services {
            Some(services) => services.browser.client(),
            None => return false,
        };
        let qt_thread = self.qt_thread();
        let generation = self.captured_download.generation;
        let task = self.services.as_ref().unwrap().runtime.spawn(async move {
            let prepared = async {
                let handle = browser.handle().await.map_err(|_| "capture")?;
                let media = resolve_captured_video(&handle, page, capture_id.clone())
                    .await
                    .map_err(|error| match error {
                        CapturedVideoError::Unsupported => "unsupported",
                        CapturedVideoError::Browser(BrowserError::BrowserRequired)
                        | CapturedVideoError::Browser(BrowserError::ChoiceRequired)
                        | CapturedVideoError::Browser(BrowserError::Disconnected) => {
                            "browser_required"
                        }
                        CapturedVideoError::Browser(BrowserError::UnsupportedMultiAudio)
                        | CapturedVideoError::Browser(BrowserError::UnsupportedPrivateTracks) => {
                            "unsupported"
                        }
                        CapturedVideoError::Browser(_) => "capture",
                    })?;
                let (gateway, playback) = MediaGateway::prepare(media).await.map_err(|error| {
                    if matches!(error, MediaGatewayError::UnsupportedManifest) {
                        "unsupported"
                    } else {
                        "capture"
                    }
                })?;
                DirectDownload::captured(source_host, title, playback, gateway)
                    .map_err(|_| "unsupported")
            }
            .await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.captured_download.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().captured_download.task = None;
                let download = match prepared {
                    Ok(download) => download,
                    Err(reason) => {
                        controller.as_mut().captured_download_finished(
                            QString::from(capture_id),
                            QString::from(reason),
                        );
                        return;
                    }
                };
                let Some(services) = &controller.services else {
                    return;
                };
                let ack_thread = controller.qt_thread();
                services.runtime.spawn(async move {
                    let outcome = queue.enqueue(download).await;
                    let _ = ack_thread.queue(move |mut controller| {
                        if controller.captured_download.generation != generation {
                            if let Ok(job) = outcome {
                                if let Some(services) = &controller.services {
                                    services.runtime.spawn(async move {
                                        let _ = queue.cancel(&job.id).await;
                                    });
                                }
                            }
                            return;
                        }
                        let reason = if outcome.is_ok() { "" } else { "failed" };
                        if outcome.is_ok() {
                            controller.as_mut().download_queued();
                        }
                        controller.as_mut().captured_download_finished(
                            QString::from(capture_id),
                            QString::from(reason),
                        );
                    });
                });
            });
        });
        self.as_mut().rust_mut().captured_download.task = Some(task);
        true
    }

    pub fn cancel_captured_download(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().captured_download.cancel();
        self.as_mut().rust_mut().captured_intent = None;
    }

    pub fn cancel_download(self: Pin<&mut Self>, id: QString) -> bool {
        if !self.downloads.cancellable(&id.to_string()) {
            return false;
        }
        let Some(queue) = self.download_queue.as_ref().map(Arc::clone) else {
            return false;
        };
        let Some(services) = &self.services else {
            return false;
        };
        let id = id.to_string();
        services.runtime.spawn(async move {
            if let Err(error) = queue.cancel(&id).await {
                ffi::log_content_error(
                    &QString::from("cancel download"),
                    &QString::from(error.to_string()),
                );
            }
        });
        true
    }

    pub fn retry_download(mut self: Pin<&mut Self>, id: QString) -> bool {
        let Some(reference) = self.downloads.retryable_reference(&id.to_string()) else {
            return false;
        };
        let Some(queue) = self.download_queue.as_ref().map(Arc::clone) else {
            return false;
        };
        let Some(request) = self.source_request(SourceOperation::Resolve(reference)) else {
            return false;
        };
        self.as_mut()
            .resolve_download(request, queue, Some(id.to_string()))
    }

    fn resolve_download(
        mut self: Pin<&mut Self>,
        request: SourceRequest,
        queue: Arc<DownloadQueue<super::library::DownloadStore>>,
        retry_id: Option<String>,
    ) -> bool {
        let Some(services) = &self.services else {
            self.as_mut()
                .set_error(Some(ffi::ContentError::ServicesUnavailable));
            return false;
        };
        let catalog = Arc::clone(&services.catalog);
        let browser = services.browser.client();
        let qt_thread = self.qt_thread();
        services.runtime.spawn(async move {
            let result = catalog.execute(&request.operation).await;
            let external = if let Ok(SourceResponse::Resolve(ResolvedContent::ExternalVideo {
                reference,
                url,
                ..
            })) = &result
            {
                if matches!(&reference.selector, VariantSelector::YummyAnimeVideo { player, .. } if player == "Плеер VK") {
                    Some(async {
                        let handle = browser.handle().await.map_err(|_| ())?;
                        let media = resolve_external_video(
                            &handle,
                            BrowserChoice::Auto,
                            reference,
                            url.clone(),
                        )
                        .await
                        .map_err(|_| ())?;
                        MediaGateway::prepare(media).await.map_err(|_| ())
                    }
                    .await)
                } else {
                    None
                }
            } else {
                None
            };
            let _ = qt_thread.queue(move |mut controller| {
                if !controller.source_request_current(&request) {
                    return;
                }
                controller.as_mut().source_outcome(&request, &result);
                if !controller.source_request_current(&request) {
                    return;
                }

                let SourceOperation::Resolve(reference) = &request.operation else {
                    return;
                };
                let download = match result {
                    Ok(SourceResponse::Resolve(ResolvedContent::DirectVideo {
                        reference: resolved,
                        title,
                        url,
                        ..
                    })) if &resolved == reference => DirectDownload::new(resolved, title, url),
                    Ok(SourceResponse::Resolve(ResolvedContent::ExternalVideo {
                        reference: resolved,
                        title,
                        ..
                    })) if &resolved == reference => match external {
                        Some(Ok((gateway, media))) => {
                            DirectDownload::prepared(resolved, title, media, gateway)
                        }
                        Some(Err(())) => {
                            controller
                                .as_mut()
                                .set_error(Some(ffi::ContentError::Download));
                            return;
                        }
                        None => {
                            controller
                                .as_mut()
                                .set_error(Some(ffi::ContentError::Download));
                            return;
                        }
                    },
                    _ => {
                        controller
                            .as_mut()
                            .set_error(Some(ffi::ContentError::Download));
                        return;
                    }
                };
                let download = match download {
                    Ok(download) => download,
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("download descriptor"),
                            &QString::from(error.to_string()),
                        );
                        controller
                            .as_mut()
                            .set_error(Some(ffi::ContentError::Download));
                        return;
                    }
                };
                let Some(services) = &controller.services else {
                    return;
                };
                let error_thread = controller.qt_thread();
                services.runtime.spawn(async move {
                    let fresh = retry_id.is_none();
                    let outcome = if let Some(id) = retry_id {
                        queue.retry(&id, download).await
                    } else {
                        queue.enqueue(download).await.map(|_| ())
                    };
                    if let Err(error) = outcome {
                        let detail = error.to_string();
                        ffi::log_content_error(
                            &QString::from("download queue"),
                            &QString::from(&detail),
                        );
                        let _ = error_thread.queue(move |mut controller| {
                            controller
                                .as_mut()
                                .set_error(Some(ffi::ContentError::Download));
                        });
                    } else if fresh {
                        let _ = error_thread.queue(move |mut controller| {
                            controller.as_mut().download_queued();
                        });
                    }
                });
            });
        });
        true
    }
}
