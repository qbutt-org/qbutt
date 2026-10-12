use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{
    PublicationRef, Source, SourceOperation, VariantRef, Work, WorkAssociation, WorkGroup,
};
use tokio::task::JoinSet;

use super::catalog::SourceResponse;
use super::ffi;
use super::library::LibraryAction;
use crate::models::{self, variant_id, work_id};

impl ffi::ContentController {
    pub(super) fn selected_work_publications(&self) -> Vec<PublicationRef> {
        self.selected_group
            .as_ref()
            .map_or_else(Vec::new, |group| group.references.clone())
    }

    pub(super) fn selected_share_preference(&self) -> Option<VariantRef> {
        self.selected_group
            .as_ref()
            .and_then(|group| self.work_preferences.get(&group.anchor))
            .and_then(|preferences| preferences.selected.clone())
    }

    pub(super) fn show_shared_publications(
        mut self: Pin<&mut Self>,
        references: Vec<PublicationRef>,
        publications: Vec<Work>,
        preferred: Option<VariantRef>,
    ) -> bool {
        if publications.is_empty()
            || references.len() != publications.len()
            || references.iter().collect::<HashSet<_>>().len() != references.len()
            || publications
                .iter()
                .map(|work| &work.reference)
                .collect::<HashSet<_>>()
                .len()
                != publications.len()
            || references
                .iter()
                .any(|reference| reference.validate().is_err())
            || references.iter().any(|reference| {
                !reference
                    .source
                    .categories()
                    .contains(&publications[0].category)
            })
            || publications
                .iter()
                .any(|work| !references.contains(&work.reference))
            || preferred.as_ref().is_some_and(|variant| {
                variant.validate().is_err() || !references.contains(&variant.publication)
            })
        {
            return false;
        }
        let mut association = WorkAssociation::default();
        for publication in &publications {
            association.update(publication.clone());
        }
        if association.groups().len() != 1 {
            return false;
        }
        let anchor = publications[0].reference.clone();
        let group = WorkGroup {
            anchor: anchor.clone(),
            references,
            publications,
        };
        let continuing = self.selected_group.as_ref().is_some_and(|current| {
            current.anchor == anchor && current.references == group.references
        });
        let generation = if continuing {
            self.inspection.generation
        } else {
            let (generation, _) = self.as_mut().cancel_detail();
            self.as_mut().rust_mut().variants.pin_mut().clear();
            self.as_mut().rust_mut().works.pin_mut().clear();
            generation
        };
        self.as_mut().rust_mut().search_association.clear();
        for work in &group.publications {
            self.as_mut()
                .rust_mut()
                .search_association
                .update(work.clone());
        }
        if let Some(work) = group.anchor_work() {
            self.as_mut()
                .rust_mut()
                .works
                .pin_mut()
                .update(work.clone());
        }
        self.as_mut()
            .rust_mut()
            .known_groups
            .insert(anchor.clone(), group.clone());
        if self.list_mode != ffi::ListMode::Search {
            self.as_mut().rust_mut().list_mode = ffi::ListMode::Search;
            self.as_mut().list_mode_changed();
        }
        if !continuing {
            self.as_mut()
                .rust_mut()
                .work_preferences
                .entry(anchor)
                .or_default()
                .selected = preferred;
        }
        self.as_mut().show_group(&group, generation)
    }

    pub(super) fn selected_work_category(&self) -> QString {
        let category = self
            .selected_group
            .as_ref()
            .and_then(WorkGroup::anchor_work)
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
            Some(qbutt_content::Transport::Text) => "text",
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
        let Some(work) = model.find(&id.to_string()).cloned() else {
            return;
        };
        if work.reference.source == Source::LocalBook {
            if self.selected_work_id == id && self.local_book.is_some() {
                return;
            }
            let (generation, _) = self.as_mut().cancel_detail();
            self.as_mut().rust_mut().variants.pin_mut().clear();
            self.as_mut().set_selection(None, generation);
            self.as_mut().load_local_book_from_library(work.reference);
            return;
        }
        let mut group = if self.list_mode == ffi::ListMode::Search {
            self.search_association
                .groups()
                .into_iter()
                .find(|group| group.references.contains(&work.reference))
                .map(|mut group| {
                    group.anchor = work.reference.clone();
                    group
                })
                .unwrap_or_else(|| WorkGroup::single(work.clone()))
        } else {
            self.known_groups
                .get(&work.reference)
                .cloned()
                .unwrap_or_else(|| WorkGroup::single(work))
        };
        if let Some(saved) = self.known_groups.get(&group.anchor) {
            let mut restored = group.clone();
            for known in &saved.publications {
                if restored.publication(&known.reference).is_none() {
                    restored.publications.push(known.clone());
                }
            }
            if restored.is_confirmed() {
                for reference in &saved.references {
                    if !restored.references.contains(reference) {
                        restored.references.push(reference.clone());
                    }
                }
                group = restored;
            }
        }
        if self.list_mode != ffi::ListMode::Search {
            for publication in &mut group.publications {
                publication.needs_inspection = true;
            }
        }
        if self.selected_work_id == id
            && (self.inspecting || group.publications.iter().all(|work| !work.needs_inspection))
        {
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
        let requests: Vec<_> = self.services.as_ref().map_or_else(Vec::new, |services| {
            group
                .publications
                .iter()
                .filter(|work| work.needs_inspection)
                .filter_map(|work| {
                    self.source_request(SourceOperation::Inspect {
                        reference: work.reference.clone(),
                        language: (work.reference.source == Source::MangaDex)
                            .then(|| services.language.clone()),
                    })
                })
                .collect()
        });
        let should_inspect = !requests.is_empty();
        // Cached variants cannot become actionable during synchronous selection notifications.
        self.as_mut().rust_mut().inspecting = should_inspect;
        if !self.as_mut().show_group(&group, generation) {
            return;
        }
        self.as_mut().set_error(None);
        if self.inspection.generation != generation {
            return;
        }
        if should_inspect {
            self.as_mut().inspecting_changed();
            self.as_mut().selected_variant_state_changed();
        }
        if self.inspection.generation != generation {
            return;
        }
        if !should_inspect {
            self.as_mut().select_initial_variant();
            return;
        }
        if self.services.is_none() {
            self.as_mut()
                .set_error(Some(ffi::ContentError::ServicesUnavailable));
            return;
        }
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let catalog = Arc::clone(&services.catalog);
        let task = services.runtime.spawn(async move {
            let mut requests = requests.into_iter();
            let mut pending = JoinSet::new();
            loop {
                while pending.len() < 4
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
                controller.as_mut().set_inspecting(false);
                if controller.inspection.generation == generation && controller.error_kind.is_none()
                {
                    if controller.selected_variant_id.is_empty() {
                        controller.as_mut().select_initial_variant();
                    } else {
                        controller.as_mut().save_selected_preferences();
                    }
                }
            });
        });
        self.as_mut().rust_mut().inspection.task = Some(task);
    }

