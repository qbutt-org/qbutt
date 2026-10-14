use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_codex::{self as codex, CodexError, ModelModality};
use qbutt_content::translation::context_tools::{
    ChapterExcerpt, WorkChapterSource, WorkContextTools,
};
use qbutt_content::translation::manga_imagegen;
use qbutt_content::translation::page::{self, PageJob};
use qbutt_content::{Chapter, ChapterRef, LibraryError, LocalBook, TranslationSelection};
use tokio::sync::{Semaphore, watch};

use super::super::catalog::Catalog;
use super::super::library::LibraryWorker;
use super::super::reader_pages::CachedPage;
use super::super::{Pending, ffi};
use crate::QtMangaRenderer;

#[derive(Default)]
pub(super) struct MangaTranslationState {
    pub(super) requested: bool,
    pub(super) busy: bool,
    pub(super) error: Option<TranslationFailure>,
    pub(super) pending: Pending,
    pub(super) page: Option<usize>,
    cancellation: Option<watch::Sender<bool>>,
}

impl MangaTranslationState {
    pub(super) fn cancel_job(&mut self) {
        if let Some(cancel) = self.cancellation.take() {
            let _ = cancel.send(true);
        }
        self.pending.cancel();
        self.page = None;
        self.busy = false;
    }

    pub(super) fn reset_chapter(&mut self) {
        self.cancel_job();
        self.error = None;
    }

    pub(super) fn reset_reader(&mut self) {
        self.reset_chapter();
        self.requested = false;
    }
}

pub(super) enum TranslationFailure {
    UnsupportedSize,
    Codex(CodexError),
    Library(LibraryError),
    Cache(String),
}

impl TranslationFailure {
    pub(super) fn needs_codex(&self) -> bool {
        matches!(
            self,
            Self::Codex(CodexError::Authentication | CodexError::Unavailable)
        )
    }

    fn detail(&self) -> String {
        match self {
            Self::UnsupportedSize => "This page is too large to translate".into(),
            Self::Codex(error) => error.to_string(),
            Self::Library(error) => error.to_string(),
            Self::Cache(error) => error.clone(),
        }
    }
}

struct MangaChapterSource {
    catalog: Arc<Catalog>,
    local_book: Option<Arc<LocalBook>>,
}

impl WorkChapterSource for MangaChapterSource {
    fn load(
        &self,
        chapter: ChapterRef,
        offset: usize,
    ) -> Pin<Box<dyn Future<Output = codex::Result<ChapterExcerpt>> + Send + '_>> {
        Box::pin(async move {
            if let Some(book) = self.local_book.as_ref().cloned() {
                return tokio::task::spawn_blocking(move || {
                    let pages = book
                        .chapter_pages(&chapter)
                        .map_err(|_| CodexError::Provider)?;
                    let image = pages.images.get(offset).ok_or(CodexError::Protocol)?;
                    let bytes = book.image_bytes(image).map_err(|_| CodexError::Provider)?;
                    Ok(ChapterExcerpt {
                        chapter,
                        offset,
                        total_parts: pages.images.len(),
                        parts: Vec::new(),
                        image: Some(page::input_image(bytes)?),
                    })
                })
                .await
                .map_err(|_| CodexError::Protocol)?;
            }
            let mut source = self
                .catalog
                .reader_source(chapter.variant.publication.source)
                .await
                .map_err(|_| CodexError::Provider)?;
            let pages = source
                .chapter_pages(&chapter)
                .await
                .map_err(|_| CodexError::Provider)?;
            if pages.reference != chapter || offset >= pages.images.len() {
                return Err(CodexError::Protocol);
            }
            let bytes = source
                .remote_image_bytes(&pages.images[offset])
                .await
                .map_err(|_| CodexError::Provider)?;
            Ok(ChapterExcerpt {
                chapter,
                offset,
                total_parts: pages.images.len(),
                parts: Vec::new(),
                image: Some(page::input_image(bytes)?),
            })
        })
    }
}

