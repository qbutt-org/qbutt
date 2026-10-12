// SPDX-License-Identifier: GPL-2.0-or-later
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{ShareError, SharedWork, Source, SourceOperation, Work};
use tokio::task::JoinSet;

use super::catalog::SourceResponse;
use super::ffi;

impl ffi::ContentController {
    fn selected_share_document(&self) -> Result<SharedWork, ShareError> {
        let publications: Vec<_> = self
            .selected_work_publications()
            .into_iter()
            .filter(SharedWork::is_publication_shareable)
            .collect();
        let preferred = self
            .selected_share_preference()
            .filter(|reference| publications.contains(&reference.publication));
        SharedWork::new(publications, preferred)
    }

    pub(super) fn can_share_selected(&self) -> bool {
        self.selected_share_document()
            .and_then(|document| document.to_url())
            .is_ok()
    }

    pub fn share_selected(self: Pin<&mut Self>) -> bool {
        let Ok(url) = self.selected_share_document().and_then(|document| document.to_url()) else {
            return false;
        };
        ffi::copy_share_url(&QString::from(url));
        true
    }

    pub fn open_shared_link(mut self: Pin<&mut Self>, url: QString) {
        self.as_mut().cancel_shared_import();
        let document = match SharedWork::from_url(&url.to_string()) {
            Ok(document) => document,
            Err(_) => {
                self.as_mut().shared_link_failed(true);
                return;
            }
        };
        self.as_mut().cancel_search();
        self.as_mut().rust_mut().shared_document = Some(document);
        self.as_mut().begin_shared_import();
    }

    pub(super) fn cancel_shared_import(mut self: Pin<&mut Self>) {
        let state = self.as_mut().rust_mut();
        state.shared_import.cancel();
        state.shared_document = None;
        state.shared_works.clear();
    }

    pub(super) fn begin_shared_import(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().shared_import.cancel();
        self.as_mut().rust_mut().shared_works.clear();
        let Some(document) = self.shared_document.clone() else {
            return;
        };
        if self.source_settings_state() == ffi::SettingsState::Loading {
            return;
        }
        let Some(services) = self.services.as_ref() else {
            self.as_mut().shared_link_failed(false);
            return;
        };
        let mut requests = Vec::new();
        for reference in document.publications() {
            let operation = SourceOperation::Inspect {
                reference: reference.clone(),
                language: (reference.source == Source::MangaDex).then(|| services.language.clone()),
            };
            if let Some(request) = self.source_request(operation) {
                requests.push(request);
            }
        }
        if requests.is_empty() {
            self.as_mut().shared_link_failed(false);
            return;
        }

        let generation = self.shared_import.generation;
        let qt_thread = self.qt_thread();
        let catalog = Arc::clone(&services.catalog);
        let task = services.runtime.spawn(async move {
            let mut pending = requests.into_iter();
            let mut active = JoinSet::new();
            for _ in 0..4 {
                let Some(request) = pending.next() else {
                    break;
                };
                let catalog = Arc::clone(&catalog);
                active.spawn(async move {
                    let result = catalog.execute(&request.operation).await;
                    (request, result)
                });
            }
            while let Some(result) = active.join_next().await {
                if let Ok((request, result)) = result {
                    let _ = qt_thread.queue(move |mut controller| {
                        if controller.shared_import.generation != generation
                            || !controller.source_request_current(&request)
                        {
                            return;
                        }
                        controller.as_mut().source_outcome(&request, &result);
                        if controller.shared_import.generation != generation {
                            return;
                        }
                        if let (
                            SourceOperation::Inspect { reference, .. },
                            Ok(SourceResponse::Inspect(work)),
                        ) = (&request.operation, result)
                            && &work.reference == reference
                        {
                            let Some(document) = controller.shared_document.clone() else {
                                return;
                            };
                            let publications = document.publications().to_vec();
                            controller.as_mut().rust_mut().shared_works.push(work);
                            let works = controller.shared_works.clone();
                            if !controller.as_mut().show_shared_publications(
                                publications,
                                works,
                                document.preferred().cloned(),
                            ) {
                                controller.as_mut().rust_mut().shared_works.pop();
                            }
                        }
                    });
                }
                if let Some(request) = pending.next() {
                    let catalog = Arc::clone(&catalog);
                    active.spawn(async move {
                        let result = catalog.execute(&request.operation).await;
                        (request, result)
                    });
                }
            }
            let _ = qt_thread.queue(move |mut controller| {
                if controller.shared_import.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().shared_import.task = None;
                if controller.shared_works.is_empty() {
                    controller.as_mut().shared_link_failed(false);
                    return;
                }
                controller.as_mut().rust_mut().shared_document = None;
                controller.as_mut().rust_mut().shared_works.clear();
            });
        });
        self.as_mut().rust_mut().shared_import.task = Some(task);
    }
}
