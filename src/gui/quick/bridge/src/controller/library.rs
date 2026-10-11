use std::path::PathBuf;
use std::pin::Pin;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{
    Library, LibraryEntry, LibraryResult, LibraryView, PublicationRef, ReaderProgress,
    VariantPreferences, Work,
};
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
    ReaderProgress {
        work: Work,
        preferences: VariantPreferences,
        progress: ReaderProgress,
        generation: u64,
    },
    ReaderResume {
        reference: PublicationRef,
        generation: u64,
    },
}

pub enum LibraryUpdate {
    Loaded(LibraryResult<Vec<LibraryEntry>>),
    Changed {
        result: LibraryResult<Box<LibraryEntry>>,
        error_generation: u64,
        reader_generation: Option<u64>,
    },
    ReaderResume {
        result: LibraryResult<Option<ReaderProgress>>,
        generation: u64,
    },
}

struct LibraryRequest {
    action: LibraryAction,
    error_generation: u64,
}

pub struct LibraryWorker {
    sender: Sender<LibraryRequest>,
}

impl LibraryWorker {
    pub fn start(
        runtime: &Runtime,
        profile: PathBuf,
        qt_thread: CxxQtThread<ContentController>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::channel::<LibraryRequest>(64);
        runtime.spawn_blocking(move || {
            // Loading permits commands only after this blocking job has entered.
            // A queued job can still be cancelled by runtime shutdown.
            let _ = qt_thread.queue(move |mut controller| {
                controller.as_mut().rust_mut().library_state = ffi::LibraryState::Loading;
                controller.as_mut().library_state_changed();
            });
            let (mut library, entries) = match Library::open(profile).and_then(|library| {
                let entries = library.entries(LibraryView::All)?;
                Ok((library, entries))
            }) {
                Ok(loaded) => loaded,
                Err(error) => {
                    let _ = qt_thread.queue(move |controller| {
                        controller.complete_library(LibraryUpdate::Loaded(Err(error)));
                    });
                    return;
                }
            };
            let _ = qt_thread.queue(move |controller| {
                controller.complete_library(LibraryUpdate::Loaded(Ok(entries)));
            });

            // This is the sole connection owner. Accepted commands stay in FIFO
            // order and finish even when selection changes or the QObject closes.
            while let Some(request) = receiver.blocking_recv() {
                let update = apply(&mut library, request);
                let _ = qt_thread.queue(move |controller| {
                    controller.complete_library(update);
                });
            }
            // Connection close also runs outside the GUI thread.
        });
        Self { sender }
    }

    pub fn submit(&self, action: LibraryAction, error_generation: u64) -> Result<(), &'static str> {
        self.sender
            .try_send(LibraryRequest {
                action,
                error_generation,
            })
            .map_err(|_| "The content library command queue is full or closed")
    }
}

