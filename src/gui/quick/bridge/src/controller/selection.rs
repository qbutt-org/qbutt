use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{Source, SourceOperation, Work};

use super::catalog::SourceResponse;
use super::ffi;
use super::library::LibraryAction;
use crate::models::{self, variant_id, work_id};

impl ffi::ContentController {
    pub(super) fn selected_work_category(&self) -> QString {
        let category = self
            .selected_work
            .as_ref()
            .map(|work| models::category_key(work.category))
            .unwrap_or_default();
        QString::from(category)
    }

    pub(super) fn selected_variant_kind(&self) -> QString {
        let transport = self
            .variants
            .find(&self.selected_variant_id.to_string())
            .map(|variant| variant.transport);
        QString::from(match transport {
            Some(qbutt_content::Transport::Torrent) => "torrent",
            Some(qbutt_content::Transport::Video) => "video",
            Some(qbutt_content::Transport::Manga) => "manga",
            None => "",
        })
    }

    pub(super) fn variant_selection_state(&self) -> ffi::VariantSelectionState {
        if self.inspecting {
            return ffi::VariantSelectionState::Inspecting;
        }
        if self.selected_variant_id.is_empty() {
            return ffi::VariantSelectionState::None;
        }
        let Some(variant) = self.variants.find(&self.selected_variant_id.to_string()) else {
            return ffi::VariantSelectionState::Unavailable;
        };
        if self.source_enabled(variant.reference.publication.source) {
            ffi::VariantSelectionState::Available
        } else {
            ffi::VariantSelectionState::SourceDisabled
        }
    }

    pub fn set_list_mode(mut self: Pin<&mut Self>, mode: ffi::ListMode) {
        if self.list_mode != mode {
            self.as_mut().rust_mut().list_mode = mode;
            self.as_mut().list_mode_changed();
        }
    }

