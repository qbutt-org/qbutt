use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use qbutt_content::{LibraryResult, ReaderFit, ReaderMode, ReaderSettings};

use super::ffi;
use super::library::LibraryAction;

#[derive(Default)]
pub(super) struct ReaderSettingsState {
    value: ReaderSettings,
    generation: u64,
    save: SaveState,
}

#[derive(Default, PartialEq, Eq)]
enum SaveState {
    #[default]
    Idle,
    Pending,
    Failed,
}

impl ffi::ContentController {
    pub(super) fn reader_mode(&self) -> ffi::ReaderMode {
        match self.reader_settings.value.mode {
            ReaderMode::Paged => ffi::ReaderMode::Paged,
            ReaderMode::Continuous => ffi::ReaderMode::Continuous,
        }
    }

    pub(super) fn reader_fit(&self) -> ffi::ReaderFit {
        match self.reader_settings.value.fit {
            ReaderFit::Height => ffi::ReaderFit::Height,
            ReaderFit::Width => ffi::ReaderFit::Width,
        }
    }

    pub(super) fn reader_settings_state(&self) -> ffi::ReaderSettingsState {
        match self.library_state {
            ffi::LibraryState::Failed => ffi::ReaderSettingsState::Failed,
            ffi::LibraryState::Loading if self.reader_settings.save != SaveState::Pending => {
                ffi::ReaderSettingsState::Loading
            }
            _ => match self.reader_settings.save {
                SaveState::Pending => ffi::ReaderSettingsState::Saving,
                SaveState::Failed => ffi::ReaderSettingsState::Failed,
                SaveState::Idle if self.library_state == ffi::LibraryState::Ready => {
                    ffi::ReaderSettingsState::Ready
                }
                _ => ffi::ReaderSettingsState::Uninitialized,
            },
        }
    }

    pub(super) fn reader_settings_error_text(&self) -> QString {
        if self.reader_settings_state() != ffi::ReaderSettingsState::Failed {
            return QString::default();
        }
        ffi::content_error(if self.library_state == ffi::LibraryState::Failed {
            ffi::ContentError::LibraryOpen
        } else {
            ffi::ContentError::ReaderSettingsWrite
        })
    }

    pub fn set_reader_fit(mut self: Pin<&mut Self>, fit: ffi::ReaderFit) -> bool {
        let fit = match fit {
            ffi::ReaderFit::Height => ReaderFit::Height,
            ffi::ReaderFit::Width => ReaderFit::Width,
            _ => return false,
        };
        if self.library_state == ffi::LibraryState::Ready
            && self.reader_settings.value.fit == fit
            && self.reader_settings.save == SaveState::Idle
        {
            return true;
        }
        self.as_mut().rust_mut().reader_settings.value.fit = fit;
        self.submit_reader_settings()
    }

    pub fn retry_reader_settings(self: Pin<&mut Self>) -> bool {
        if self.reader_settings_state() != ffi::ReaderSettingsState::Failed {
            return false;
        }
        if self.library_state == ffi::LibraryState::Failed {
            return self.retry_library();
        }
        self.submit_reader_settings()
    }

    pub fn set_reader_mode(mut self: Pin<&mut Self>, mode: ffi::ReaderMode) -> bool {
        let mode = match mode {
            ffi::ReaderMode::Paged => ReaderMode::Paged,
            ffi::ReaderMode::Continuous => ReaderMode::Continuous,
            _ => return false,
        };
        if self.library_state == ffi::LibraryState::Ready
            && self.reader_settings.value.mode == mode
            && self.reader_settings.save == SaveState::Idle
        {
            return true;
        }
        self.as_mut().rust_mut().reader_settings.value.mode = mode;
        let accepted = self.as_mut().submit_reader_settings();
        if self.reader_settings.value.mode == mode {
            self.reader_mode_changed();
        }
        accepted
    }

    fn submit_reader_settings(mut self: Pin<&mut Self>) -> bool {
        let generation = self.reader_settings.generation.wrapping_add(1);
        let settings = self.reader_settings.value;
        {
            let mut state = self.as_mut().rust_mut();
            state.reader_settings.generation = generation;
            state.reader_settings.save = SaveState::Pending;
        }
        let accepted = self.as_mut().submit_library(LibraryAction::ReaderSettings {
            settings,
            generation,
        });
        if self.reader_settings.generation == generation {
            if !accepted {
                self.as_mut().rust_mut().reader_settings.save = SaveState::Failed;
            }
            self.as_mut().reader_changed();
            self.reader_settings_changed();
        }
        accepted
    }

    pub(super) fn complete_reader_settings(
        mut self: Pin<&mut Self>,
        result: LibraryResult<()>,
        generation: u64,
    ) {
        if let Err(error) = &result {
            ffi::log_content_error(
                &QString::from("reader settings"),
                &QString::from(error.to_string()),
            );
        }
        if self.reader_settings.generation != generation
            || self.reader_settings.save != SaveState::Pending
        {
            return;
        }
        self.as_mut().rust_mut().reader_settings.save = if result.is_ok() {
            SaveState::Idle
        } else {
            SaveState::Failed
        };
        self.reader_settings_changed();
    }

    pub(super) fn load_reader_settings(mut self: Pin<&mut Self>, settings: ReaderSettings) {
        if self.reader_settings.generation != 0 {
            return;
        }
        self.as_mut().rust_mut().reader_settings.value = settings;
        self.reader_changed();
    }

    pub(super) fn reader_settings_retry_generation(&self) -> Option<u64> {
        (self.reader_settings.save == SaveState::Failed).then_some(self.reader_settings.generation)
    }

    pub(super) fn resume_reader_settings_after_library_retry(
        self: Pin<&mut Self>,
        generation: u64,
    ) {
        if self.reader_settings_retry_generation() == Some(generation) {
            self.submit_reader_settings();
        }
    }

    pub(super) fn reader_settings_library_failed(mut self: Pin<&mut Self>) {
        if self.reader_settings.save == SaveState::Pending {
            self.as_mut().rust_mut().reader_settings.save = SaveState::Failed;
        }
        self.reader_settings_changed();
    }
}
