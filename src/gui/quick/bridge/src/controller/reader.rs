use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};
use qbutt_content::{
    ChapterRef, LibraryResult, ReaderProgress, Source, SourceOperation, Transport, VariantRef,
};
use tokio::sync::watch;

use super::library::{LibraryAction, ReaderSave};
use super::reader_pages::{PageEvent, read_chapter};
use super::sources::{SourceRequest, SourceResponse, execute_operation};
use super::{Pending, ffi};
use crate::page_model;
use crate::reader_models;

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
    reference: Option<VariantRef>,
    busy: bool,
    error: QString,
    chapter_title: QString,
    chapter_id: QString,
    page_index: i32,
    scroll_offset: f64,
    shown_position: Option<(i32, f64)>,
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
    pub(super) fn complete_reader_manifest(
        mut self: Pin<&mut Self>,
        generation: u64,
        count: usize,
    ) {
        if self.reader.chapter.generation != generation || self.reader.reference.is_none() {
            return;
        }
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .reset(generation, count);
        if self.reader.chapter.generation != generation || self.reader.reference.is_none() {
            return;
        }
        self.as_mut().reader_changed();
        if self.reader.chapter.generation == generation && self.reader.reference.is_some() {
            let index = self.reader.page_index;
            let offset = self.reader.scroll_offset;
            self.reader_position_requested(generation, index, offset);
        }
    }

    pub(super) fn complete_reader_page(
        mut self: Pin<&mut Self>,
        generation: u64,
        request: ReaderViewport,
        index: usize,
        event: PageEvent,
    ) {
        if self.reader.chapter.generation != generation || self.reader.reference.is_none() {
            return;
        }
        let Some(viewport) = self
            .reader
            .position
            .as_ref()
            .map(|position| *position.borrow())
        else {
            return;
        };
        if viewport != request
            || (index != viewport.anchor
                && !viewport
                    .retained(self.reader.pages.count())
                    .contains(&index))
        {
            return;
        }
        match event {
            PageEvent::Loading => self
                .as_mut()
                .rust_mut()
                .reader
                .pages
                .pin_mut()
                .loading(generation, index),
            PageEvent::Ready(page) => {
                if self.reader.pages.failed(index) {
                    return;
                }
                self.as_mut()
                    .rust_mut()
                    .reader
                    .pages
                    .pin_mut()
                    .ready(generation, index, page);
            }
            PageEvent::Failed => self
                .as_mut()
                .rust_mut()
                .reader
                .pages
                .pin_mut()
                .fail(generation, index),
        }
        if self.reader.chapter.generation != generation
            || self.reader.reference.is_none()
            || self
                .reader
                .position
                .as_ref()
                .map(|position| *position.borrow())
                != Some(request)
        {
            return;
        }
        let index = self.reader.page_index as usize;
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.busy = state.reader.error.is_empty()
                && state.reader.pages.url(index).is_empty()
                && !state.reader.pages.failed(index);
        }
        self.reader_changed();
    }

    pub(super) fn complete_reader_error(
        mut self: Pin<&mut Self>,
        generation: u64,
        page: Option<usize>,
        error: ffi::ReaderError,
    ) {
        if self.reader.chapter.generation != generation
            || self.reader.reference.is_none()
            || page.is_some_and(|page| page != self.reader.page_index as usize)
        {
            return;
        }
        self.as_mut().rust_mut().reader.error = ffi::reader_error(error);
        self.as_mut().rust_mut().reader.busy = false;
        self.reader_changed();
    }

    pub(super) fn reader_mode_changed(mut self: Pin<&mut Self>) {
        if self.reader.reference.is_none() || self.reader.pages.count() as i32 == 0 {
            return;
        }
        let generation = self.reader.chapter.generation;
        let index = self.reader.page_index;
        let offset = self.reader.scroll_offset;
        if self
            .as_mut()
            .apply_reader_window(ReaderViewport::page(index as usize))
            && self.reader.chapter.generation == generation
            && self.reader.page_index == index
            && self.reader.scroll_offset == offset
        {
            self.reader_position_requested(generation, index, offset);
        }
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

    pub(super) fn reader_progress_error_text(&self) -> &QString {
        &self.reader.progress_error
    }

    pub(super) fn reader_progress_pending(&self) -> bool {
        self.reader.progress_pending
    }

    pub(super) fn reader_retry_generation(&self) -> Option<u64> {
        (self.reader.reference.is_some() && self.reader.chapter_id.is_empty())
            .then_some(self.reader.loading.generation)
    }

    pub(super) fn resume_reader_after_library_retry(self: Pin<&mut Self>, generation: u64) {
        if self.reader_retry_generation() == Some(generation) {
            self.load_reader_resume();
        }
    }

    pub(super) fn prepare_reader_library_retry(mut self: Pin<&mut Self>, generation: u64) {
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

    pub(super) fn reader_library_failed(mut self: Pin<&mut Self>) -> bool {
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
        if variant.transport != Transport::Manga
            || self.services.is_none()
            || !self.source_enabled(variant.reference.publication.source)
        {
            return;
        }
        let selected = self.selected_variant_id.clone();
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
            .reset(chapter_generation, 0);
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
        }
        let generation = self.reader.chapter.generation;
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .reset(generation, 0);
        self.as_mut().reader_changed();
    }

    fn load_reader_resume(mut self: Pin<&mut Self>) {
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
        if !self.as_mut().submit_library(LibraryAction::ReaderResume {
            reference: reference.publication,
            generation,
        }) && self.reader.loading.generation == generation
            && self.reader.reference.is_some()
        {
            self.as_mut().rust_mut().reader.busy = false;
            self.as_mut().rust_mut().reader.error =
                ffi::content_error(ffi::ContentError::LibraryOpen);
            self.as_mut().reader_changed();
        }
    }

    pub(super) fn complete_reader_resume(
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

    fn load_chapters(mut self: Pin<&mut Self>) {
        let Some(reference) = self.reader.reference.clone() else {
            return;
        };
        let Some(request) = self.source_request(SourceOperation::Chapters(reference)) else {
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
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let anime = Arc::clone(&services.anime);
        let manga = Arc::clone(&services.manga);
        let mangalib = Arc::clone(&services.mangalib);
        let rutor = Arc::clone(&services.rutor);
        let nyaa = Arc::clone(&services.nyaa);
        let task = services.runtime.spawn(async move {
            let result =
                execute_operation(&request.operation, anime, manga, rutor, nyaa, mangalib).await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.reader.loading.generation != generation
                    || controller.reader.reference.is_none()
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                controller.as_mut().rust_mut().reader.loading.task = None;
                controller.as_mut().source_outcome(&request, &result);
                if controller.reader.loading.generation != generation
                    || controller.reader.reference.is_none()
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                match result {
                    Ok(SourceResponse::Chapters(chapters)) => {
                        controller
                            .as_mut()
                            .rust_mut()
                            .reader
                            .chapters
                            .pin_mut()
                            .replace(chapters);
                        if controller.reader.loading.generation != generation
                            || controller.reader.reference.is_none()
                        {
                            return;
                        }
                        if !controller.reader.chapter_id.is_empty() {
                            return;
                        }
                        if let Some(progress) = controller.reader.resume.clone() {
                            if controller
                                .reader
                                .chapters
                                .find(&progress.chapter.id)
                                .is_some()
                            {
                                controller.load_chapter(
                                    QString::from(&progress.chapter.id),
                                    progress.page as usize,
                                    progress.scroll_offset,
                                );
                                return;
                            }
                            controller.as_mut().rust_mut().reader.error =
                                ffi::reader_error(ffi::ReaderError::ChapterUnavailable);
                        } else if let Some(first) = controller.reader.chapters.first() {
                            controller.select_chapter(QString::from(first.id));
                            return;
                        } else {
                            controller.as_mut().rust_mut().reader.error =
                                ffi::reader_error(ffi::ReaderError::ChapterUnavailable);
                        }
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("chapters"),
                            &QString::from(error.to_string()),
                        );
                        controller.as_mut().rust_mut().reader.error =
                            ffi::reader_error(ffi::ReaderError::Chapters);
                    }
                    Ok(_) => unreachable!("chapters operation response"),
                }
                controller.as_mut().rust_mut().reader.busy = false;
                controller.as_mut().reader_changed();
            });
        });
        self.as_mut().rust_mut().reader.loading.task = Some(task);
    }

    pub fn select_chapter(self: Pin<&mut Self>, id: QString) {
        if self.reader.chapter_id == id && self.reader.error.is_empty() {
            return;
        }
        self.load_chapter(id, 0, 0.0);
    }

    fn load_chapter(mut self: Pin<&mut Self>, id: QString, page: usize, scroll_offset: f64) {
        if self.reader.reference.is_none() {
            return;
        }
        let Some(chapter) = self.reader.chapters.find(&id.to_string()).cloned() else {
            return;
        };
        if Some(&chapter.reference.variant) != self.reader.reference.as_ref() {
            return;
        }
        let Some(request) =
            self.source_request(SourceOperation::Chapter(chapter.reference.clone()))
        else {
            return;
        };
        self.as_mut().rust_mut().reader.loading.cancel();
        self.as_mut().rust_mut().reader.chapter.cancel();
        let generation = self.reader.chapter.generation;
        let (sender, position) = watch::channel(ReaderViewport::page(page));
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.chapter_id = id;
            state.reader.chapter_title = ffi::chapter_title(
                &QString::from(chapter.number.as_deref().unwrap_or_default()),
                &QString::from(chapter.volume.as_deref().unwrap_or_default()),
                &QString::from(chapter.title.as_deref().unwrap_or_default()),
            );
            state.reader.page_index = page as i32;
            state.reader.scroll_offset = scroll_offset;
            state.reader.shown_position = None;
            state.reader.resume = None;
            state.reader.error = QString::default();
            state.reader.busy = true;
            state.reader.position = Some(sender);
        }
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .reset(generation, 0);
        if self.reader.reference.is_none() || self.reader.chapter.generation != generation {
            return;
        }
        self.as_mut().reader_changed();
        if self.reader.reference.is_none() || self.reader.chapter.generation != generation {
            return;
        }
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let Some(source) = services.reader_source(chapter.reference.variant.publication.source)
        else {
            return;
        };
        let load_limit = Arc::clone(&services.reader_load);
        let byte_limit = Arc::clone(&services.reader_bytes);
        let task = services.runtime.spawn(read_chapter(
            source, request, position, qt_thread, generation, load_limit, byte_limit,
        ));
        self.as_mut().rust_mut().reader.chapter.task = Some(task);
    }

    pub fn select_reader_page(mut self: Pin<&mut Self>, index: i32) {
        if self.reader.reference.is_none() || index < 0 || index >= self.reader.pages.count() as i32
        {
            return;
        }
        if !self.reader.error.is_empty() {
            let chapter = self.reader.chapter_id.clone();
            self.load_chapter(chapter, index as usize, 0.0);
            return;
        }
        if index == self.reader.page_index && self.reader.scroll_offset == 0.0 {
            return;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.page_index = index;
            state.reader.scroll_offset = 0.0;
            state.reader.error = QString::default();
            state.reader.busy = true;
        }
        if !self
            .as_mut()
            .apply_reader_window(ReaderViewport::page(index as usize))
        {
            return;
        }
        self.as_mut().reader_changed();
        if self.reader.reference.is_some()
            && self.reader.page_index == index
            && self.reader.scroll_offset == 0.0
        {
            let generation = self.reader.chapter.generation;
            self.reader_position_requested(generation, index, 0.0);
        }
    }

    pub fn turn_reader_page(self: Pin<&mut Self>, offset: i32) {
        let index = self.reader.page_index.saturating_add(offset);
        self.select_reader_page(index);
    }

    /// Only visible Image.Ready for this exact row and local URL confirms a page.
    pub fn reader_page_shown(mut self: Pin<&mut Self>, index: i32, url: QUrl) {
        if self.reader.reference.is_none()
            || index < 0
            || self.reader.pages.chapter_generation() != self.reader.chapter.generation
            || url.is_empty()
            || !self.reader.position.as_ref().is_some_and(|position| {
                let viewport = *position.borrow();
                (viewport.first..=viewport.last).contains(&(index as usize))
            })
            || !self
                .as_mut()
                .rust_mut()
                .reader
                .pages
                .pin_mut()
                .confirm(index as usize, &url)
        {
            return;
        }
        if self.reader.page_index == index {
            self.save_reader_position();
        }
    }

    pub fn reader_page_failed(mut self: Pin<&mut Self>, index: i32, url: QUrl) {
        if self.reader.reference.is_none()
            || index < 0
            || url.is_empty()
            || self.reader.pages.chapter_generation() != self.reader.chapter.generation
            || self.reader.pages.url(index as usize) != url
        {
            return;
        }
        let generation = self.reader.chapter.generation;
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .fail(generation, index as usize);
        if self.reader.chapter.generation == generation && self.reader.page_index == index {
            self.as_mut().rust_mut().reader.busy = false;
            self.reader_changed();
        }
    }

    /// Layout/restore reports only the loading window. User movement additionally
    /// replaces the desired anchor and saves it after that exact row was decoded.
    pub fn set_reader_viewport(
        mut self: Pin<&mut Self>,
        generation: u64,
        first: i32,
        last: i32,
        anchor: i32,
        offset: f64,
        user_moved: bool,
    ) -> bool {
        if self.reader.reference.is_none()
            || self.reader.chapter.generation != generation
            || !self.reader.error.is_empty()
            || self.reader_mode() != ffi::ReaderMode::Continuous
            || first < 0
            || anchor < first
            || anchor > last
            || last < first
            || last >= self.reader.pages.count() as i32
            || !offset.is_finite()
            || !(0.0..=1.0).contains(&offset)
        {
            return false;
        }
        let old = self
            .reader
            .position
            .as_ref()
            .map(|position| *position.borrow());
        let Some(old) = old else {
            return false;
        };
        let viewport = ReaderViewport {
            first: first as usize,
            last: last as usize,
            anchor: if user_moved {
                anchor
            } else {
                self.reader.page_index
            } as usize,
            retry: old.retry,
            revision: old.revision,
        };
        if user_moved {
            let mut state = self.as_mut().rust_mut();
            state.reader.page_index = anchor;
            state.reader.scroll_offset = offset;
        }
        if !self.as_mut().apply_reader_window(viewport) {
            return true;
        }
        if user_moved
            && self.reader.page_index == anchor
            && self.reader.scroll_offset == offset
            && self.reader.pages.shown(anchor as usize)
        {
            self.as_mut().save_reader_position();
        }
        if self.reader.chapter.generation == generation {
            self.reader_changed();
        }
        true
    }

    fn apply_reader_window(mut self: Pin<&mut Self>, mut viewport: ReaderViewport) -> bool {
        if viewport.anchor != self.reader.page_index as usize {
            return false;
        }
        let generation = self.reader.chapter.generation;
        let Some(position) = &self.reader.position else {
            return false;
        };
        let previous = *position.borrow();
        viewport.revision = previous.revision;
        let window_changed = previous != viewport;
        if window_changed {
            viewport.revision = previous.revision.wrapping_add(1);
            position.send_replace(viewport);
        }
        if window_changed {
            let retained = viewport.retained(self.reader.pages.count());
            self.as_mut().rust_mut().reader.pages.pin_mut().window(
                generation,
                viewport.first,
                viewport.last,
                *retained.start(),
                *retained.end(),
                viewport.anchor,
            );
        }
        if self.reader.chapter.generation != generation
            || self.reader.reference.is_none()
            || self.reader.page_index != viewport.anchor as i32
            || self
                .reader
                .position
                .as_ref()
                .map(|position| *position.borrow())
                != Some(viewport)
        {
            return false;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.busy = state.reader.pages.url(viewport.anchor).is_empty()
                && !state.reader.pages.failed(viewport.anchor)
                && state.reader.error.is_empty();
        }
        true
    }

    pub fn retry_reader_page(mut self: Pin<&mut Self>, generation: u64, index: i32) {
        if self.reader.reference.is_none()
            || self.reader.chapter.generation != generation
            || index < 0
            || !self.reader.pages.failed(index as usize)
        {
            return;
        }
        let Some(mut viewport) = self
            .reader
            .position
            .as_ref()
            .map(|position| *position.borrow())
        else {
            return;
        };
        viewport.revision = viewport.revision.wrapping_add(1);
        viewport.retry = Some((viewport.revision, index as usize));
        // Accept the request before model notifications. A reentrant navigation
        // may replace this window, but this retry cannot replace that navigation.
        self.reader
            .position
            .as_ref()
            .unwrap()
            .send_replace(viewport);
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .loading(generation, index as usize);
        if self.reader.chapter.generation == generation
            && self
                .reader
                .position
                .as_ref()
                .map(|position| *position.borrow())
                == Some(viewport)
            && self.as_mut().apply_reader_window(viewport)
        {
            self.as_mut().reader_changed();
            if self.reader.chapter.generation == generation && self.reader.page_index == index {
                let offset = self.reader.scroll_offset;
                self.reader_position_requested(generation, index, offset);
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

    pub(super) fn complete_reader_progress(mut self: Pin<&mut Self>, generation: u64, saved: bool) {
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

    fn save_reader_position(mut self: Pin<&mut Self>) {
        let position = (self.reader.page_index, self.reader.scroll_offset);
        if self.reader.shown_position == Some(position) {
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
            page: self.reader.page_index as u32,
            scroll_offset: self.reader.scroll_offset,
        };
        self.as_mut().rust_mut().reader.progress_save = Some(ReaderSave {
            work,
            preferences,
            progress,
        });
        self.as_mut().rust_mut().reader.shown_position = Some(position);
        self.submit_reader_progress();
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

    pub fn set_chapter_query(mut self: Pin<&mut Self>, query: QString) {
        if self.reader.query == query {
            return;
        }
        self.as_mut().rust_mut().reader.query = query.clone();
        self.as_mut()
            .rust_mut()
            .reader
            .chapters
            .pin_mut()
            .filter(&query.to_string());
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
        } else {
            if self.reader.pages.count() > 0
                && self.reader.pages.failed(self.reader.page_index as usize)
            {
                let generation = self.reader.chapter.generation;
                let index = self.reader.page_index;
                self.retry_reader_page(generation, index);
                return;
            }
            let chapter = self.reader.chapter_id.clone();
            let page = self.reader.page_index as usize;
            let offset = self.reader.scroll_offset;
            self.load_chapter(chapter, page, offset);
        }
    }
}