fn apply(library: &mut Library, request: LibraryRequest) -> LibraryUpdate {
    let (result, reader_generation) = match request.action {
        LibraryAction::Favorite { work, favorite } => (library.set_favorite(&work, favorite), None),
        LibraryAction::Preferences { work, preferences } => {
            (library.save_preferences(&work, preferences), None)
        }
        LibraryAction::ReaderProgress {
            work,
            preferences,
            progress,
            generation,
        } => (
            library.save_reader_progress(&work, preferences, progress),
            Some(generation),
        ),
        LibraryAction::ReaderResume {
            reference,
            generation,
        } => {
            return LibraryUpdate::ReaderResume {
                result: library
                    .get(&reference)
                    .map(|entry| entry.and_then(|entry| entry.reader_progress)),
                generation,
            };
        }
    };
    LibraryUpdate::Changed {
        result: result.map(Box::new),
        error_generation: request.error_generation,
        reader_generation,
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
        if self.library_state != ffi::LibraryState::Uninitialized || self.library_worker.is_some() {
            return;
        }
        let profile = PathBuf::from(profile_dir.to_string());
        if !profile.is_absolute() || self.services.is_none() {
            self.as_mut().rust_mut().library_state = ffi::LibraryState::Failed;
            self.as_mut().library_state_changed();
            self.as_mut().set_library_error(ffi::content_error(4));
            return;
        }
        let worker = LibraryWorker::start(
            &self.services.as_ref().unwrap().runtime,
            profile,
            self.qt_thread(),
        );
        self.as_mut().rust_mut().library_worker = Some(worker);
    }

    pub(super) fn set_library_error(mut self: Pin<&mut Self>, error: QString) {
        if !error.is_empty() {
            self.as_mut().rust_mut().library_error_generation =
                self.library_error_generation.wrapping_add(1);
        }
        if self.library_error_text != error {
            self.as_mut().rust_mut().library_error_text = error;
            self.as_mut().library_error_text_changed();
        }
    }

    pub(super) fn complete_library(mut self: Pin<&mut Self>, update: LibraryUpdate) {
        let was_pending = self.is_library_pending();
        if !matches!(update, LibraryUpdate::Loaded(_)) {
            self.as_mut().rust_mut().library_pending -= 1;
        }
        self.as_mut().rust_mut().library_updates.push_back(update);
        if self.publishing_library {
            // I/O has completed, but a Qt begin/end notification is still open.
            // Let its observer return before applying another model mutation.
            if was_pending != self.is_library_pending() {
                self.as_mut().library_pending_changed();
            }
            return;
        }
        self.as_mut().rust_mut().publishing_library = true;
        loop {
            let update = self.as_mut().rust_mut().library_updates.pop_front();
            let Some(update) = update else {
                break;
            };
            self.as_mut().publish_library(update);
        }
        self.as_mut().rust_mut().publishing_library = false;
        if was_pending != self.is_library_pending() {
            self.as_mut().library_pending_changed();
        }
    }

    fn publish_library(mut self: Pin<&mut Self>, update: LibraryUpdate) {
        let (initial, error_generation, result) = match update {
            LibraryUpdate::ReaderResume { result, generation } => {
                self.complete_reader_resume(result, generation);
                return;
            }
            LibraryUpdate::Loaded(result) => {
                let result = result.map(|entries| {
                    for entry in &entries {
                        self.as_mut()
                            .rust_mut()
                            .work_preferences
                            .entry(entry.work.reference.clone())
                            .or_insert_with(|| entry.preferences.clone());
                    }
                    for entry in entries.into_iter().filter(|entry| entry.favorite) {
                        self.as_mut()
                            .rust_mut()
                            .favorites
                            .pin_mut()
                            .update(entry.work);
                    }
                });
                (true, 0, result)
            }
            LibraryUpdate::Changed {
                result,
                error_generation,
                reader_generation,
            } => {
                if let Some(generation) = reader_generation {
                    self.as_mut()
                        .complete_reader_progress(generation, result.is_ok());
                }
                let result = result.map(|entry| {
                    if entry.favorite {
                        self.as_mut()
                            .rust_mut()
                            .favorites
                            .pin_mut()
                            .update(entry.work);
                    } else {
                        self.as_mut()
                            .rust_mut()
                            .favorites
                            .pin_mut()
                            .remove(&entry.work.reference);
                    }
                });
                (false, error_generation, result)
            }
        };
        match result {
            Ok(()) => {
                self.as_mut().refresh_selected_favorite();
                if self.library_error_generation == error_generation {
                    self.as_mut().set_library_error(QString::default());
                }
                if initial {
                    self.as_mut().rust_mut().library_state = ffi::LibraryState::Ready;
                    let generation = self.inspection.generation;
                    if let Some(work) = self.selected_work.clone()
                        && self.as_mut().set_selection(Some(&work), generation)
                    {
                        self.as_mut().selected_variant_state_changed();
                        if self.inspection.generation == generation && !self.inspecting {
                            if self.selected_variant_id.is_empty() {
                                self.as_mut().select_initial_variant();
                            } else {
                                self.as_mut().save_selected_preferences();
                            }
                        }
                    }
                    self.as_mut().library_state_changed();
                }
            }
            Err(error) => {
                if initial {
                    let mut state = self.as_mut().rust_mut();
                    state.library_state = ffi::LibraryState::Failed;
                    state.library_pending = 0;
                    self.as_mut().reader_library_failed();
                }
                ffi::log_content_error(
                    &QString::from("library"),
                    &QString::from(error.to_string()),
                );
                self.as_mut()
                    .set_library_error(ffi::content_error(if initial { 4 } else { 5 }));
                if initial {
                    self.as_mut().library_state_changed();
                }
            }
        }
    }

    pub(super) fn submit_library(mut self: Pin<&mut Self>, action: LibraryAction) -> bool {
        if !matches!(
            self.library_state,
            ffi::LibraryState::Loading | ffi::LibraryState::Ready
        ) {
            return false;
        }
        match self
            .library_worker
            .as_ref()
            .unwrap()
            .submit(action, self.library_error_generation)
        {
            Ok(()) => {
                let was_pending = self.is_library_pending();
                self.as_mut().rust_mut().library_pending += 1;
                if !was_pending {
                    self.as_mut().library_pending_changed();
                }
                true
            }
            Err(error) => {
                ffi::log_content_error(&QString::from("library"), &QString::from(error));
                self.as_mut().set_library_error(ffi::content_error(5));
                false
            }
        }
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
}
