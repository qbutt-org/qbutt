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
        let Ok(url) = self
            .selected_share_document()
            .and_then(|document| document.to_url())
        else {
            return false;
        };
        ffi::copy_share_url(&QString::from(url));
        true
    }

    pub fn open_shared_link(mut self: Pin<&mut Self>, url: QString) {
        self.as_mut().cancel_shared_import();
        let import_generation = self.shared_import.generation;
        let document = match SharedWork::from_url(&url.to_string()) {
            Ok(document) => document,
            Err(_) => {
                self.as_mut().shared_link_failed(true);
                return;
            }
        };
        self.as_mut().cancel_search();
        if self.shared_import.generation != import_generation {
            return;
        }
        self.as_mut().rust_mut().shared_document = Some(document);
        self.as_mut().set_shared_importing(true);
        if self.shared_document.is_none() || self.shared_import.generation != import_generation {
            return;
        }

        let (generation, _) = self.as_mut().cancel_detail();
        if !self.shared_open_current(import_generation, generation) {
            return;
        }
        if self.list_mode != ffi::ListMode::Search {
            self.as_mut().rust_mut().list_mode = ffi::ListMode::Search;
            self.as_mut().list_mode_changed();
        }
        if !self.shared_open_current(import_generation, generation) {
            return;
        }
        self.as_mut().set_search_state(ffi::SearchState::Idle);
        if !self.shared_open_current(import_generation, generation) {
            return;
        }
        self.as_mut().set_error(None);
        if !self.shared_open_current(import_generation, generation) {
            return;
        }
        self.as_mut().rust_mut().search_association.clear();
        self.as_mut().rust_mut().works.pin_mut().clear();
        if !self.shared_open_current(import_generation, generation) {
            return;
        }
        self.as_mut().rust_mut().variants.pin_mut().clear();
        if !self.shared_open_current(import_generation, generation)
            || !self.as_mut().set_selection(None, generation)
            || !self.shared_open_current(import_generation, generation)
        {
            return;
        }
        self.as_mut().begin_shared_import();
    }

    fn shared_open_current(&self, import_generation: u64, selection_generation: u64) -> bool {
        self.shared_document.is_some()
            && self.shared_import.generation == import_generation
            && self.inspection.generation == selection_generation
    }

    pub(super) fn cancel_shared_import(mut self: Pin<&mut Self>) {
        {
            let state = self.as_mut().rust_mut();
            state.shared_import.cancel();
            state.shared_document = None;
            state.shared_works.clear();
        }
        self.as_mut().set_shared_importing(false);
    }

    pub(super) fn set_shared_importing(mut self: Pin<&mut Self>, importing: bool) {
        if self.shared_importing != importing {
            self.as_mut().rust_mut().shared_importing = importing;
            self.as_mut().shared_importing_changed();
        }
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
        let generation = self.shared_import.generation;
        self.as_mut().set_shared_importing(true);
        if self.shared_import.generation != generation || self.shared_document.is_none() {
            return;
        }
        let Some(services) = self.services.as_ref() else {
            self.as_mut().set_shared_importing(false);
            if self.shared_import.generation == generation {
                self.as_mut().shared_link_failed(false);
            }
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
        let preferred_request = document.preferred().and_then(|preferred| {
            requests.iter().position(|request| {
                matches!(&request.operation, SourceOperation::Inspect { reference, .. }
                    if reference == &preferred.publication)
            })
        });
        if let Some(index) = preferred_request {
            requests.swap(0, index);
        }
        if requests.is_empty() {
            self.as_mut().set_shared_importing(false);
            if self.shared_import.generation == generation {
                self.as_mut().shared_link_failed(false);
            }
            return;
        }

        let qt_thread = self.qt_thread();
        let catalog = Arc::clone(&services.catalog);
        let initial_parallelism = if preferred_request.is_some() { 1 } else { 4 };
        let task = services.runtime.spawn(async move {
            let mut pending = requests.into_iter();
            let mut active = JoinSet::new();
            for _ in 0..initial_parallelism {
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
                            if controller.shared_works.is_empty() && document.preferred().is_some()
                            {
                                controller
                                    .as_mut()
                                    .rust_mut()
                                    .work_preferences
                                    .entry(work.reference.clone())
                                    .or_default()
                                    .selected = None;
                            }
                            controller.as_mut().rust_mut().shared_works.push(work);
                            let works = controller.shared_works.clone();
                            let publications: Vec<_> = document
                                .publications()
                                .iter()
                                .filter(|reference| {
                                    works.iter().any(|work| &work.reference == *reference)
                                })
                                .cloned()
                                .collect();
                            let preferred = document
                                .preferred()
                                .filter(|variant| publications.contains(&variant.publication))
                                .cloned();
                            if !controller.as_mut().show_shared_publications(
                                publications,
                                works,
                                preferred,
                            ) {
                                controller.as_mut().rust_mut().shared_works.pop();
                            }
                        }
                    });
                }
                while active.len() < 4
                    && let Some(request) = pending.next()
                {
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
                controller.as_mut().set_shared_importing(false);
                if controller.shared_import.generation != generation {
                    return;
                }
                if controller.shared_works.is_empty() {
                    controller.as_mut().shared_link_failed(false);
                    return;
                }
                let complete = controller.shared_document.as_ref().is_some_and(|document| {
                    controller.shared_works.len() == document.publications().len()
                });
                controller.as_mut().rust_mut().shared_works.clear();
                if complete {
                    controller.as_mut().rust_mut().shared_document = None;
                } else {
                    controller.as_mut().shared_link_failed(false);
                }
            });
        });
        self.as_mut().rust_mut().shared_import.task = Some(task);
    }
}
