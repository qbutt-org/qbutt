use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{
    LibraryResult, Source, SourceFailure, SourceFailureReason, SourceOperation, SourceSettings,
};

use super::catalog::SourceResponse;
use super::library::LibraryAction;
use super::{Pending, ffi};
use crate::source_model;

#[derive(Clone)]
pub enum SourceWrite {
    Enabled {
        source: Source,
        enabled: bool,
    },
    Failure(SourceFailure),
    Clear {
        operation: SourceOperation,
        observed_revision: i64,
    },
}

impl SourceWrite {
    pub fn source(&self) -> Source {
        match self {
            Self::Enabled { source, .. } => *source,
            Self::Failure(failure) => failure.operation.source(),
            Self::Clear { operation, .. } => operation.source(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveState {
    Pending,
    Failed,
}

struct SourceIntent {
    generation: u64,
    write: SourceWrite,
    save: SaveState,
}

struct SourceStatus {
    settings: SourceSettings,
    epoch: u64,
    enabled_intent: Option<SourceIntent>,
    health_intent: Option<SourceIntent>,
    pending: usize,
    retry: Pending,
}

impl SourceStatus {
    fn enabled(&self) -> bool {
        match self.enabled_intent.as_ref().map(|intent| &intent.write) {
            Some(SourceWrite::Enabled { enabled, .. }) => *enabled,
            _ => self.settings.enabled,
        }
    }

    fn failure(&self) -> Option<&SourceFailure> {
        match self.health_intent.as_ref().map(|intent| &intent.write) {
            Some(SourceWrite::Failure(failure)) => Some(failure),
            // A clear is confirmed only when SQLite acknowledged this revision.
            _ => self.settings.failure.as_ref(),
        }
    }

    fn row(&self) -> source_model::SourceRow {
        source_model::SourceRow {
            source: self.settings.source,
            enabled: self.enabled(),
            failure: self.failure().map(|failure| failure.reason),
            pending: self.pending != 0,
            retrying: self.retry.task.is_some(),
        }
    }

    fn intent_mut(&mut self, write: &SourceWrite) -> &mut Option<SourceIntent> {
        if matches!(write, SourceWrite::Enabled { .. }) {
            &mut self.enabled_intent
        } else {
            &mut self.health_intent
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Loading,
    Ready,
    Failed,
}

pub struct SourceState {
    statuses: Vec<SourceStatus>,
    load: LoadState,
    generation: u64,
    active_query: Option<String>,
    deferred_query: Option<String>,
    retry_writes_after_load: bool,
}

impl Default for SourceState {
    fn default() -> Self {
        Self {
            statuses: Source::ALL
                .into_iter()
                .map(|source| SourceStatus {
                    settings: SourceSettings {
                        source,
                        enabled: true,
                        revision: 0,
                        failure: None,
                    },
                    epoch: 0,
                    enabled_intent: None,
                    health_intent: None,
                    pending: 0,
                    retry: Pending::default(),
                })
                .collect(),
            load: LoadState::Ready,
            generation: 0,
            active_query: None,
            deferred_query: None,
            retry_writes_after_load: false,
        }
    }
}

impl SourceState {
    fn status(&self, source: Source) -> &SourceStatus {
        self.statuses
            .iter()
            .find(|status| status.settings.source == source)
            .unwrap()
    }

    fn status_mut(&mut self, source: Source) -> &mut SourceStatus {
        self.statuses
            .iter_mut()
            .find(|status| status.settings.source == source)
            .unwrap()
    }

    pub fn rows(&self) -> Vec<source_model::SourceRow> {
        self.statuses.iter().map(SourceStatus::row).collect()
    }

    pub fn cancel_retries(&mut self) {
        for status in &mut self.statuses {
            status.retry.cancel();
        }
    }
}

/// Policy generation rejects an outcome even after disable/enable ABA.
/// The persisted revision independently protects the exact failure being cleared.
#[derive(Clone)]
pub struct SourceRequest {
    pub operation: SourceOperation,
    epoch: u64,
    observed_revision: i64,
}

impl ffi::ContentController {
    pub(super) fn sources_model(&self) -> *mut source_model::ffi::SourceModel {
        self.sources.as_ptr().cast_mut()
    }

    pub(super) fn source_settings_state(&self) -> ffi::SettingsState {
        match self.source_status.load {
            LoadState::Loading => ffi::SettingsState::Loading,
            LoadState::Failed => ffi::SettingsState::Failed,
            LoadState::Ready => {
                if self.source_status.statuses.iter().any(|status| {
                    [&status.enabled_intent, &status.health_intent]
                        .into_iter()
                        .flatten()
                        .any(|intent| intent.save == SaveState::Failed)
                }) {
                    ffi::SettingsState::Failed
                } else if self
                    .source_status
                    .statuses
                    .iter()
                    .any(|status| status.pending != 0)
                {
                    ffi::SettingsState::Saving
                } else {
                    ffi::SettingsState::Ready
                }
            }
        }
    }

    pub(super) fn source_settings_error_text(&self) -> QString {
        if self.source_settings_state() != ffi::SettingsState::Failed {
            return QString::default();
        }
        source_model::ffi::source_error(if self.source_status.load == LoadState::Failed {
            4
        } else {
            5
        })
    }

    pub(super) fn source_enabled(&self, source: Source) -> bool {
        if source == Source::LocalBook {
            return self.local_book.is_some();
        }
        self.source_status.load == LoadState::Ready && self.source_status.status(source).enabled()
    }

    pub(super) fn source_request(&self, operation: SourceOperation) -> Option<SourceRequest> {
        let source = operation.source();
        if !self.source_enabled(source) || operation.validate().is_err() {
            return None;
        }
        if source == Source::LocalBook {
            return Some(SourceRequest {
                operation,
                epoch: 0,
                observed_revision: 0,
            });
        }
        let status = self.source_status.status(source);
        Some(SourceRequest {
            operation,
            epoch: status.epoch,
            observed_revision: status.settings.revision,
        })
    }

    pub(super) fn source_request_current(&self, request: &SourceRequest) -> bool {
        let source = request.operation.source();
        if source == Source::LocalBook {
            return self
                .local_book
                .as_ref()
                .is_some_and(|book| match &request.operation {
                    SourceOperation::Chapters(reference) => {
                        reference.publication == book.reference()
                    }
                    SourceOperation::Chapter(reference) => {
                        reference.variant.publication == book.reference()
                    }
                    _ => false,
                });
        }
        self.source_enabled(source) && self.source_status.status(source).epoch == request.epoch
    }

    pub(super) fn begin_source_search(mut self: Pin<&mut Self>, query: &str) -> bool {
        self.as_mut().rust_mut().source_status.active_query = Some(query.to_owned());
        if self.source_status.load == LoadState::Loading {
            self.as_mut().rust_mut().source_status.deferred_query = Some(query.to_owned());
            return false;
        }
        self.source_status.load == LoadState::Ready
    }

    pub(super) fn cancel_deferred_search(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().source_status.deferred_query = None;
    }

    pub(super) fn source_settings_loading(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().shared_import.cancel();
        self.as_mut().rust_mut().shared_works.clear();
        {
            let mut state = self.as_mut().rust_mut();
            state.source_status.load = LoadState::Loading;
            state.source_status.cancel_retries();
            for status in &mut state.source_status.statuses {
                status.epoch = status.epoch.wrapping_add(1);
            }
        }
        self.as_mut().cancel_search();
        self.as_mut().cancel_detail();
        self.as_mut()
            .rust_mut()
            .sources
            .pin_mut()
            .replace(Vec::new());
        self.sources_changed();
    }

    pub(super) fn load_source_settings(
        mut self: Pin<&mut Self>,
        result: Option<LibraryResult<Vec<SourceSettings>>>,
    ) {
        match result {
            Some(Ok(settings)) => {
                for settings in settings {
                    let source = settings.source;
                    self.as_mut()
                        .rust_mut()
                        .source_status
                        .status_mut(source)
                        .settings = settings;
                }
                let rows = self.source_status.rows();
                self.as_mut().rust_mut().sources.pin_mut().replace(rows);
                self.as_mut().rust_mut().source_status.load = LoadState::Ready;
                self.as_mut().sources_changed();
                if self.shared_document.is_some() {
                    self.as_mut().begin_shared_import();
                }
                if self.source_status.retry_writes_after_load {
                    self.as_mut()
                        .rust_mut()
                        .source_status
                        .retry_writes_after_load = false;
                    self.as_mut().retry_source_settings();
                }
                if self.source_status.load == LoadState::Ready {
                    let query = self.as_mut().rust_mut().source_status.deferred_query.take();
                    if let Some(query) = query {
                        self.search(QString::from(query));
                    }
                }
            }
            result => {
                if let Some(Err(error)) = result {
                    ffi::log_content_error(
                        &QString::from("source settings"),
                        &QString::from(error.to_string()),
                    );
                }
                self.as_mut().rust_mut().source_status.load = LoadState::Failed;
                self.as_mut().rust_mut().source_status.deferred_query = None;
                self.as_mut().sources_changed();
                if self.shared_document.is_some() {
                    let generation = self.shared_import.generation;
                    self.as_mut().set_shared_importing(false);
                    if self.shared_import.generation == generation {
                        self.as_mut().shared_link_failed(false);
                    }
                }
                if self.is_searching() {
                    self.as_mut().cancel_search();
                }
            }
        }
    }

    pub fn set_source_enabled(mut self: Pin<&mut Self>, id: QString, enabled: bool) -> bool {
        let Some(source) = Source::from_id(&id.to_string()) else {
            return false;
        };
        if self.source_status.load != LoadState::Ready {
            return false;
        }
        let status = self.source_status.status(source);
        if status.enabled() == enabled && status.enabled_intent.is_none() {
            return true;
        }
        {
            let mut state = self.as_mut().rust_mut();
            let status = state.source_status.status_mut(source);
            status.epoch = status.epoch.wrapping_add(1);
            status.retry.cancel();
        }
        let epoch = self.source_status.status(source).epoch;
        let accepted = self
            .as_mut()
            .submit_source_write(SourceWrite::Enabled { source, enabled });
        if !enabled && self.source_status.status(source).epoch == epoch {
            self.as_mut().close_disabled_reader(source);
        }
        if self.source_status.status(source).epoch == epoch
            && self.selected_group.as_ref().is_some_and(|group| {
                self.work_preferences
                    .get(&group.anchor)
                    .and_then(|preferences| preferences.selected.as_ref())
                    .is_some_and(|variant| variant.publication.source == source)
            })
        {
            let was_resolving = self.resolving;
            self.as_mut().rust_mut().resolution.cancel();
            self.as_mut().rust_mut().resolving = false;
            if was_resolving {
                self.as_mut().resolving_changed();
            }
            self.as_mut().selected_variant_state_changed();
        }
        accepted
    }

    fn publish_source(mut self: Pin<&mut Self>, source: Source) {
        let row = self.source_status.status(source).row();
        self.as_mut().rust_mut().sources.pin_mut().update(row);
        self.sources_changed();
    }

    fn submit_source_write(mut self: Pin<&mut Self>, write: SourceWrite) -> bool {
        let source = write.source();
        let generation = self.source_status.generation.wrapping_add(1);
        {
            let mut state = self.as_mut().rust_mut();
            state.source_status.generation = generation;
            *state.source_status.status_mut(source).intent_mut(&write) = Some(SourceIntent {
                generation,
                write: write.clone(),
                save: SaveState::Pending,
            });
        }
        if self.library_worker.is_none() && self.library_state == ffi::LibraryState::Uninitialized {
            let settings = {
                let mut state = self.as_mut().rust_mut();
                let settings = &mut state.source_status.status_mut(source).settings;
                match &write {
                    SourceWrite::Enabled { enabled, .. } if settings.enabled != *enabled => {
                        settings.enabled = *enabled;
                        settings.revision += 1;
                    }
                    SourceWrite::Failure(failure) => {
                        settings.failure = Some(failure.clone());
                        settings.revision += 1;
                    }
                    SourceWrite::Clear {
                        operation,
                        observed_revision,
                    } if settings.revision == *observed_revision
                        && settings.failure.as_ref().map(|failure| &failure.operation)
                            == Some(operation) =>
                    {
                        settings.failure = None;
                        settings.revision += 1;
                    }
                    _ => {}
                }
                settings.clone()
            };
            self.complete_source_write(write, generation, Ok(settings), false);
            return true;
        }
        self.as_mut()
            .rust_mut()
            .source_status
            .status_mut(source)
            .pending += 1;
        let accepted = self.as_mut().submit_library(LibraryAction::SourceWrite {
            write: write.clone(),
            generation,
        });
        {
            let mut state = self.as_mut().rust_mut();
            let status = state.source_status.status_mut(source);
            if !accepted {
                status.pending -= 1;
                if let Some(intent) = status.intent_mut(&write)
                    && intent.generation == generation
                {
                    intent.save = SaveState::Failed;
                }
            }
        }
        self.publish_source(source);
        accepted
    }

    pub(super) fn complete_source_write(
        mut self: Pin<&mut Self>,
        write: SourceWrite,
        generation: u64,
        result: LibraryResult<SourceSettings>,
        accepted: bool,
    ) {
        let source = write.source();
        if let Err(error) = &result {
            ffi::log_content_error(
                &QString::from("source settings"),
                &QString::from(error.to_string()),
            );
        }
        {
            let mut state = self.as_mut().rust_mut();
            let status = state.source_status.status_mut(source);
            if accepted {
                status.pending -= 1;
            }
            let succeeded = result.is_ok();
            if let Ok(settings) = result
                && settings.revision >= status.settings.revision
            {
                status.settings = settings;
            }
            let intent = status.intent_mut(&write);
            if intent
                .as_ref()
                .is_some_and(|intent| intent.generation == generation)
            {
                if succeeded {
                    *intent = None;
                } else {
                    intent.as_mut().unwrap().save = SaveState::Failed;
                }
            }
        }
        self.publish_source(source);
    }

    pub(super) fn source_outcome<T>(
        self: Pin<&mut Self>,
        request: &SourceRequest,
        result: &qbutt_content::Result<T>,
    ) {
        if request.operation.source() == Source::LocalBook {
            return;
        }
        if !self.source_request_current(request) {
            return;
        }
        if let Err(error) = result {
            if let Some(reason) = SourceFailureReason::from_error(error) {
                self.submit_source_write(SourceWrite::Failure(SourceFailure {
                    operation: request.operation.clone(),
                    reason,
                }));
            }
        } else {
            let status = self.source_status.status(request.operation.source());
            if status.health_intent.is_none()
                && status.settings.revision == request.observed_revision
                && status
                    .settings
                    .failure
                    .as_ref()
                    .map(|failure| &failure.operation)
                    == Some(&request.operation)
            {
                self.submit_source_write(SourceWrite::Clear {
                    operation: request.operation.clone(),
                    observed_revision: request.observed_revision,
                });
            }
        }
    }

    pub fn retry_source_settings(mut self: Pin<&mut Self>) -> bool {
        if self.source_settings_state() != ffi::SettingsState::Failed {
            return false;
        }
        if self.library_state == ffi::LibraryState::Failed {
            self.as_mut()
                .rust_mut()
                .source_status
                .retry_writes_after_load = true;
            let accepted = self.as_mut().retry_library();
            if !accepted {
                self.as_mut()
                    .rust_mut()
                    .source_status
                    .retry_writes_after_load = false;
            }
            return accepted;
        }
        if self.source_status.load == LoadState::Failed {
            self.as_mut().source_settings_loading();
            let accepted = self.as_mut().submit_library(LibraryAction::SourceSettings);
            if !accepted {
                self.load_source_settings(None);
            }
            return accepted;
        }
        let writes: Vec<_> = self
            .source_status
            .statuses
            .iter()
            .flat_map(|status| {
                [&status.enabled_intent, &status.health_intent]
                    .into_iter()
                    .flatten()
                    .filter(|intent| intent.save == SaveState::Failed)
                    .map(|intent| (intent.generation, intent.write.clone()))
            })
            .collect();
        let mut accepted = !writes.is_empty();
        for (generation, write) in writes {
            let status = self.source_status.status(write.source());
            let current = if matches!(write, SourceWrite::Enabled { .. }) {
                &status.enabled_intent
            } else {
                &status.health_intent
            };
            if current.as_ref().is_some_and(|intent| {
                intent.generation == generation && intent.save == SaveState::Failed
            }) {
                accepted &= self.as_mut().submit_source_write(write);
            }
        }
        accepted
    }

    pub fn retry_source(mut self: Pin<&mut Self>, id: QString) -> bool {
        let Some(source) = Source::from_id(&id.to_string()) else {
            return false;
        };
        let status = self.source_status.status(source);
        if !self.source_enabled(source)
            || status.pending != 0
            || status.retry.task.is_some()
            || status.health_intent.is_some()
            || status.enabled_intent.is_some()
        {
            return false;
        }
        let Some(failure) = status.settings.failure.clone() else {
            return false;
        };
        let manual_check = source == Source::RuTracker
            && matches!(
                failure.reason,
                SourceFailureReason::LoginRequired | SourceFailureReason::SourceCheckRequired
            );
        let Some(request) = self.source_request(failure.operation) else {
            return false;
        };
        let Some(services) = self.services.as_ref() else {
            return false;
        };
        let catalog = Arc::clone(&services.catalog);
        let thread = self.qt_thread();
        let search_generation = self.search.generation;
        let inspection_generation = self.inspection.generation;
        let retry_generation = status.retry.generation;
        let task = services.runtime.spawn(async move {
            let result = if manual_check {
                match catalog.complete_source_check(source).await {
                    Ok(()) => catalog.execute(&request.operation).await,
                    Err(error) => Err(error),
                }
            } else {
                catalog.execute(&request.operation).await
            };
            let _ = thread.queue(move |mut controller| {
                if !controller.source_request_current(&request)
                    || controller.source_status.status(source).retry.generation != retry_generation
                {
                    return;
                }
                controller
                    .as_mut()
                    .rust_mut()
                    .source_status
                    .status_mut(source)
                    .retry
                    .task = None;
                controller.as_mut().source_outcome(&request, &result);
                if !controller.source_request_current(&request) {
                    return;
                }
                match (&request.operation, result) {
                    (SourceOperation::Search { query, .. }, Ok(SourceResponse::Search(works)))
                        if controller.search.generation == search_generation
                            && controller.source_status.active_query.as_ref() == Some(query) =>
                    {
                        for work in works {
                            if !controller.source_request_current(&request)
                                || controller.search.generation != search_generation
                            {
                                break;
                            }
                            controller
                                .as_mut()
                                .rust_mut()
                                .search_association
                                .update(work);
                        }
                        controller.as_mut().refresh_search_groups(search_generation);
                    }
                    (
                        SourceOperation::Inspect { reference, .. },
                        Ok(SourceResponse::Inspect(work)),
                    ) if controller.inspection.generation == inspection_generation
                        && controller
                            .selected_group
                            .as_ref()
                            .is_some_and(|group| group.references.contains(reference)) =>
                    {
                        controller
                            .as_mut()
                            .publish_inspected_work(work, inspection_generation);
                    }
                    // Retrying resolution checks availability; it never starts playback/download.
                    _ => {}
                }
                controller.publish_source(source);
            });
        });
        self.as_mut()
            .rust_mut()
            .source_status
            .status_mut(source)
            .retry
            .task = Some(task);
        self.publish_source(source);
        true
    }
}
