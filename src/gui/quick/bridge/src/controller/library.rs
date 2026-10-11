use std::path::PathBuf;
use std::pin::Pin;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{Library, LibraryEntry, LibraryResult, LibraryView, VariantPreferences, Work};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, Sender};

use super::ffi;
use super::ffi::ContentController;
use crate::models;

pub enum LibraryAction {
    Favorite {
        work: Work,
        favorite: bool,
    },
    Preferences {
        work: Work,
        preferences: VariantPreferences,
    },
}

pub enum LibraryUpdate {
    Loaded(Vec<LibraryEntry>),
    Changed(Box<LibraryEntry>),
}

pub struct LibraryWorker {
    sender: Sender<LibraryAction>,
}

impl LibraryWorker {
    pub fn start(
        runtime: &Runtime,
        profile: PathBuf,
        qt_thread: CxxQtThread<ContentController>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::channel::<LibraryAction>(64);
        runtime.spawn_blocking(move || {
            let mut library = match Library::open(profile) {
                Ok(library) => library,
                Err(error) => {
                    let _ = qt_thread.queue(move |controller| {
                        controller.complete_library(Err(error));
                    });
                    return;
                }
            };
            let entries = library.entries(LibraryView::All).map(LibraryUpdate::Loaded);
            let _ = qt_thread.queue(move |controller| {
                controller.complete_library(entries);
            });

            // This is the sole connection owner. Accepted commands stay in FIFO
            // order and finish even when selection changes or the QObject closes.
            while let Some(request) = receiver.blocking_recv() {
                let result = apply(&mut library, request)
                    .map(|entry| LibraryUpdate::Changed(Box::new(entry)));
                let _ = qt_thread.queue(move |controller| {
                    controller.complete_library(result);
                });
            }
            // Connection close also runs outside the GUI thread.
        });
        Self { sender }
    }

    pub fn submit(&self, action: LibraryAction) -> Result<(), &'static str> {
        self.sender
            .try_send(action)
            .map_err(|_| "The content library command queue is full or closed")
    }
}

fn apply(library: &mut Library, action: LibraryAction) -> LibraryResult<LibraryEntry> {
    match action {
        LibraryAction::Favorite { work, favorite } => library.set_favorite(&work, favorite),
        LibraryAction::Preferences { work, preferences } => {
            library.save_preferences(&work, preferences)
        }
    }
}

impl ffi::ContentController {
    pub(super) fn favorites_model(&self) -> *mut models::ffi::WorkModel {
        self.favorites.as_ptr().cast_mut()
    }

    pub(super) fn is_library_pending(&self) -> bool {
        self.library_pending != 0
    }

    pub fn init_library(mut self: Pin<&mut Self>, profile_dir: QString) {
        if self.library_state != ffi::LibraryState::Uninitialized {
            return;
        }
        let profile = PathBuf::from(profile_dir.to_string());
        if !profile.is_absolute() || self.services.is_none() {
            self.as_mut().rust_mut().library_state = ffi::LibraryState::Failed;
            self.as_mut().library_state_changed();
            self.as_mut().set_library_error(ffi::content_error(4));
            return;
        }
        self.as_mut().rust_mut().library_state = ffi::LibraryState::Loading;
        self.as_mut().library_state_changed();
        let worker = LibraryWorker::start(
            &self.services.as_ref().unwrap().runtime,
            profile,
            self.qt_thread(),
        );
        self.as_mut().rust_mut().library_worker = Some(worker);
    }

    pub(super) fn set_library_error(mut self: Pin<&mut Self>, error: QString) {
        if self.library_error_text != error {
            self.as_mut().rust_mut().library_error_text = error;
            self.as_mut().library_error_text_changed();
        }
    }