    pub fn select_variant(mut self: Pin<&mut Self>, id: QString) {
        if self.selected_variant_id == id {
            return;
        }
        let Some(variant) = self.variants.find(&id.to_string()) else {
            return;
        };
        let Some(group) = self.selected_group.clone() else {
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
                .entry(group.anchor.clone())
                .or_default();
            preferences.selected = Some(reference);
            preferences.clone()
        };
        let resolution_generation = self.resolution.generation;
        // Accept the captured per-work intent before any notification can select
        // another work. An acknowledgement never owns the current UI choice.
        self.as_mut()
            .submit_library(LibraryAction::Preferences { group, preferences });
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
        let Some(group) = self.selected_group.clone() else {
            return;
        };
        self.as_mut()
            .submit_library(LibraryAction::Favorite { group, favorite });
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
        if let Some(group) = self.selected_group.clone() {
            let preferences = self
                .work_preferences
                .get(&group.anchor)
                .cloned()
                .unwrap_or_default();
            self.as_mut()
                .submit_library(LibraryAction::Preferences { group, preferences });
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
        self.as_mut().rust_mut().local_book = None;
        self.as_mut().rust_mut().local_book_load.cancel();
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

    pub(super) fn show_group(mut self: Pin<&mut Self>, group: &WorkGroup, generation: u64) -> bool {
        if !self.as_mut().set_selection(Some(group), generation) {
            return false;
        }
        let language = self
            .services
            .as_ref()
            .map(|services| services.language.clone());
        self.as_mut()
            .rust_mut()
            .variants
            .pin_mut()
            .update_group(group, language.as_deref());
        if self.inspection.generation != generation {
            return false;
        }
        self.as_mut().selected_variant_state_changed();
        self.inspection.generation == generation
    }

    pub(super) fn publish_inspected_work(mut self: Pin<&mut Self>, work: Work, generation: u64) {
        if self.inspection.generation != generation {
            return;
        }
        self.as_mut()
            .rust_mut()
            .search_association
            .update(work.clone());
        if self.list_mode == ffi::ListMode::Search {
            let search_generation = self.search.generation;
            self.as_mut().refresh_search_groups(search_generation);
        } else if let Some(mut group) = self.selected_group.clone() {
            if let Some(previous) = group
                .publications
                .iter_mut()
                .find(|previous| previous.reference == work.reference)
            {
                *previous = work;
            }
            if !group.is_confirmed() {
                group = confirmed_anchor_group(&group);
            }
            self.as_mut()
                .rust_mut()
                .known_groups
                .insert(group.anchor.clone(), group.clone());
            self.as_mut().show_group(&group, generation);
        }
    }

    pub(super) fn set_selection(
        mut self: Pin<&mut Self>,
        group: Option<&WorkGroup>,
        generation: u64,
    ) -> bool {
        if self.inspection.generation != generation {
            return false;
        }
        let work = group.and_then(WorkGroup::anchor_work);
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
                state
                    .selected_group
                    .as_ref()
                    .and_then(WorkGroup::anchor_work)
                    .map(|work| work.category)
                    != work.map(|work| work.category),
                state.selected_work_title != title,
                state.selected_work_description != description,
                state.selected_variant_id != variant_id,
            );
            state.selected_work_id = id;
            state.selected_work_title = title;
            state.selected_work_description = description;
            state.selected_group = group.cloned();
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
        let favorite = self.selected_group.as_ref().is_some_and(|group| {
            self.favorites
                .find(&models::work_id(&group.anchor))
                .is_some()
        });
        if self.selected_favorite != favorite {
            self.as_mut().rust_mut().selected_favorite = favorite;
            self.as_mut().selected_favorite_changed();
        }
    }
}

fn confirmed_anchor_group(group: &WorkGroup) -> WorkGroup {
    let mut association = WorkAssociation::default();
    if let Some(anchor) = group.anchor_work() {
        association.update(anchor.clone());
    }
    for publication in &group.publications {
        if publication.reference != group.anchor {
            association.update(publication.clone());
        }
    }
    let mut confirmed = association
        .groups()
        .into_iter()
        .find(|candidate| candidate.anchor == group.anchor)
        .expect("the selected group has a resolved anchor");
    for reference in &group.references {
        if group.publication(reference).is_none() && !confirmed.references.contains(reference) {
            confirmed.references.push(reference.clone());
        }
    }
    confirmed
}
