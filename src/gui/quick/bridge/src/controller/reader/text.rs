use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{ReaderPosition, Source, SourceOperation, TextChapter, TextChapterCache};

use super::super::sources::SourceRequest;
use super::super::{Pending, ffi};
use crate::text_model;

pub(super) struct TextReaderState {
    pub(super) blocks: UniquePtr<text_model::ffi::TextBlockModel>,
    pub(super) original: Option<Arc<TextChapter>>,
    pub(super) anchor: QString,
    pub(super) offset: f64,
    pub(super) cache: TextChapterCache,
    pub(super) prefetch: Pending,
}

impl Default for TextReaderState {
    fn default() -> Self {
        Self {
            blocks: text_model::ffi::create_text_block_model(),
            original: None,
            anchor: QString::default(),
            offset: 0.0,
            cache: TextChapterCache::default(),
            prefetch: Pending::default(),
        }
    }
}

impl ffi::ContentController {
    pub(in crate::controller) fn reader_is_text(&self) -> bool {
        self.reader.is_text
    }

    pub(in crate::controller) fn reader_text_blocks(&self) -> *mut text_model::ffi::TextBlockModel {
        self.reader.text.blocks.as_ptr().cast_mut()
    }

    pub(in crate::controller) fn reader_text_generation(&self) -> u64 {
        self.reader.chapter.generation
    }

    pub(in crate::controller) fn reader_text_anchor(&self) -> &QString {
        &self.reader.text.anchor
    }

    pub(in crate::controller) fn reader_text_offset(&self) -> f64 {
        self.reader.text.offset
    }

    pub(super) fn load_text_chapter(
        mut self: Pin<&mut Self>,
        request: SourceRequest,
        resume: ReaderPosition,
        generation: u64,
    ) {
        let SourceOperation::Chapter(reference) = &request.operation else {
            return;
        };
        let cached = self.as_mut().rust_mut().reader.text.cache.get(reference);
        if let Some(chapter) = cached {
            self.complete_text_chapter(request, resume, generation, Ok(chapter));
            return;
        }
        let reference = reference.clone();
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        if reference.variant.publication.source == Source::LocalBook {
            let Some(book) = self.local_book.as_ref().cloned() else {
                return;
            };
            let task = services.runtime.spawn_blocking(move || {
                let result = book.read_chapter(&reference);
                let _ = qt_thread.queue(move |mut controller| {
                    if !controller.reader_source_current(generation, &request) {
                        return;
                    }
                    let result = result.and_then(|chapter| {
                        controller
                            .as_mut()
                            .rust_mut()
                            .reader
                            .text
                            .cache
                            .insert(chapter)
                    });
                    controller.complete_text_chapter(request, resume, generation, result);
                });
            });
            self.as_mut().rust_mut().reader.chapter.task = Some(task);
            return;
        }
        let catalog = Arc::clone(&services.catalog);
        let load_limit = Arc::clone(&services.reader_load);
        let task = services.runtime.spawn(async move {
            let Ok(_permit) = load_limit.acquire_owned().await else {
                return;
            };
            let result = catalog.text_chapter(&reference).await;
            let _ = qt_thread.queue(move |mut controller| {
                if !controller.reader_source_current(generation, &request) {
                    return;
                }
                let result = result.and_then(|chapter| {
                    if chapter.reference != reference {
                        return Err(qbutt_content::Error::InvalidResponse(
                            "chapter text identity does not match the request",
                        ));
                    }
                    controller
                        .as_mut()
                        .rust_mut()
                        .reader
                        .text
                        .cache
                        .insert(chapter)
                });
                controller.complete_text_chapter(request, resume, generation, result);
            });
        });
        self.as_mut().rust_mut().reader.chapter.task = Some(task);
    }

