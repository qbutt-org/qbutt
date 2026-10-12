use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};

use super::super::ffi;
use super::super::reader_pages::PageEvent;
use super::ReaderViewport;

impl ffi::ContentController {
    pub(in crate::controller) fn complete_reader_manifest(
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

    pub(in crate::controller) fn complete_reader_page(
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
        self.as_mut().reader_changed();
        self.as_mut().start_manga_translation();
    }

    pub(in crate::controller) fn complete_reader_error(
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

    pub(in crate::controller) fn reader_mode_changed(mut self: Pin<&mut Self>) {
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

    pub fn select_reader_page(mut self: Pin<&mut Self>, index: i32) {
        if self.reader.reference.is_none() || index < 0 || index >= self.reader.pages.count() as i32
        {
            return;
        }
        if !self.reader.error.is_empty() {
            let chapter = self.reader.chapter_id.clone();
            self.load_chapter(
                chapter,
                qbutt_content::ReaderPosition::Page {
                    page: index as u32,
                    scroll_offset: 0.0,
                },
            );
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
        self.as_mut().start_manga_translation();
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
        {
            return;
        }
        let confirmed = self
            .as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .confirm(index as usize, &url);
        if confirmed && self.reader.page_index == index {
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
        let Some(previous) = self
            .reader
            .position
            .as_ref()
            .map(|position| *position.borrow())
        else {
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
            retry: previous.retry,
            revision: previous.revision,
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
            self.as_mut().reader_changed();
            self.as_mut().start_manga_translation();
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
        if previous != viewport {
            viewport.revision = previous.revision.wrapping_add(1);
            position.send_replace(viewport);
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
        if self.reader.chapter.generation != generation
            || self
                .reader
                .position
                .as_ref()
                .map(|position| *position.borrow())
                != Some(viewport)
        {
            return;
        }
        if !self.as_mut().apply_reader_window(viewport) {
            return;
        }
        self.as_mut().reader_changed();
        if self.reader.chapter.generation == generation && self.reader.page_index == index {
            let offset = self.reader.scroll_offset;
            self.reader_position_requested(generation, index, offset);
        }
    }
}