async fn translate(
    original: Arc<CachedPage>,
    chapter: ChapterRef,
    page: usize,
    chapters: Vec<Chapter>,
    source: MangaChapterSource,
    library: LibraryWorker,
    byte_limit: Arc<Semaphore>,
    language: String,
    cancelled: watch::Receiver<bool>,
) -> Result<Arc<CachedPage>, TranslationFailure> {
    if !ffi::manga_translation_size_supported(original.width, original.height) {
        return Err(TranslationFailure::UnsupportedSize);
    }
    let bytes = CachedPage::original_bytes(Arc::clone(&original))
        .await
        .map_err(TranslationFailure::Cache)?;
    if bytes.len() > codex::MAX_IMAGE_BYTES {
        return Err(TranslationFailure::UnsupportedSize);
    }
    let input = page::input_image(bytes).map_err(TranslationFailure::Codex)?;
    let saved = library
        .translation_selection(language.clone())
        .await
        .map_err(TranslationFailure::Library)?;
    if let Some(selection) = &saved {
        let key = manga_imagegen::cache_key(
            chapter.clone(),
            page as u32,
            &input,
            &language,
            &selection.model,
            &selection.effort,
        );
        let lookup = library
            .manga_lookup(key)
            .await
            .map_err(TranslationFailure::Library)?;
        if let Some(png) = lookup.cached {
            if *cancelled.borrow() {
                return Err(TranslationFailure::Codex(CodexError::Cancelled));
            }
            let page = CachedPage::translated(original, png, byte_limit)
                .await
                .map_err(TranslationFailure::Cache)?;
            return Ok(Arc::new(page));
        }
    }

    let explicit = saved.as_ref().filter(|selection| selection.explicit);
    let (model, effort) = codex::resolve_selection(
        ModelModality::Image,
        explicit.map(|selection| selection.model.as_str()),
        explicit.map(|selection| selection.effort.as_str()),
    )
    .await
    .map_err(TranslationFailure::Codex)?;
    let key = manga_imagegen::cache_key(
        chapter.clone(),
        page as u32,
        &input,
        &language,
        &model,
        &effort,
    );
    let lookup = library
        .manga_lookup(key)
        .await
        .map_err(TranslationFailure::Library)?;
    if *cancelled.borrow() {
        return Err(TranslationFailure::Codex(CodexError::Cancelled));
    }
    let png = if let Some(png) = lookup.cached {
        library
            .remember_translation_selection(TranslationSelection {
                target_language: language,
                model,
                effort,
                explicit: explicit.is_some(),
            })
            .await
            .map_err(TranslationFailure::Library)?;
        png
    } else {
        let mut tools = WorkContextTools::new(
            &chapter,
            &chapters,
            &source,
            lookup.context.clone(),
            cancelled.clone(),
        )
        .map_err(TranslationFailure::Codex)?;
        let expected_context_revision = lookup.context.revision;
        let result = manga_imagegen::translate_page(
            PageJob {
                chapter,
                page: page as u32,
                original: input,
                target_language: language,
                work_context: lookup.context,
                model: Some(model),
                effort: Some(effort),
            },
            Arc::new(QtMangaRenderer),
            &mut tools,
            cancelled.clone(),
        )
        .await
        .map_err(TranslationFailure::Codex)?;
        if *cancelled.borrow() {
            return Err(TranslationFailure::Codex(CodexError::Cancelled));
        }
        library
            .save_manga_translation(
                lookup.key,
                expected_context_revision,
                result.png.clone(),
                result.additions,
                result.evidence,
                cancelled.clone(),
            )
            .await
            .map_err(TranslationFailure::Library)?;
        result.png
    };
    if *cancelled.borrow() {
        return Err(TranslationFailure::Codex(CodexError::Cancelled));
    }
    let page = CachedPage::translated(original, png, byte_limit)
        .await
        .map_err(TranslationFailure::Cache)?;
    Ok(Arc::new(page))
}

impl ffi::ContentController {
    pub(in crate::controller) fn reader_translation_image_size_error(&self) -> bool {
        !self.reader.is_text
            && matches!(
                self.reader.manga_translation.error,
                Some(TranslationFailure::UnsupportedSize)
            )
    }

    pub fn set_reader_manga_translation(mut self: Pin<&mut Self>, translated: bool) -> bool {
        if self.reader.is_text || self.reader.reference.is_none() {
            return false;
        }
        if !translated {
            let state = &mut self.as_mut().rust_mut().reader.manga_translation;
            state.reset_reader();
            self.as_mut()
                .rust_mut()
                .reader
                .pages
                .pin_mut()
                .translation_mode(false);
            self.reader_changed();
            return true;
        }
        let state = &mut self.as_mut().rust_mut().reader.manga_translation;
        state.requested = true;
        state.error = None;
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .translation_mode(true);
        self.as_mut().reader_changed();
        self.as_mut().start_manga_translation();
        true
    }