    fn complete_text_chapter(
        mut self: Pin<&mut Self>,
        request: SourceRequest,
        resume: ReaderPosition,
        generation: u64,
        result: qbutt_content::Result<Arc<TextChapter>>,
    ) {
        if !self.reader_source_current(generation, &request) {
            return;
        }
        self.as_mut().rust_mut().reader.chapter.task = None;
        self.as_mut().source_outcome(&request, &result);
        if !self.reader_source_current(generation, &request) {
            return;
        }
        let chapter = match result {
            Ok(chapter) => chapter,
            Err(error) => {
                ffi::log_content_error(
                    &QString::from("text chapter"),
                    &QString::from(error.to_string()),
                );
                self.as_mut().rust_mut().reader.error =
                    ffi::reader_error(ffi::ReaderError::Chapter);
                self.as_mut().rust_mut().reader.busy = false;
                self.reader_changed();
                return;
            }
        };
        self.as_mut().reader_assets_cleared(generation);
        for asset in &chapter.assets {
            self.as_mut().reader_asset(
                generation,
                QString::from(&asset.id),
                cxx_qt_lib::QByteArray::from(asset.bytes.as_ref()),
            );
        }
        let (anchor, offset) = match resume {
            ReaderPosition::Text { block_id, offset }
                if chapter.blocks.iter().any(|block| block.id == block_id) =>
            {
                (block_id, offset)
            }
            _ => (
                chapter
                    .entry_anchor
                    .clone()
                    .unwrap_or_else(|| chapter.blocks[0].id.clone()),
                0.0,
            ),
        };
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.text.original = Some(Arc::clone(&chapter));
            state.reader.text.anchor = QString::from(anchor);
            state.reader.text.offset = offset;
            state.reader.busy = false;
        }
        self.as_mut()
            .rust_mut()
            .reader
            .text
            .blocks
            .pin_mut()
            .replace(Some(chapter));
        if !self.reader_source_current(generation, &request) {
            return;
        }
        self.as_mut().reader_changed();
        if self.reader_source_current(generation, &request) {
            let anchor = self.reader.text.anchor.clone();
            let offset = self.reader.text.offset;
            self.as_mut()
                .reader_text_position_requested(generation, anchor, offset);
        }
        if self.reader_source_current(generation, &request) {
            self.as_mut().on_original_text_chapter_ready();
        }
    }

    /// The canvas confirms laid-out original text. A stale delegate or initial
    /// layout must not replace the requested restored position.
    pub fn set_reader_text_viewport(
        mut self: Pin<&mut Self>,
        generation: u64,
        block_id: QString,
        offset: f64,
        user_moved: bool,
    ) -> bool {
        if !self.reader.is_text
            || self.reader.reference.is_none()
            || self.reader.busy
            || self.reader.chapter.generation != generation
            || !self.reader.error.is_empty()
            || !offset.is_finite()
            || !(0.0..=1.0).contains(&offset)
            || self.reader.text.blocks.index_of_id(block_id.clone()) < 0
            || (!user_moved
                && (self.reader.text.anchor != block_id
                    || (self.reader.text.offset - offset).abs() > 0.001))
        {
            return false;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.text.anchor = block_id;
            state.reader.text.offset = offset;
        }
        let first_shown = self.reader.shown_position.is_none();
        self.as_mut().save_reader_position();
        if self.reader.chapter.generation == generation && first_shown {
            self.as_mut().prefetch_text_chapter(generation);
        }
        self.reader.chapter.generation == generation && self.reader.reference.is_some()
    }

    fn prefetch_text_chapter(mut self: Pin<&mut Self>, chapter_generation: u64) {
        if self
            .reader
            .reference
            .as_ref()
            .is_some_and(|reference| reference.publication.source == Source::LocalBook)
        {
            return;
        }
        let Some(next) = self
            .reader
            .chapters
            .relative(&self.reader.chapter_id.to_string(), 1)
        else {
            return;
        };
        if self
            .as_mut()
            .rust_mut()
            .reader
            .text
            .cache
            .get(&next)
            .is_some()
        {
            return;
        }
        let Some(request) = self.source_request(SourceOperation::Chapter(next.clone())) else {
            return;
        };
        self.as_mut().rust_mut().reader.text.prefetch.cancel();
        let generation = self.reader.text.prefetch.generation;
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let catalog = Arc::clone(&services.catalog);
        let load_limit = Arc::clone(&services.reader_load);
        let task = services.runtime.spawn(async move {
            let Ok(_permit) = load_limit.acquire_owned().await else {
                return;
            };
            let result = catalog.text_chapter(&next).await;
            let _ = qt_thread.queue(move |mut controller| {
                if !controller.reader_source_current(chapter_generation, &request)
                    || controller.reader.text.prefetch.generation != generation
                {
                    return;
                }
                controller.as_mut().rust_mut().reader.text.prefetch.task = None;
                // Speculative acquisition never records reading or a source
                // failure. Opening the chapter performs the normal outcome path.
                if let Ok(chapter) = result
                    && chapter.reference == next
                {
                    let _ = controller
                        .as_mut()
                        .rust_mut()
                        .reader
                        .text
                        .cache
                        .insert(chapter);
                }
            });
        });
        self.as_mut().rust_mut().reader.text.prefetch.task = Some(task);
    }
}