    pub(super) fn complete_library(mut self: Pin<&mut Self>, result: LibraryResult<LibraryUpdate>) {
        let initial = self.library_state == ffi::LibraryState::Loading;
        self.as_mut().rust_mut().library_revision = self.library_revision.wrapping_add(1);
        let revision = self.library_revision;
        let was_pending = self.is_library_pending();
        if !initial {
            self.as_mut().rust_mut().library_pending -= 1;
        }
        match result {
            Ok(update) => {
                match update {
                    LibraryUpdate::Loaded(entries) => {
                        let favorites: Vec<_> = entries
                            .iter()
                            .filter(|entry| entry.favorite)
                            .map(|entry| entry.work.clone())
                            .collect();
                        self.as_mut().rust_mut().library_entries = entries;
                        for work in favorites {
                            self.as_mut().rust_mut().favorites.pin_mut().update(work);
                            if self.library_revision != revision {
                                return;
                            }
                        }
                    }
                    LibraryUpdate::Changed(entry) => {
                        let entry = *entry;
                        let reference = entry.work.reference.clone();
                        let work = entry.favorite.then(|| entry.work.clone());
                        {
                            let mut state = self.as_mut().rust_mut();
                            if let Some(saved) = state
                                .library_entries
                                .iter_mut()
                                .find(|saved| saved.work.reference == reference)
                            {
                                *saved = entry;
                            } else {
                                state.library_entries.push(entry);
                            }
                        }
                        if let Some(work) = work {
                            self.as_mut().rust_mut().favorites.pin_mut().update(work);
                        } else {
                            self.as_mut()
                                .rust_mut()
                                .favorites
                                .pin_mut()
                                .remove(&reference);
                        }
                    }
                }
                if self.library_revision != revision {
                    return;
                }
                self.as_mut().refresh_selected_favorite();
                if self.library_revision != revision {
                    return;
                }
                self.as_mut().set_library_error(QString::default());
                if self.library_revision != revision {
                    return;
                }
                if initial {
                    self.as_mut().rust_mut().library_state = ffi::LibraryState::Ready;
                    self.as_mut().library_state_changed();
                }
            }
            Err(error) => {
                ffi::log_content_error(
                    &QString::from("library"),
                    &QString::from(error.to_string()),
                );
                self.as_mut()
                    .set_library_error(ffi::content_error(if initial { 4 } else { 5 }));
                if self.library_revision != revision {
                    return;
                }
                if initial {
                    self.as_mut().rust_mut().library_state = ffi::LibraryState::Failed;
                    self.as_mut().library_state_changed();
                }
            }
        }
        if self.library_revision == revision && was_pending != self.is_library_pending() {
            self.as_mut().library_pending_changed();
        }
    }

    pub(super) fn submit_library(mut self: Pin<&mut Self>, action: LibraryAction) {
        if self.library_state != ffi::LibraryState::Ready {
            return;
        }
        match self.library_worker.as_ref().unwrap().submit(action) {
            Ok(()) => {
                let was_pending = self.is_library_pending();
                self.as_mut().rust_mut().library_pending += 1;
                if !was_pending {
                    self.as_mut().library_pending_changed();
                }
            }
            Err(error) => {
                ffi::log_content_error(&QString::from("library"), &QString::from(error));
                self.as_mut().set_library_error(ffi::content_error(5));
            }
        }
    }

    pub(super) fn refresh_selected_favorite(mut self: Pin<&mut Self>) {
        let favorite = self.selected_work.as_ref().is_some_and(|work| {
            self.library_entries
                .iter()
                .any(|entry| entry.favorite && entry.work.reference == work.reference)
        });
        if self.selected_favorite != favorite {
            self.as_mut().rust_mut().selected_favorite = favorite;
            self.as_mut().selected_favorite_changed();
        }
    }

    pub fn set_list_mode(mut self: Pin<&mut Self>, mode: ffi::ListMode) {
        if self.list_mode != mode {
            self.as_mut().rust_mut().list_mode = mode;
            self.as_mut().list_mode_changed();
        }
    }

    pub fn set_selected_favorite(mut self: Pin<&mut Self>, favorite: bool) {
        let Some(work) = self.selected_work.clone() else {
            return;
        };
        self.as_mut()
            .submit_library(LibraryAction::Favorite { work, favorite });
    }

    pub(super) fn variant_selection_state(&self) -> ffi::VariantSelectionState {
        if self.inspecting {
            ffi::VariantSelectionState::Inspecting
        } else if self.selected_variant_id.is_empty() {
            ffi::VariantSelectionState::None
        } else if self
            .variants
            .find(&self.selected_variant_id.to_string())
            .is_some()
        {
            ffi::VariantSelectionState::Available
        } else {
            ffi::VariantSelectionState::Unavailable
        }
    }

    pub fn select_variant(mut self: Pin<&mut Self>, id: QString) {
        if self.selected_variant_id == id || self.inspecting {
            return;
        }
        let Some(variant) = self.variants.find(&id.to_string()) else {
            return;
        };
        let reference = variant.reference.clone();
        let generation = self.inspection.generation;
        self.as_mut().rust_mut().resolution.cancel();
        let resolution_generation = self.resolution.generation;
        self.as_mut().rust_mut().selected_preferences.selected = Some(reference);
        self.as_mut().rust_mut().selected_variant_id = id.clone();
        self.as_mut().set_resolving(false);
        if self.inspection.generation != generation
            || self.resolution.generation != resolution_generation
            || self.selected_variant_id != id
        {
            return;
        }
        self.as_mut().selected_variant_id_changed();
        if self.inspection.generation != generation
            || self.resolution.generation != resolution_generation
            || self.selected_variant_id != id
        {
            return;
        }
        self.as_mut().selected_variant_state_changed();
        if self.inspection.generation == generation
            && self.resolution.generation == resolution_generation
            && self.selected_variant_id == id
        {
            self.as_mut().save_selected_preferences();
        }
    }

    pub(super) fn save_selected_preferences(mut self: Pin<&mut Self>) {
        if let Some(work) = self.selected_work.clone() {
            let preferences = self.selected_preferences.clone();
            self.as_mut()
                .submit_library(LibraryAction::Preferences { work, preferences });
        }
    }
}
