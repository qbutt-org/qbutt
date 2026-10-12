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
        save: ReaderSave,
        generation: u64,
    },
    ReaderResume {
        reference: PublicationRef,
        generation: u64,
    },
    History,
    RemoveHistory {
        reference: PublicationRef,
    },
}

/// The exact confirmed reading intent retained until its write is acknowledged.
/// Navigation cannot replace it; a later confirmed render can.
#[derive(Clone)]
pub struct ReaderSave {
    pub work: Work,
    pub preferences: VariantPreferences,
    pub progress: ReaderProgress,
}

pub enum LibraryUpdate {
    Loaded {
        result: LibraryResult<(Vec<LibraryEntry>, Vec<Work>)>,
        error_generation: u64,
        reader_generation: Option<u64>,
    },
    Changed {
        result: LibraryResult<Option<Box<LibraryEntry>>>,
        history: Option<LibraryResult<Vec<Work>>>,
        error_generation: u64,
        reader_generation: Option<u64>,
    },
    ReaderResume {
        result: LibraryResult<Option<ReaderProgress>>,
        generation: u64,
    },
    History {
        result: LibraryResult<Vec<Work>>,
        error_generation: u64,
    },
}

struct LibraryRequest {
    action: LibraryAction,
    error_generation: u64,
}

enum LibraryCommand {
    Request(Box<LibraryRequest>),
    Retry {
        error_generation: u64,
        reader_generation: Option<u64>,
    },
}

pub struct LibraryWorker {
    sender: Sender<LibraryCommand>,
}

impl LibraryWorker {
    pub fn start(
        runtime: &Runtime,
        profile: PathBuf,
        qt_thread: CxxQtThread<ContentController>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::channel::<LibraryCommand>(64);
        runtime.spawn_blocking(move || {
            // Loading permits commands only after this blocking job has entered.
            // A queued job can still be cancelled by runtime shutdown.
            let _ = qt_thread.queue(move |mut controller| {
                controller.as_mut().rust_mut().library_state = ffi::LibraryState::Loading;
                controller.as_mut().library_state_changed();
            });
            let mut error_generation = 0;
            let mut reader_generation = None;
            loop {
                let (mut library, result) = match Library::open(&profile).and_then(|library| {
                    let entries = library.entries(LibraryView::All)?;
                    let history = read_history(&library)?;
                    Ok((library, (entries, history)))
                }) {
                    Ok((library, entries)) => (Some(library), Ok(entries)),
                    Err(error) => (None, Err(error)),
                };
                let _ = qt_thread.queue(move |controller| {
                    controller.complete_library(LibraryUpdate::Loaded {
                        result,
                        error_generation,
                        reader_generation,
                    });
                });

                // This job remains the sole connection owner after a failed
                // open. An explicit retry reopens the same profile in FIFO order.
                loop {
                    let Some(command) = receiver.blocking_recv() else {
                        return;
                    };
                    match command {
                        LibraryCommand::Retry {
                            error_generation: retry_error_generation,
                            reader_generation: retry_reader_generation,
                        } => {
                            error_generation = retry_error_generation;
                            reader_generation = retry_reader_generation;
                            break;
                        }
                        LibraryCommand::Request(request) => {
                            if let Some(library) = &mut library {
                                let update = apply(library, *request);
                                let _ = qt_thread.queue(move |controller| {
                                    controller.complete_library(update);
                                });
                            }
                            // Initial failure terminates all accepted commands
                            // through Loaded; discard them before the next retry.
                        }
                    }
                }
                // Drop the old connection before another open attempt.
            }
        });
        Self { sender }
    }

    pub fn submit(&self, action: LibraryAction, error_generation: u64) -> Result<(), &'static str> {
        self.sender
            .try_send(LibraryCommand::Request(Box::new(LibraryRequest {
                action,
                error_generation,
            })))
            .map_err(|_| "The content library command queue is full or closed")
    }

    fn retry(
        &self,
        error_generation: u64,
        reader_generation: Option<u64>,
    ) -> Result<(), &'static str> {
        self.sender
            .try_send(LibraryCommand::Retry {
                error_generation,
                reader_generation,
            })
            .map_err(|_| "The content library retry queue is full or closed")
    }
}

