use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};
use qbutt_content::{ChapterRef, LibraryResult, ReaderPosition, ReaderProgress};

use super::super::ffi;
use super::super::library::{LibraryAction, ReaderSave};

impl ffi::ContentController {
    pub(in crate::controller) fn reader_progress_error_text(&self) -> &QString {
        &self.reader.progress_error
    }

    pub(in crate::controller) fn reader_progress_pending(&self) -> bool {
        self.reader.progress_pending
    }

    pub(in crate::controller) fn reader_retry_generation(&self) -> Option<u64> {
        (self.reader.reference.is_some() && self.reader.chapter_id.is_empty())
            .then_some(self.reader.loading.generation)
    }

    pub(in crate::controller) fn resume_reader_after_library_retry(
        self: Pin<&mut Self>,
        generation: u64,
    ) {
        if self.reader_retry_generation() == Some(generation) {
            self.load_reader_resume();
        }
    }

    pub(in crate::controller) fn prepare_reader_library_retry(
        mut self: Pin<&mut Self>,
        generation: u64,
    ) {
        if self.reader_retry_generation() != Some(generation) {
            return;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.busy = true;
            state.reader.error = QString::default();
        }
        self.reader_changed();
    }

    pub(in crate::controller) fn reader_library_failed(mut self: Pin<&mut Self>) -> bool {
        if self.reader.reference.is_none() || !self.reader.chapter_id.is_empty() {
            return false;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.loading.cancel();
            state.reader.busy = false;
            state.reader.error = ffi::content_error(ffi::ContentError::LibraryOpen);
        }
        true
    }

    pub(super) fn load_reader_resume(mut self: Pin<&mut Self>) {
        let Some(reference) = self.reader.reference.clone() else {
            return;
        };
        self.as_mut().rust_mut().reader.loading.cancel();
        let generation = self.reader.loading.generation;
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.busy = true;
            state.reader.error = QString::default();
        }
        self.as_mut().reader_changed();
        if self.reader.loading.generation != generation || self.reader.reference.is_none() {
            return;
        }
        let accepted = self.as_mut().submit_library(LibraryAction::ReaderResume {
            reference: reference.publication,
            generation,
        });
        if !accepted
            && self.reader.loading.generation == generation
            && self.reader.reference.is_some()
        {
            self.as_mut().rust_mut().reader.busy = false;
            self.as_mut().rust_mut().reader.error =
                ffi::content_error(ffi::ContentError::LibraryOpen);
            self.as_mut().reader_changed();
        }
    }

    pub(in crate::controller) fn complete_reader_resume(
        mut self: Pin<&mut Self>,
        result: LibraryResult<Option<ReaderProgress>>,
        generation: u64,
    ) {
        if self.reader.loading.generation != generation || self.reader.reference.is_none() {
            return;
        }
        match result {
            Ok(progress) => {
                let resume = progress.filter(|progress| {
                    Some(&progress.chapter.variant) == self.reader.reference.as_ref()
                });
                self.as_mut().rust_mut().reader.resume = resume;
                self.load_chapters();
            }
            Err(error) => {
                ffi::log_content_error(
                    &QString::from("reader resume"),
                    &QString::from(error.to_string()),
                );
                self.as_mut().rust_mut().reader.busy = false;
                self.as_mut().rust_mut().reader.error =
                    ffi::content_error(ffi::ContentError::LibraryOpen);
                self.as_mut().reader_changed();
            }
        }
    }

    pub fn scroll_reader_page(mut self: Pin<&mut Self>, url: QUrl, offset: f64) {
        if !self.reader.pages.shown(self.reader.page_index as usize)
            || self.reader_mode() != ffi::ReaderMode::Paged
            || self.reader.reference.is_none()
            || self.reader_page_url() != url
            || !offset.is_finite()
            || !(0.0..=1.0).contains(&offset)
        {
            return;
        }
        self.as_mut().rust_mut().reader.scroll_offset = offset;
        self.save_reader_position();
    }

    pub fn retry_reader_progress(self: Pin<&mut Self>) {
        if !self.reader.progress_pending && !self.reader.progress_error.is_empty() {
            self.submit_reader_progress();
        }
    }

    pub(in crate::controller) fn complete_reader_progress(
        mut self: Pin<&mut Self>,
        generation: u64,
        saved: bool,
    ) {
        if self.reader.progress_generation != generation || self.reader.reference.is_none() {
            return;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.progress_pending = false;
            state.reader.progress_error = if saved {
                QString::default()
            } else {
                ffi::reader_error(ffi::ReaderError::ProgressWrite)
            };
            if saved {
                state.reader.progress_save = None;
            }
        }
        self.reader_changed();
    }

    pub(super) fn save_reader_position(mut self: Pin<&mut Self>) {
        let position = self.reader_current_position();
        if self.reader.shown_position.as_ref() == Some(&position) {
            return;
        }
        let Some(variant) = self.reader.reference.clone() else {
            return;
        };
        let Some(work) = self.selected_work.clone() else {
            return;
        };
        if variant.publication != work.reference {
            return;
        }
        let preferences = self
            .work_preferences
            .get(&work.reference)
            .cloned()
            .unwrap_or_default();
        let progress = ReaderProgress {
            chapter: ChapterRef {
                variant,
                id: self.reader.chapter_id.to_string(),
            },
            position: position.clone(),
        };
        self.as_mut().rust_mut().reader.progress_save = Some(ReaderSave {
            work,
            preferences,
            progress,
        });
        self.as_mut().rust_mut().reader.shown_position = Some(position);
        self.submit_reader_progress();
    }

    pub(super) fn reader_current_position(&self) -> ReaderPosition {
        if self.reader.is_text {
            ReaderPosition::Text {
                block_id: self.reader.text.anchor.to_string(),
                offset: self.reader.text.offset,
            }
        } else {
            ReaderPosition::Page {
                page: self.reader.page_index as u32,
                scroll_offset: self.reader.scroll_offset,
            }
        }
    }

    fn submit_reader_progress(mut self: Pin<&mut Self>) {
        let Some(save) = self.reader.progress_save.clone() else {
            return;
        };
        let generation = self.reader.progress_generation.wrapping_add(1);
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.progress_generation = generation;
            state.reader.progress_pending = true;
        }
        let accepted = self
            .as_mut()
            .submit_library(LibraryAction::ReaderProgress { save, generation });
        if self.reader.progress_generation != generation || self.reader.reference.is_none() {
            return;
        }
        if !accepted {
            self.as_mut().rust_mut().reader.progress_pending = false;
            self.as_mut().rust_mut().reader.progress_error =
                ffi::reader_error(ffi::ReaderError::ProgressWrite);
        }
        self.reader_changed();
    }
}
