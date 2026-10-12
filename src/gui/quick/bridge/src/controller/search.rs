use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{Source, SourceOperation};
use tokio::task::JoinSet;

use super::catalog::SourceResponse;
use super::ffi;

const MAX_SEARCH_REQUESTS: usize = 4;

impl ffi::ContentController {
    pub fn search(mut self: Pin<&mut Self>, query: QString) {
        self.as_mut().cancel_shared_import();
        if self.source_settings_state() == ffi::SettingsState::Failed {
            return;
        }
        let query = query.to_string().trim().to_owned();
        self.as_mut().cancel_deferred_search();
        self.as_mut().rust_mut().search.cancel();
        let generation = self.search.generation;
        let previous_selection_generation = self.inspection.generation;
        let previous_resolution_generation = self.resolution.generation;
        self.as_mut().set_search_state(ffi::SearchState::Idle);
        if self.search.generation != generation
            || self.inspection.generation != previous_selection_generation
            || self.resolution.generation != previous_resolution_generation
        {
            return;
        }
        let (selection_generation, resolution_generation) = self.as_mut().cancel_detail();
        if self.search.generation != generation
            || self.inspection.generation != selection_generation
            || self.resolution.generation != resolution_generation
        {
            return;
        }

        self.as_mut().rust_mut().search_association.clear();
        self.as_mut().rust_mut().works.pin_mut().clear();
        if self.search.generation != generation
            || self.inspection.generation != selection_generation
            || self.resolution.generation != resolution_generation
        {
            return;
        }
        self.as_mut().rust_mut().variants.pin_mut().clear();
        if self.search.generation != generation
            || self.inspection.generation != selection_generation
            || self.resolution.generation != resolution_generation
        {
            return;
        }
        if !self.as_mut().set_selection(None, selection_generation)
            || self.search.generation != generation
        {
            return;
        }
        self.as_mut().set_error(None);
        if self.search.generation != generation {
            return;
        }
        if query.is_empty() {
            return;
        }
        if self.services.is_none() {
            self.as_mut()
                .set_error(Some(ffi::ContentError::ServicesUnavailable));
            if self.search.generation == generation {
                self.as_mut().set_search_state(ffi::SearchState::Failed);
            }
            return;
        }
        self.as_mut().set_search_state(ffi::SearchState::Searching);
        if self.search.generation != generation {
            return;
        }
        if !self.as_mut().begin_source_search(&query) {
            if self.source_settings_state() == ffi::SettingsState::Failed {
                self.as_mut().set_search_state(ffi::SearchState::Cancelled);
            }
            return;
        }

        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let catalog = Arc::clone(&services.catalog);
        let mut requests = Vec::new();
        for source in Source::ALL {
            for category in source.categories() {
                if let Some(request) = self.source_request(SourceOperation::Search {
                    source,
                    query: query.clone(),
                    category: *category,
                    language: (source == Source::MangaDex).then(|| services.language.clone()),
                }) {
                    requests.push(request);
                }
            }
        }
        let task = services.runtime.spawn(async move {
            let had_requests = !requests.is_empty();
            let mut requests = requests.into_iter();
            let mut pending = JoinSet::new();
            let mut any_succeeded = false;
            loop {
                while pending.len() < MAX_SEARCH_REQUESTS
                    && let Some(request) = requests.next()
                {
                    let catalog = Arc::clone(&catalog);
                    pending.spawn(async move {
                        let result = catalog.execute(&request.operation).await;
                        (request, result)
                    });
                }
                let Some(completed) = pending.join_next().await else {
                    break;
                };
                let (request, result) = match completed {
                    Ok(completed) => completed,
                    Err(error) => {
                        let _ = qt_thread.queue(move |controller| {
                            if controller.search.generation == generation {
                                ffi::log_content_error(
                                    &QString::from("search"),
                                    &QString::from(error.to_string()),
                                );
                            }
                        });
                        continue;
                    }
                };
                any_succeeded |= result.is_ok();
                let _ = qt_thread.queue(move |mut controller| {
                    if controller.search.generation != generation
                        || !controller.source_request_current(&request)
                    {
                        return;
                    }
                    controller.as_mut().source_outcome(&request, &result);
                    if controller.search.generation != generation
                        || !controller.source_request_current(&request)
                    {
                        return;
                    }
                    if let Ok(SourceResponse::Search(works)) = result {
                        for work in works {
                            if controller.search.generation != generation
                                || !controller.source_request_current(&request)
                            {
                                return;
                            }
                            controller
                                .as_mut()
                                .rust_mut()
                                .search_association
                                .update(work);
                        }
                        controller.as_mut().refresh_search_groups(generation);
                    }
                });
            }
            let _ = qt_thread.queue(move |mut controller| {
                if controller.search.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().search.task = None;
                controller
                    .as_mut()
                    .set_search_state(if any_succeeded || !had_requests {
                        ffi::SearchState::Succeeded
                    } else {
                        ffi::SearchState::Failed
                    });
            });
        });
        self.as_mut().rust_mut().search.task = Some(task);
    }

    pub(super) fn refresh_search_groups(mut self: Pin<&mut Self>, generation: u64) {
        if self.search.generation != generation {
            return;
        }
        let mut groups = self.search_association.groups();
        for group in &mut groups {
            if let Some(saved) = group
                .references
                .iter()
                .find(|reference| self.known_groups.contains_key(*reference))
            {
                group.anchor = saved.clone();
            }
        }
        if self.list_mode == ffi::ListMode::Search
            && let Some(selected) = &self.selected_group
        {
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group.references.contains(&selected.anchor))
            {
                group.anchor = selected.anchor.clone();
                for publication in &mut group.publications {
                    if publication.needs_inspection
                        && let Some(inspected) = selected.publication(&publication.reference)
                        && !inspected.needs_inspection
                    {
                        *publication = inspected.clone();
                    }
                }
                for reference in &selected.references {
                    if selected.publication(reference).is_none()
                        && !group.references.contains(reference)
                    {
                        group.references.push(reference.clone());
                    }
                }
            }
        }
        let visible: HashSet<_> = groups.iter().map(|group| group.anchor.clone()).collect();
        for reference in self.works.references() {
            if !visible.contains(&reference) {
                self.as_mut().rust_mut().works.pin_mut().remove(&reference);
                if self.search.generation != generation {
                    return;
                }
            }
        }
        for group in &groups {
            if let Some(work) = group.anchor_work() {
                self.as_mut()
                    .rust_mut()
                    .works
                    .pin_mut()
                    .update(work.clone());
                if self.search.generation != generation {
                    return;
                }
            }
        }
        if self.list_mode == ffi::ListMode::Search
            && let Some(selected) = &self.selected_group
            && let Some(group) = groups.iter().find(|group| group.anchor == selected.anchor)
        {
            let selection_generation = self.inspection.generation;
            self.as_mut().show_group(group, selection_generation);
        }
    }

    pub fn cancel_search(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().search.cancel();
        self.as_mut().cancel_deferred_search();
        if self.is_searching() {
            self.as_mut().set_search_state(ffi::SearchState::Cancelled);
        }
    }
}