    pub(super) fn start_manga_translation(mut self: Pin<&mut Self>) {
        if self.reader.is_text
            || !self.reader.manga_translation.requested
            || self.reader.reference.is_none()
            || !self.reader.error.is_empty()
            || self.reader.page_index < 0
        {
            return;
        }
        let index = self.reader.page_index as usize;
        if self.reader.pages.translated_ready(index) {
            if self.reader.manga_translation.busy {
                self.as_mut()
                    .rust_mut()
                    .reader
                    .manga_translation
                    .cancel_job();
                self.reader_changed();
            }
            return;
        }
        if self.reader.manga_translation.busy && self.reader.manga_translation.page == Some(index) {
            return;
        }
        let Some(original) = self.reader.pages.original(index) else {
            let was_busy = self.reader.manga_translation.busy;
            self.as_mut()
                .rust_mut()
                .reader
                .manga_translation
                .cancel_job();
            if was_busy {
                self.reader_changed();
            }
            return;
        };
        let original_url = original.url.clone();
        let Some(chapter) = self
            .reader
            .chapters
            .find(&self.reader.chapter_id.to_string())
        else {
            return;
        };
        let Some(library) = self.library_worker.as_ref().cloned() else {
            return;
        };
        let Some(services) = self.services.as_ref() else {
            return;
        };
        let chapter = chapter.reference.clone();
        let chapters = self.reader.chapters.all();
        let source = MangaChapterSource {
            catalog: Arc::clone(&services.catalog),
            local_book: self
                .local_book
                .as_ref()
                .cloned()
                .filter(|book| book.reference() == chapter.variant.publication),
        };
        let byte_limit = Arc::clone(&services.reader_bytes);
        let language = self.reader_settings.value.translation_language.clone();
        let qt_thread = self.qt_thread();
        let chapter_generation = self.reader.chapter.generation;
        let (cancel, cancelled) = watch::channel(false);
        {
            let state = &mut self.as_mut().rust_mut().reader.manga_translation;
            state.cancel_job();
            state.page = Some(index);
            state.busy = true;
            state.cancellation = Some(cancel);
        }
        let generation = self.reader.manga_translation.pending.generation;
        self.as_mut().reader_changed();
        if self.reader.chapter.generation != chapter_generation
            || self.reader.manga_translation.pending.generation != generation
        {
            return;
        }
        let task = self.services.as_ref().unwrap().runtime.spawn(async move {
            let result = translate(
                original, chapter, index, chapters, source, library, byte_limit, language,
                cancelled,
            )
            .await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.reader.chapter.generation != chapter_generation
                    || controller.reader.manga_translation.pending.generation != generation
                    || controller.reader.manga_translation.page != Some(index)
                {
                    return;
                }
                if controller
                    .reader
                    .pages
                    .original(index)
                    .as_ref()
                    .is_none_or(|page| page.url != original_url)
                {
                    controller
                        .as_mut()
                        .rust_mut()
                        .reader
                        .manga_translation
                        .cancel_job();
                    controller.as_mut().reader_changed();
                    controller.as_mut().start_manga_translation();
                    return;
                }
                {
                    let state = &mut controller.as_mut().rust_mut().reader.manga_translation;
                    state.pending.task = None;
                    state.cancellation = None;
                    state.page = None;
                    state.busy = false;
                }
                match result {
                    Ok(page) => {
                        controller
                            .as_mut()
                            .rust_mut()
                            .reader
                            .pages
                            .pin_mut()
                            .translated(chapter_generation, index, page);
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("manga translation"),
                            &QString::from(error.detail()),
                        );
                        let state = &mut controller.as_mut().rust_mut().reader.manga_translation;
                        state.requested = false;
                        state.error = Some(error);
                        controller
                            .as_mut()
                            .rust_mut()
                            .reader
                            .pages
                            .pin_mut()
                            .translation_mode(false);
                    }
                }
                controller.reader_changed();
            });
        });
        self.as_mut()
            .rust_mut()
            .reader
            .manga_translation
            .pending
            .task = Some(task);
    }
}
