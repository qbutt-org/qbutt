use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::browser::{BrowserCommand, BrowserError, ResolveKind, Selection};
use qbutt_content::{
    CapturedVideoError, DirectDownload, DownloadError, DownloadQueue, MediaGateway, MediaTools,
    PlaybackMedia, PreparedPlayback, ResolvedContent, SourceOperation, Transport, VariantSelector,
    resolve_captured_video, resolve_external_video,
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

static NEXT_MEDIA_TOOL_REQUEST: AtomicU64 = AtomicU64::new(1);

struct MediaToolRequest {
    id: u64,
    receiver: tokio::sync::oneshot::Receiver<Result<MediaTools, String>>,
    qt_thread: CxxQtThread<ffi::ContentController>,
}

impl MediaToolRequest {
    async fn receive(mut self) -> Result<MediaTools, qbutt_content::DownloadError> {
        tokio::time::timeout(Duration::from_secs(35), &mut self.receiver)
            .await
            .map_err(|_| qbutt_content::DownloadError::MediaTools)?
            .map_err(|_| qbutt_content::DownloadError::MediaTools)?
            .map_err(|_| qbutt_content::DownloadError::MediaTools)
    }
}

impl Drop for MediaToolRequest {
    fn drop(&mut self) {
        let id = self.id;
        let _ = self.qt_thread.queue(move |controller| {
            controller.cancel_media_tools(id);
        });
    }
}

enum PreparedAcquisition {
    Direct(MediaGateway, PreparedPlayback),
    Segmented(PlaybackMedia),
}

async fn prepare_acquisition(media: PlaybackMedia) -> Result<PreparedAcquisition, DownloadError> {
    if DirectDownload::requires_mux(&media).await? {
        return Ok(PreparedAcquisition::Segmented(media));
    }
    let (gateway, prepared) = MediaGateway::prepare(media)
        .await
        .map_err(|_| DownloadError::Stream)?;
    Ok(PreparedAcquisition::Direct(gateway, prepared))
}

impl ffi::ContentController {
    fn request_media_tools(mut self: Pin<&mut Self>) -> MediaToolRequest {
        let id = NEXT_MEDIA_TOOL_REQUEST.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.as_mut()
            .rust_mut()
            .media_tool_requests
            .insert(id, sender);
        self.as_mut().media_tools_requested(id);
        MediaToolRequest {
            id,
            receiver,
            qt_thread: self.qt_thread(),
        }
    }

    fn cancel_media_tools(mut self: Pin<&mut Self>, request_id: u64) {
        if self
            .as_mut()
            .rust_mut()
            .media_tool_requests
            .remove(&request_id)
            .is_some()
        {
            self.as_mut().media_tools_cancelled(request_id);
        }
        if self.captured_media_tool_request == Some(request_id) {
            self.as_mut().rust_mut().captured_media_tool_request = None;
        }
    }

    pub fn media_tools_ready(
        mut self: Pin<&mut Self>,
        request_id: u64,
        ffmpeg: QString,
        ffprobe: QString,
        error: QString,
    ) {
        let Some(sender) = self
            .as_mut()
            .rust_mut()
            .media_tool_requests
            .remove(&request_id)
        else {
            return;
        };
        let result = if error.is_empty() {
            Ok(MediaTools {
                ffmpeg: PathBuf::from(ffmpeg.to_string()),
                ffprobe: PathBuf::from(ffprobe.to_string()),
            })
        } else {
            Err(error.to_string())
        };
        let _ = sender.send(result);
        if self.captured_media_tool_request == Some(request_id) {
            self.as_mut().rust_mut().captured_media_tool_request = None;
        }
    }

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
        self.as_mut().resolve_download(request, queue, None, 0)
    }

    pub fn can_download_playback(&self, launch_token: u64) -> bool {
        self.download_queue.is_some()
            && self
                .playback
                .reference_for(launch_token)
                .is_some_and(|reference| reference.supports_video_acquisition())
    }

    pub fn download_playback(mut self: Pin<&mut Self>, launch_token: u64) -> bool {
        let Some(reference) = self.playback.reference_for(launch_token) else {
            return false;
        };
        if !reference.supports_video_acquisition() {
            return false;
        }
        let Some(queue) = self.download_queue.as_ref().map(Arc::clone) else {
            return false;
        };
        let Some(request) = self.source_request(SourceOperation::Resolve(reference)) else {
            return false;
        };
        self.as_mut()
            .resolve_download(request, queue, None, launch_token)
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
        if let Some(id) = self.as_mut().rust_mut().captured_media_tool_request.take() {
            self.as_mut().cancel_media_tools(id);
        }
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
        let choice = browser.choice();
        let generation = self.captured_download.generation;
        let task = self.services.as_ref().unwrap().runtime.spawn(async move {
            let prepared = async {
                let handle = browser.handle().await.map_err(|_| "capture")?;
                let media = resolve_captured_video(&handle, choice, page, capture_id.clone())
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
                prepare_acquisition(media).await.map_err(|_| "capture")
            }
            .await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.captured_download.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().captured_download.task = None;
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(reason) => {
                        controller.as_mut().captured_download_finished(
                            QString::from(capture_id),
                            QString::from(reason),
                        );
                        return;
                    }
                };
                let (direct, media_for_segmented) = match prepared {
                    PreparedAcquisition::Direct(gateway, playback) => {
                        match DirectDownload::captured(
                            source_host.clone(),
                            title.clone(),
                            playback,
                            gateway,
                        ) {
                            Ok(download) => (Some(download), None),
                            Err(_) => {
                                controller.as_mut().captured_download_finished(
                                    QString::from(capture_id),
                                    QString::from("unsupported"),
                                );
                                return;
                            }
                        }
                    }
                    PreparedAcquisition::Segmented(media) => (None, Some(media)),
                };
                let segmented = if direct.is_none() {
                    let request = controller.as_mut().request_media_tools();
                    controller.as_mut().rust_mut().captured_media_tool_request = Some(request.id);
                    Some(request)
                } else {
                    None
                };
                let Some(services) = &controller.services else {
                    return;
                };
                let ack_thread = controller.qt_thread();
                services.runtime.spawn(async move {
                    let outcome = if let Some(download) = direct {
                        queue.enqueue(download).await
                    } else {
                        let tools = segmented.unwrap().receive().await;
                        match tools.and_then(|tools| {
                            DirectDownload::segmented_capture(
                                source_host,
                                title,
                                media_for_segmented.unwrap(),
                                tools,
                            )
                        }) {
                            Ok(download) => queue.enqueue(download).await,
                            Err(error) => Err(error),
                        }
                    };
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
                        let reason = if let Err(error) = &outcome {
                            ffi::log_content_error(
                                &QString::from("capture download"),
                                &QString::from(error.to_string()),
                            );
                            "failed"
                        } else {
                            ""
                        };
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
        if let Some(id) = self.as_mut().rust_mut().captured_media_tool_request.take() {
            self.as_mut().cancel_media_tools(id);
        }
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
            .resolve_download(request, queue, Some(id.to_string()), 0)
    }

    fn resolve_download(
        mut self: Pin<&mut Self>,
        request: SourceRequest,
        queue: Arc<DownloadQueue<super::library::DownloadStore>>,
        retry_id: Option<String>,
        launch_token: u64,
    ) -> bool {
        let Some(services) = &self.services else {
            self.as_mut()
                .set_error(Some(ffi::ContentError::ServicesUnavailable));
            return false;
        };
        let catalog = Arc::clone(&services.catalog);
        let browser = services.browser.client();
        let choice = browser.choice();
        let qt_thread = self.qt_thread();
        services.runtime.spawn(async move {
            let result = catalog.execute(&request.operation).await;
            let external = if let Ok(SourceResponse::Resolve(ResolvedContent::BrowserVideo {
                media,
                ..
            })) = &result
            {
                Some(prepare_acquisition(media.clone()).await.map_err(|_| ()))
            } else if let Ok(SourceResponse::Resolve(ResolvedContent::ExternalVideo {
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
                            choice,
                            reference,
                            url.clone(),
                        )
                        .await
                        .map_err(|_| ())?;
                        prepare_acquisition(media).await.map_err(|_| ())
                    }
                    .await)
                } else {
                    None
                }
            } else {
                None
            };
            let direct_requires_mux = if let Ok(SourceResponse::Resolve(
                ResolvedContent::DirectVideo { url, quality, .. },
            )) = &result
            {
                let media = PlaybackMedia::direct(url.clone())
                    .with_video_height(quality.height());
                Some(DirectDownload::requires_mux(&media).await)
            } else {
                None
            };
            let _ = qt_thread.queue(move |mut controller| {
                if !controller.source_request_current(&request) {
                    controller.as_mut().playback_download_finished(launch_token, false);
                    return;
                }
                controller.as_mut().source_outcome(&request, &result);
                if !controller.source_request_current(&request) {
                    controller.as_mut().playback_download_finished(launch_token, false);
                    return;
                }

                let SourceOperation::Resolve(reference) = &request.operation else {
                    controller.as_mut().playback_download_finished(launch_token, false);
                    return;
                };
                let download = match result {
                    Ok(SourceResponse::Resolve(ResolvedContent::DirectVideo {
                        reference: resolved,
                        title,
                        url,
                        quality,
                        ..
                    })) if &resolved == reference => match direct_requires_mux {
                        Some(Ok(true)) => {
                            let media = PlaybackMedia::direct(url)
                                .with_video_height(quality.height());
                            Ok((None, Some((resolved, title, media))))
                        }
                        Some(Ok(false)) => DirectDownload::new(resolved, title, url)
                            .map(|download| (Some(download), None)),
                        Some(Err(error)) => Err(error),
                        None => Err(qbutt_content::DownloadError::InvalidSelection),
                    },
                    Ok(SourceResponse::Resolve(ResolvedContent::ExternalVideo {
                        reference: resolved,
                        title,
                        ..
                    }))
                    | Ok(SourceResponse::Resolve(ResolvedContent::BrowserVideo {
                        reference: resolved,
                        title,
                        ..
                    })) if &resolved == reference => match external {
                        Some(Ok(PreparedAcquisition::Direct(gateway, media))) => {
                            DirectDownload::prepared(resolved, title, media, gateway)
                                .map(|download| (Some(download), None))
                        }
                        Some(Ok(PreparedAcquisition::Segmented(media))) => {
                            Ok((None, Some((resolved, title, media))))
                        }
                        Some(Err(())) | None => Err(DownloadError::Stream),
                    },
                    _ => Err(DownloadError::InvalidSelection),
                };
                let (download, segmented) = match download {
                    Ok(download) => download,
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("download descriptor"),
                            &QString::from(error.to_string()),
                        );
                        controller
                            .as_mut()
                            .set_error(Some(ffi::ContentError::Download));
                        controller.as_mut().playback_download_finished(launch_token, false);
                        return;
                    }
                };
                let tools = segmented.as_ref().map(|_| controller.as_mut().request_media_tools());
                let Some(services) = &controller.services else {
                    controller.as_mut().playback_download_finished(launch_token, false);
                    return;
                };
                let error_thread = controller.qt_thread();
                services.runtime.spawn(async move {
                    let fresh = retry_id.is_none();
                    let download = if let Some(download) = download {
                        Ok(download)
                    } else {
                        let tools = tools.unwrap().receive().await;
                        tools.and_then(|tools| {
                            let (reference, title, media) = segmented.unwrap();
                            DirectDownload::segmented_catalog(reference, title, media, tools)
                        })
                    };
                    let outcome = match download {
                        Ok(download) => if let Some(id) = retry_id {
                            queue.retry(&id, download).await
                        } else {
                            queue.enqueue(download).await.map(|_| ())
                        },
                        Err(error) => Err(error),
                    };
                    let queued = outcome.is_ok();
                    if let Err(error) = outcome {
                        let detail = error.to_string();
                        ffi::log_content_error(
                            &QString::from("download queue"),
                            &QString::from(&detail),
                        );
                    }
                    let _ = error_thread.queue(move |mut controller| {
                        if !queued {
                            controller.as_mut().set_error(Some(ffi::ContentError::Download));
                        } else if fresh {
                            controller.as_mut().download_queued();
                        }
                        controller.as_mut().playback_download_finished(launch_token, queued);
                    });
                });
            });
        });
        true
    }
}
