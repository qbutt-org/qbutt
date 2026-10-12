use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::browser::BrowserChoice;
use qbutt_content::{
    DirectDownload, DownloadQueue, MediaGateway, ResolvedContent, SourceOperation, Transport,
    VariantSelector, resolve_external_video,
};

use super::catalog::SourceResponse;
use super::ffi;
use super::sources::SourceRequest;
use crate::download_model;

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
        let Some(services) = &self.services else {
            self.as_mut()
                .set_error(Some(ffi::ContentError::ServicesUnavailable));
            return;
        };
        let Some(worker) = &self.library_worker else {
            self.as_mut()
                .set_error(Some(ffi::ContentError::LibraryOpen));
            return;
        };

        let destination = PathBuf::from(destination.to_string());
        let storage = worker.download_store();
        let qt_thread = self.qt_thread();
        let task = services.runtime.spawn(async move {
            let queue = match DownloadQueue::open(storage, destination).await {
                Ok(queue) => Arc::new(queue),
                Err(error) => {
                    let detail = error.to_string();
                    let _ = qt_thread.queue(move |mut controller| {
                        ffi::log_content_error(&QString::from("downloads"), &QString::from(detail));
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
                    controller
                        .as_mut()
                        .rust_mut()
                        .downloads
                        .pin_mut()
                        .apply(initial);
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
                    }
                });
            });
        });
        true
    }
}
