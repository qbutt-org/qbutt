use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{Source, SourceOperation, WorkGroup};
use tokio::task::JoinSet;

use super::catalog::SourceResponse;
use super::ffi;
use super::library::LibraryAction;

const MAX_INSPECTION_REQUESTS: usize = 4;

impl ffi::ContentController {
    pub(super) fn inspect_publications(
        mut self: Pin<&mut Self>,
        group: &WorkGroup,
        generation: u64,
    ) -> bool {
        if self.inspection.generation != generation || self.inspection.task.is_some() {
            return false;
        }
        let Some(services) = self.services.as_ref() else {
            return false;
        };
        let (references, requests): (Vec<_>, Vec<_>) = group
            .publications
            .iter()
            .filter(|work| {
                work.needs_inspection && !self.inspection_attempted.contains(&work.reference)
            })
            .filter_map(|work| {
                self.source_request(SourceOperation::Inspect {
                    reference: work.reference.clone(),
                    language: (work.reference.source == Source::MangaDex)
                        .then(|| services.language.clone()),
                })
                .map(|request| (work.reference.clone(), request))
            })
            .unzip();
        if requests.is_empty() {
            return false;
        }
        let qt_thread = self.qt_thread();
        let catalog = Arc::clone(&services.catalog);
        let task = services.runtime.spawn(async move {
            let mut requests = requests.into_iter();
            let mut pending = JoinSet::new();
            loop {
                while pending.len() < MAX_INSPECTION_REQUESTS
                    && let Some(request) = requests.next()
                {
                    let catalog = Arc::clone(&catalog);
                    pending.spawn(async move {
                        let result = catalog.execute(&request.operation).await;
                        (request, result)
                    });
                }
                let (request, result) = match pending.join_next().await {
                    Some(Ok(completed)) => completed,
                    Some(Err(error)) => {
                        let _ = qt_thread.queue(move |controller| {
                            if controller.inspection.generation == generation {
                                ffi::log_content_error(
                                    &QString::from("inspect"),
                                    &QString::from(error.to_string()),
                                );
                            }
                        });
                        continue;
                    }
                    None => break,
                };
                let _ = qt_thread.queue(move |mut controller| {
                    if controller.inspection.generation != generation
                        || !controller.source_request_current(&request)
                    {
                        return;
                    }
                    controller.as_mut().source_outcome(&request, &result);
                    if controller.inspection.generation != generation
                        || !controller.source_request_current(&request)
                    {
                        return;
                    }
                    if let Ok(SourceResponse::Inspect(work)) = result {
                        controller.as_mut().publish_inspected_work(work, generation);
                    }
                });
            }
            let _ = qt_thread.queue(move |mut controller| {
                if controller.inspection.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().inspection.task = None;
                // Search may have associated more publications while this batch was in flight.
                if let Some(group) = controller.selected_group.clone()
                    && controller.as_mut().inspect_publications(&group, generation)
                {
                    return;
                }
                controller.as_mut().rust_mut().inspecting = false;
                controller.as_mut().inspecting_changed();
                if controller.inspection.generation != generation {
                    return;
                }
                controller.as_mut().selected_variant_state_changed();
                if controller.inspection.generation == generation
                    && controller.error_kind.is_none()
                    && let Some(group) = controller.selected_group.clone()
                {
                    controller
                        .as_mut()
                        .submit_library(LibraryAction::Refresh { group });
                }
            });
        });
        let mut state = self.as_mut().rust_mut();
        state.inspection_attempted.extend(references);
        state.inspection.task = Some(task);
        state.inspecting = true;
        true
    }
}