fn apply(library: &mut Library, request: LibraryRequest) -> LibraryUpdate {
    let (result, reader_generation) = match request.action {
        LibraryAction::Favorite { work, favorite } => {
            (library.set_favorite(&work, favorite).map(Some), None)
        }
        LibraryAction::Preferences { work, preferences } => {
            (library.save_preferences(&work, preferences).map(Some), None)
        }
        LibraryAction::ReaderProgress { save, generation } => (
            library
                .save_reader_progress(&save.work, save.preferences, save.progress)
                .map(Some),
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
        LibraryAction::History => {
            return LibraryUpdate::History {
                result: read_history(library),
                error_generation: request.error_generation,
            };
        }
        LibraryAction::RemoveHistory { reference } => (library.remove_history(&reference), None),
    };
    let history = result.as_ref().ok().map(|_| read_history(library));
    LibraryUpdate::Changed {
        result: result.map(|entry| entry.map(Box::new)),
        history,
        error_generation: request.error_generation,
        reader_generation,
    }
}

fn read_history(library: &Library) -> LibraryResult<Vec<Work>> {
    library
        .entries(LibraryView::History)
        .map(|entries| entries.into_iter().map(|entry| entry.work).collect())
}

impl ffi::ContentController {
    pub(super) fn favorites_model(&self) -> *mut models::ffi::WorkModel {
        self.favorites.as_ptr().cast_mut()
    }

    pub(super) fn history_model(&self) -> *mut models::ffi::WorkModel {
        self.history.as_ptr().cast_mut()
    }

    pub fn refresh_history(self: Pin<&mut Self>) -> bool {
        self.submit_library(LibraryAction::History)
    }

    pub fn remove_history(self: Pin<&mut Self>, id: QString) -> bool {
        let Some(work) = self.history.find(&id.to_string()) else {
            return false;
        };
        let reference = work.reference.clone();
        self.submit_library(LibraryAction::RemoveHistory { reference })
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

    pub fn retry_library(mut self: Pin<&mut Self>) -> bool {
        if self.library_state != ffi::LibraryState::Failed {
            return false;
        }
        let Some(worker) = &self.library_worker else {
            return false;
        };
        let reader_generation = self.reader_retry_generation();
        match worker.retry(self.library_error_generation, reader_generation) {
            Ok(()) => {
                // The blocking owner has already entered and remains alive;
                // FIFO requests can be accepted behind this reopen command.
                self.as_mut().rust_mut().library_state = ffi::LibraryState::Loading;
                if let Some(generation) = reader_generation {
                    self.as_mut().prepare_reader_library_retry(generation);
                }
                self.as_mut().library_state_changed();
                true
            }
            Err(error) => {
                ffi::log_content_error(&QString::from("library retry"), &QString::from(error));
                self.as_mut().set_library_error(ffi::content_error(4));
                false
            }
        }
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
        if !matches!(update, LibraryUpdate::Loaded { .. }) {
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
        let history_read = matches!(&update, LibraryUpdate::History { .. })
            || matches!(
                &update,
                LibraryUpdate::Changed {
                    result: Ok(_),
                    history: Some(Err(_)),
                    ..
                }
            );
        let (initial, error_generation, reader_generation, result) = match update {
            LibraryUpdate::ReaderResume { result, generation } => {
                self.complete_reader_resume(result, generation);
                return;
            }
            LibraryUpdate::History {
                result,
                error_generation,
            } => {
                let result = result.map(|works| {
                    self.as_mut().rust_mut().history.pin_mut().replace(works);
                });
                (false, error_generation, None, result)
            }
            LibraryUpdate::Loaded {
                result,
                error_generation,
                reader_generation,
            } => {
                let result = result.map(|(entries, history)| {
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
                    self.as_mut().rust_mut().history.pin_mut().replace(history);
                });
                (true, error_generation, reader_generation, result)
            }
            LibraryUpdate::Changed {
                result,
                history,
                error_generation,
                reader_generation,
            } => {
                if let Some(generation) = reader_generation {
                    self.as_mut()
                        .complete_reader_progress(generation, result.is_ok());
                }
                let result = result.and_then(|entry| {
                    if let Some(entry) = entry {
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
                    }
                    history.transpose().map(|history| {
                        if let Some(works) = history {
                            self.as_mut().rust_mut().history.pin_mut().replace(works);
                        }
                    })
                });
                (false, error_generation, None, result)
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
                    if let Some(generation) = reader_generation {
                        self.as_mut().resume_reader_after_library_retry(generation);
                    }
                }
            }
            Err(error) => {
                let reader_failed = if initial {
                    let mut state = self.as_mut().rust_mut();
                    state.library_state = ffi::LibraryState::Failed;
                    state.library_pending = 0;
                    self.as_mut().reader_library_failed()
                } else {
                    false
                };
                ffi::log_content_error(
                    &QString::from("library"),
                    &QString::from(error.to_string()),
                );
                self.as_mut()
                    .set_library_error(ffi::content_error(if initial {
                        4
                    } else if history_read {
                        6
                    } else {
                        5
                    }));
                if reader_failed {
                    self.as_mut().reader_changed();
                }
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