    pub fn select_work(mut self: Pin<&mut Self>, id: QString) {
        let model = match self.list_mode {
            ffi::ListMode::Favorites => &self.favorites,
            ffi::ListMode::History => &self.history,
            _ => &self.works,
        };
        let Some(mut work) = model.find(&id.to_string()).cloned() else {
            return;
        };
        if self.list_mode != ffi::ListMode::Search {
            work.needs_inspection = true;
        }
        if self.selected_work_id == id && (self.inspecting || !work.needs_inspection) {
            return;
        }

        let (generation, resolution_generation) = self.as_mut().cancel_detail();
        if self.inspection.generation != generation
            || self.resolution.generation != resolution_generation
        {
            return;
        }
        if self.selected_work_id != id {
            self.as_mut().rust_mut().variants.pin_mut().clear();
            if self.inspection.generation != generation
                || self.resolution.generation != resolution_generation
            {
                return;
            }
        }
        let request = self.services.as_ref().and_then(|services| {
            self.source_request(SourceOperation::Inspect {
                reference: work.reference.clone(),
                language: (work.reference.source == Source::MangaDex)
                    .then(|| services.language.clone()),
            })
        });
        let should_inspect = work.needs_inspection && request.is_some();
        // Cached variants cannot become actionable during synchronous selection notifications.
        self.as_mut().rust_mut().inspecting = should_inspect;
        if !self.as_mut().show_work(&work, generation) {
            return;
        }
        self.as_mut().set_error(None);
        if self.inspection.generation != generation {
            return;
        }
        if should_inspect {
            self.as_mut().inspecting_changed();
        }
        if self.inspection.generation != generation {
            return;
        }
        if !work.needs_inspection {
            self.as_mut().select_initial_variant();
            return;
        }
        if self.services.is_none() {
            self.as_mut()
                .set_error(Some(ffi::ContentError::ServicesUnavailable));
            return;
        }
        let Some(request) = request else {
            return;
        };
        if !self.source_request_current(&request) {
            self.as_mut().set_inspecting(false);
            return;
        }

        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let catalog = Arc::clone(&services.catalog);
        let task = services.runtime.spawn(async move {
            let result = catalog.execute(&request.operation).await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.inspection.generation != generation
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                controller.as_mut().rust_mut().inspection.task = None;
                controller.as_mut().source_outcome(&request, &result);
                if controller.inspection.generation != generation
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                if let Ok(SourceResponse::Inspect(work)) = result {
                    if !controller.as_mut().show_work(&work, generation) {
                        return;
                    }
                    if controller.works.find(&work_id(&work.reference)).is_some() {
                        controller.as_mut().rust_mut().works.pin_mut().update(work);
                    }
                }
                if controller.inspection.generation == generation {
                    controller.as_mut().set_inspecting(false);
                    if controller.inspection.generation == generation
                        && controller.error_kind.is_none()
                    {
                        if controller.selected_variant_id.is_empty() {
                            controller.as_mut().select_initial_variant();
                        } else {
                            controller.as_mut().save_selected_preferences();
                        }
                    }
                }
            });
        });
        self.as_mut().rust_mut().inspection.task = Some(task);
    }

    pub fn select_variant(mut self: Pin<&mut Self>, id: QString) {
        if self.selected_variant_id == id || self.inspecting {
            return;
        }
        let Some(variant) = self.variants.find(&id.to_string()) else {
            return;
        };
        let Some(work) = self.selected_work.clone() else {
            return;
        };
        let reference = variant.reference.clone();
        let generation = self.inspection.generation;
        let was_resolving = self.resolving;
        let preferences = {
            let mut state = self.as_mut().rust_mut();
            state.resolution.cancel();
            state.resolving = false;
            state.selected_variant_id = id.clone();
            let preferences = state
                .work_preferences
                .entry(work.reference.clone())
                .or_default();
            preferences.selected = Some(reference);
            preferences.clone()
        };
        let resolution_generation = self.resolution.generation;
        // Accept the captured per-work intent before any notification can select
        // another work. An acknowledgement never owns the current UI choice.
        self.as_mut()
            .submit_library(LibraryAction::Preferences { work, preferences });
        if self.inspection.generation != generation
            || self.resolution.generation != resolution_generation
            || self.selected_variant_id != id
        {
            return;
        }
        self.as_mut().close_reader();
        if self.inspection.generation != generation
            || self.resolution.generation != resolution_generation
            || self.selected_variant_id != id
        {
            return;
        }
        if was_resolving {
            self.as_mut().resolving_changed();
        }
        if self.inspection.generation != generation
            || self.resolution.generation != resolution_generation
            || self.selected_variant_id != id
        {
            return;
        }
        self.as_mut().selected_variant_id_changed();
        if self.inspection.generation == generation
            && self.resolution.generation == resolution_generation
            && self.selected_variant_id == id
        {
            self.as_mut().selected_variant_state_changed();
        }
    }

    pub fn set_selected_favorite(mut self: Pin<&mut Self>, favorite: bool) {
        let Some(work) = self.selected_work.clone() else {
            return;
        };
        self.as_mut()
            .submit_library(LibraryAction::Favorite { work, favorite });
    }

    pub(super) fn select_initial_variant(mut self: Pin<&mut Self>) {
        if self.library_state == ffi::LibraryState::Ready
            && !self.inspecting
            && self.selected_variant_id.is_empty()
        {
            let id = self.variants.id_at(0);
            if !id.is_empty() {
                self.as_mut().select_variant(id);
            }
        }
    }

    pub(super) fn save_selected_preferences(mut self: Pin<&mut Self>) {
        if let Some(work) = self.selected_work.clone() {
            let preferences = self
                .work_preferences
                .get(&work.reference)
                .cloned()
                .unwrap_or_default();
            self.as_mut()
                .submit_library(LibraryAction::Preferences { work, preferences });
        }
    }

    pub(super) fn cancel_detail(mut self: Pin<&mut Self>) -> (u64, u64) {
        let (generations, inspecting_changed, resolving_changed) = {
            let mut state = self.as_mut().rust_mut();
            state.inspection.cancel();
            state.resolution.cancel();
            let changed = (
                (state.inspection.generation, state.resolution.generation),
                state.inspecting,
                state.resolving,
            );
            state.inspecting = false;
            state.resolving = false;
            changed
        };
        self.as_mut().close_reader();
        if inspecting_changed {
            self.as_mut().inspecting_changed();
            self.as_mut().selected_variant_state_changed();
        }
        if resolving_changed {
            self.as_mut().resolving_changed();
        }
        generations
    }

    fn set_inspecting(mut self: Pin<&mut Self>, value: bool) {
        if self.inspecting != value {
            self.as_mut().rust_mut().inspecting = value;
            self.as_mut().inspecting_changed();
            self.as_mut().selected_variant_state_changed();
        }
    }

    pub(super) fn show_work(mut self: Pin<&mut Self>, work: &Work, generation: u64) -> bool {
        if !self.as_mut().set_selection(Some(work), generation) {
            return false;
        }
        self.as_mut().rust_mut().variants.pin_mut().update(work);
        if self.inspection.generation != generation {
            return false;
        }
        self.as_mut().selected_variant_state_changed();
        self.inspection.generation == generation
    }

    pub(super) fn set_selection(
        mut self: Pin<&mut Self>,
        work: Option<&Work>,
        generation: u64,
    ) -> bool {
        if self.inspection.generation != generation {
            return false;
        }
        let (id, title, description) = match work {
            Some(work) => (
                QString::from(work_id(&work.reference)),
                QString::from(&work.title),
                QString::from(work.description.as_deref().unwrap_or_default()),
            ),
            None => (QString::default(), QString::default(), QString::default()),
        };
        let variant_id = work
            .and_then(|work| self.work_preferences.get(&work.reference))
            .and_then(|preferences| preferences.selected.as_ref())
            .map(variant_id)
            .map(QString::from)
            .unwrap_or_default();
        let (id_changed, category_changed, title_changed, description_changed, variant_changed) = {
            let mut state = self.as_mut().rust_mut();
            let changed = (
                state.selected_work_id != id,
                state.selected_work.as_ref().map(|work| work.category)
                    != work.map(|work| work.category),
                state.selected_work_title != title,
                state.selected_work_description != description,
                state.selected_variant_id != variant_id,
            );
            state.selected_work_id = id;
            state.selected_work_title = title;
            state.selected_work_description = description;
            state.selected_work = work.cloned();
            state.selected_variant_id = variant_id;
            changed
        };
        if id_changed {
            self.as_mut().selected_work_id_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        if category_changed {
            self.as_mut().selected_work_category_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        if title_changed {
            self.as_mut().selected_work_title_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        if description_changed {
            self.as_mut().selected_work_description_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        if variant_changed {
            self.as_mut().selected_variant_id_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        self.as_mut().variant_filters_changed();
        if self.inspection.generation != generation {
            return false;
        }
        self.as_mut().refresh_selected_favorite();
        self.inspection.generation == generation
    }

    pub(super) fn refresh_selected_favorite(mut self: Pin<&mut Self>) {
        let favorite = self.selected_work.as_ref().is_some_and(|work| {
            self.favorites
                .find(&models::work_id(&work.reference))
                .is_some()
        });
        if self.selected_favorite != favorite {
            self.as_mut().rust_mut().selected_favorite = favorite;
            self.as_mut().selected_favorite_changed();
        }
    }
}
