use std::pin::Pin;

use cxx::UniquePtr;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};
use qbutt_content::{ReaderPosition, ReaderProgress, Source, Transport, VariantRef};
use tokio::sync::watch;

use super::library::ReaderSave;
use super::sources::SourceRequest;
use super::{Pending, ffi};
use crate::page_model;
use crate::reader_models;

mod chapters;
mod manga;
mod progress;
mod text;
mod translation;
mod viewport;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ReaderViewport {
    pub(super) first: usize,
    pub(super) last: usize,
    pub(super) anchor: usize,
    pub(super) retry: Option<(u64, usize)>,
    pub(super) revision: u64,
}

impl ReaderViewport {
    fn page(index: usize) -> Self {
        Self {
            first: index,
            last: index,
            anchor: index,
            retry: None,
            revision: 0,
        }
    }

    pub(super) fn retained(self, count: usize) -> std::ops::RangeInclusive<usize> {
        self.first.saturating_sub(1)..=self.last.saturating_add(2).min(count.saturating_sub(1))
    }
}

pub(super) struct ReaderState {
    chapters: UniquePtr<reader_models::ffi::ChapterModel>,
    pages: UniquePtr<page_model::ffi::PageModel>,
    text: text::TextReaderState,
    translation: translation::TextTranslationState,
    manga_translation: manga::MangaTranslationState,
    is_text: bool,
    reference: Option<VariantRef>,
    busy: bool,
    error: QString,
    chapter_title: QString,
    chapter_id: QString,
    page_index: i32,
    scroll_offset: f64,
    shown_position: Option<ReaderPosition>,
    resume: Option<ReaderProgress>,
    progress_generation: u64,
    progress_pending: bool,
    progress_error: QString,
    progress_save: Option<ReaderSave>,
    query: QString,
    loading: Pending,
    chapter: Pending,
    position: Option<watch::Sender<ReaderViewport>>,
}

impl Default for ReaderState {
    fn default() -> Self {
        Self {
            chapters: reader_models::ffi::create_chapter_model(),
            pages: page_model::ffi::create_page_model(),
            text: text::TextReaderState::default(),
            translation: translation::TextTranslationState::default(),
            manga_translation: manga::MangaTranslationState::default(),
            is_text: false,
            reference: None,
            busy: false,
            error: QString::default(),
            chapter_title: QString::default(),
            chapter_id: QString::default(),
            page_index: 0,
            scroll_offset: 0.0,
            shown_position: None,
            resume: None,
            progress_generation: 0,
            progress_pending: false,
            progress_error: QString::default(),
            progress_save: None,
            query: QString::default(),
            loading: Pending::default(),
            chapter: Pending::default(),
            position: None,
        }
    }
}

impl ReaderState {
    pub(super) fn cancel(&mut self) {
        self.loading.cancel();
        self.chapter.cancel();
        self.position.take();
        self.text.prefetch.cancel();
        self.translation.reset_reader();
        self.manga_translation.reset_reader();
        self.progress_generation = self.progress_generation.wrapping_add(1);
        self.progress_pending = false;
        self.progress_error = QString::default();
        self.progress_save = None;
    }
}

impl ffi::ContentController {
    pub(super) fn close_disabled_reader(self: Pin<&mut Self>, source: Source) {
        if self
            .reader
            .reference
            .as_ref()
            .is_some_and(|reference| reference.publication.source == source)
        {
            self.close_reader();
        }
    }

    pub(super) fn reader_source_current(&self, generation: u64, request: &SourceRequest) -> bool {
        self.reader.chapter.generation == generation
            && self.reader.reference.is_some()
            && self.source_request_current(request)
    }

    pub(super) fn reader_pages(&self) -> *mut page_model::ffi::PageModel {
        self.reader.pages.as_ptr().cast_mut()
    }

    pub(super) fn reader_image_width(&self) -> i32 {
        self.reader
            .pages
            .dimensions(self.reader.page_index as usize)
            .0
    }

    pub(super) fn reader_image_height(&self) -> i32 {
        self.reader
            .pages
            .dimensions(self.reader.page_index as usize)
            .1
    }

    pub(super) fn reader_chapters(&self) -> *mut reader_models::ffi::ChapterModel {
        self.reader.chapters.as_ptr().cast_mut()
    }

    pub(super) fn reader_open(&self) -> bool {
        self.reader.reference.is_some()
    }

    pub(super) fn reader_busy(&self) -> bool {
        self.reader.busy
    }

    pub(super) fn reader_has_chapters(&self) -> bool {
        self.reader.reference.is_some() && self.reader.chapters.first().is_some()
    }

    pub(super) fn reader_error_text(&self) -> QString {
        if self.reader_mode() == ffi::ReaderMode::Paged
            && self.reader.pages.failed(self.reader.page_index as usize)
        {
            ffi::reader_error(ffi::ReaderError::Page)
        } else {
            self.reader.error.clone()
        }
    }

    pub(super) fn reader_title(&self) -> &QString {
        &self.selected_work_title
    }

    pub(super) fn reader_chapter_title(&self) -> &QString {
        &self.reader.chapter_title
    }

    pub(super) fn reader_chapter_id(&self) -> &QString {
        &self.reader.chapter_id
    }

    pub(super) fn reader_page_url(&self) -> QUrl {
        self.reader.pages.url(self.reader.page_index as usize)
    }

    pub(super) fn reader_page_index(&self) -> &i32 {
        &self.reader.page_index
    }

    pub(super) fn reader_page_count(&self) -> i32 {
        self.reader.pages.count() as i32
    }

    pub(super) fn reader_scroll_offset(&self) -> &f64 {
        &self.reader.scroll_offset
    }

    pub(super) fn chapter_query(&self) -> &QString {
        &self.reader.query
    }

    pub fn open_reader(mut self: Pin<&mut Self>) {
        if self.reader.reference.is_some() || self.inspecting {
            return;
        }
        let Some(variant) = self.variants.find(&self.selected_variant_id.to_string()) else {
            return;
        };
        if !matches!(variant.transport, Transport::Manga | Transport::Text)
            || self.services.is_none()
            || !self.source_enabled(variant.reference.publication.source)
        {
            return;
        }
        let selected = self.selected_variant_id.clone();
        let is_text = variant.transport == Transport::Text;
        let reference = variant.reference.clone();
        self.as_mut().set_chapter_query(QString::default());
        if self.reader.reference.is_some()
            || self.inspecting
            || self.selected_variant_id != selected
        {
            return;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.cancel();
            state.reader.reference = Some(reference);
            state.reader.is_text = is_text;
            state.reader.text.cache.clear();
            state.reader.text.original = None;
            state.reader.text.anchor = QString::default();
            state.reader.text.offset = 0.0;
            state.reader.chapter_id = QString::default();
            state.reader.chapter_title = QString::default();
            state.reader.page_index = 0;
            state.reader.scroll_offset = 0.0;
            state.reader.shown_position = None;
            state.reader.resume = None;
            state.reader.busy = true;
            state.reader.error = QString::default();
        }
        let generation = self.reader.loading.generation;
        let chapter_generation = self.reader.chapter.generation;
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .translation_mode(false);
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .reset(chapter_generation, 0);
        if self.reader.reference.is_none() || self.reader.loading.generation != generation {
            return;
        }
        self.as_mut()
            .rust_mut()
            .reader
            .text
            .blocks
            .pin_mut()
            .replace(None);
        if self.reader.reference.is_none() || self.reader.loading.generation != generation {
            return;
        }
        self.as_mut()
            .rust_mut()
            .reader
            .chapters
            .pin_mut()
            .replace(Vec::new());
        if self.reader.reference.is_none()
            || self.reader.loading.generation != generation
            || !self.reader.chapter_id.is_empty()
        {
            return;
        }
        self.as_mut().load_reader_resume();
    }

    pub fn close_reader(mut self: Pin<&mut Self>) {
        if self.reader.reference.is_none() {
            return;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.cancel();
            state.reader.busy = false;
            state.reader.error = QString::default();
            state.reader.reference = None;
            state.reader.text.cache.clear();
            state.reader.text.original = None;
        }
        let generation = self.reader.chapter.generation;
        self.as_mut().reader_assets_cleared(generation);
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .translation_mode(false);
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .reset(generation, 0);
        if self.reader.reference.is_some() || self.reader.chapter.generation != generation {
            return;
        }
        self.as_mut()
            .rust_mut()
            .reader
            .text
            .blocks
            .pin_mut()
            .replace(None);
        self.as_mut().reader_changed();
    }

    pub fn retry_reader(self: Pin<&mut Self>) {
        if self.reader.reference.is_none() || self.reader.busy {
            return;
        }
        if self.library_state == ffi::LibraryState::Failed {
            self.retry_library();
            return;
        }
        if self.reader.chapter_id.is_empty() {
            self.load_reader_resume();
            return;
        }
        if self.reader.pages.count() > 0
            && self.reader.pages.failed(self.reader.page_index as usize)
        {
            let generation = self.reader.chapter.generation;
            let index = self.reader.page_index;
            self.retry_reader_page(generation, index);
            return;
        }
        let chapter = self.reader.chapter_id.clone();
        let position = self.reader_current_position();
        self.load_chapter(chapter, position);
    }
}
