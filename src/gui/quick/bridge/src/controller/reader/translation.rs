use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_codex::{self as codex, CodexError, ModelModality};
use qbutt_content::translation::text::{TextTranslationOptions, translate_chapter};
use qbutt_content::{
    Chapter, ChapterRef, Error, LibraryError, LocalBook, TextChapter, TranslationSelection,
};
use tokio::sync::watch;

use super::super::library::LibraryWorker;
use super::super::{Pending, ffi};

#[derive(Default)]
pub(super) struct TextTranslationState {
    pub(super) requested: bool,
    pub(super) busy: bool,
    pub(super) translated: Option<Arc<TextChapter>>,
    pub(super) error: bool,
    pub(super) needs_codex: bool,
    pub(super) pending: Pending,
    cancellation: Option<watch::Sender<bool>>,
}

impl TextTranslationState {
    fn cancel_job(&mut self) {
        if let Some(cancel) = self.cancellation.take() {
            let _ = cancel.send(true);
        }
        self.pending.cancel();
        self.busy = false;
    }

    pub(super) fn reset_chapter(&mut self) {
        self.cancel_job();
        self.translated = None;
        self.error = false;
        self.needs_codex = false;
    }

    pub(super) fn reset_reader(&mut self) {
        self.reset_chapter();
        self.requested = false;
    }
}

enum TranslationFailure {
    Codex(CodexError),
    Library(LibraryError),
}

impl TranslationFailure {
    fn needs_codex(&self) -> bool {
        matches!(
            self,
            Self::Codex(CodexError::Authentication | CodexError::Unavailable)
        )
    }

    fn detail(&self) -> String {
        match self {
            Self::Codex(error) => error.to_string(),
            Self::Library(error) => error.to_string(),
        }
    }
}

async fn translate(
    original: Arc<TextChapter>,
    chapters: Vec<Chapter>,
    catalog: Arc<super::super::catalog::Catalog>,
    local_book: Option<Arc<LocalBook>>,
    library: LibraryWorker,
    language: String,
    cancelled: watch::Receiver<bool>,
) -> Result<Arc<TextChapter>, TranslationFailure> {
    let saved = library
        .translation_selection(language.clone())
        .await
        .map_err(TranslationFailure::Library)?;
    if let Some(selection) = &saved {
        let lookup = library
            .translation_lookup(
                Arc::clone(&original),
                language.clone(),
                selection.model.clone(),
                selection.effort.clone(),
            )
            .await
            .map_err(TranslationFailure::Library)?;
        if let Some(cached) = lookup.cached {
            return Ok(Arc::new(cached));
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
    let lookup = library
        .translation_lookup(
            Arc::clone(&original),
            language.clone(),
            model.clone(),
            effort.clone(),
        )
        .await
        .map_err(TranslationFailure::Library)?;
    if let Some(cached) = lookup.cached {
        library
            .remember_translation_selection(TranslationSelection {
                target_language: language,
                model,
                effort,
                explicit: explicit.is_some(),
            })
            .await
            .map_err(TranslationFailure::Library)?;
        return Ok(Arc::new(cached));
    }

    let source = move |reference: ChapterRef| {
        let catalog = Arc::clone(&catalog);
        let local_book = local_book.clone();
        async move {
            if let Some(book) = local_book {
                tokio::task::spawn_blocking(move || book.read_chapter(&reference))
                    .await
                    .map_err(|_| Error::InvalidLocalBook("The book could not be read"))?
            } else {
                catalog.text_chapter(&reference).await
            }
        }
    };
    let result = translate_chapter(
        &original,
        &chapters,
        &source,
        &lookup.context,
        &TextTranslationOptions {
            target_language: language,
            model: Some(model),
            effort: Some(effort),
        },
        cancelled,
    )
    .await
    .map_err(TranslationFailure::Codex)?;
    library
        .save_translation(
            lookup.key,
            lookup.context.revision,
            result.chapter.clone(),
            result.additions,
            result.evidence,
        )
        .await
        .map_err(TranslationFailure::Library)?;
    Ok(Arc::new(result.chapter))
}

impl ffi::ContentController {
    pub(in crate::controller) fn reset_reader_translation(mut self: Pin<&mut Self>) {
        if self.reader.is_text {
            self.as_mut().set_reader_text_translation(false);
            self.as_mut().rust_mut().reader.translation.reset_chapter();
        } else {
            self.as_mut().set_reader_manga_translation(false);
            self.as_mut()
                .rust_mut()
                .reader
                .pages
                .pin_mut()
                .clear_translations();
        }
    }

    pub(in crate::controller) fn restore_reader_translation(mut self: Pin<&mut Self>) {
        if self.reader.is_text {
            self.as_mut().set_reader_text_translation(true);
        } else {
            self.as_mut().set_reader_manga_translation(true);
        }
    }

    pub(in crate::controller) fn reader_translation_requested(&self) -> bool {
        if self.reader.is_text {
            self.reader.translation.requested
        } else {
            self.reader.manga_translation.requested
        }
    }

    pub(in crate::controller) fn reader_translation_request_identity(&self) -> Option<(u64, u64)> {
        self.reader.reference.as_ref()?;
        let request_generation = if self.reader.is_text {
            self.reader.translation.pending.generation
        } else {
            self.reader.manga_translation.pending.generation
        };
        Some((self.reader.chapter.generation, request_generation))
    }

    pub(in crate::controller) fn reader_translation_busy(&self) -> bool {
        if self.reader.is_text {
            self.reader.translation.busy
        } else {
            self.reader.manga_translation.busy
        }
    }

    pub(in crate::controller) fn reader_translation_error(&self) -> bool {
        if self.reader.is_text {
            self.reader.translation.error
        } else {
            self.reader.manga_translation.error
        }
    }

    pub(in crate::controller) fn reader_translation_needs_codex(&self) -> bool {
        if self.reader.is_text {
            self.reader.translation.needs_codex
        } else {
            self.reader.manga_translation.needs_codex
        }
    }

    pub(super) fn on_original_text_chapter_ready(mut self: Pin<&mut Self>) {
        if self.reader.translation.requested {
            self.as_mut().set_reader_text_translation(true);
        }
    }

    pub fn set_reader_text_translation(mut self: Pin<&mut Self>, translated: bool) -> bool {
        if !self.reader.is_text || self.reader.reference.is_none() {
            return false;
        }
        let Some(original) = self.reader.text.original.as_ref().cloned() else {
            return false;
        };
        if !translated {
            let was_translated =
                self.reader.translation.requested && self.reader.translation.translated.is_some();
            let had_state = self.reader.translation.requested
                || self.reader.translation.busy
                || self.reader.translation.error;
            let state = &mut self.as_mut().rust_mut().reader.translation;
            state.cancel_job();
            state.requested = false;
            state.error = false;
            state.needs_codex = false;
            if was_translated {
                self.as_mut()
                    .rust_mut()
                    .reader
                    .text
                    .blocks
                    .pin_mut()
                    .replace(Some(original));
                if self.reader.reference.is_none() {
                    return true;
                }
                self.as_mut().reader_changed();
                if self.reader.reference.is_some() {
                    let generation = self.reader.chapter.generation;
                    let anchor = self.reader.text.anchor.clone();
                    let offset = self.reader.text.offset;
                    self.as_mut()
                        .reader_text_position_requested(generation, anchor, offset);
                }
            } else if had_state {
                self.as_mut().reader_changed();
            }
            return true;
        }
        if self.reader.translation.busy {
            return true;
        }
        if let Some(cached) = self.reader.translation.translated.as_ref().cloned() {
            self.as_mut().rust_mut().reader.translation.requested = true;
            self.as_mut()
                .rust_mut()
                .reader
                .text
                .blocks
                .pin_mut()
                .replace(Some(cached));
            if self.reader.reference.is_none() {
                return true;
            }
            self.as_mut().reader_changed();
            if self.reader.reference.is_some() {
                let generation = self.reader.chapter.generation;
                let anchor = self.reader.text.anchor.clone();
                let offset = self.reader.text.offset;
                self.as_mut()
                    .reader_text_position_requested(generation, anchor, offset);
            }
            return true;
        }
        let Some(library) = self.library_worker.as_ref().cloned() else {
            return false;
        };
        let Some(services) = self.services.as_ref() else {
            return false;
        };
        let chapters = self.reader.chapters.all();
        let catalog = Arc::clone(&services.catalog);
        let local_book = self
            .local_book
            .as_ref()
            .cloned()
            .filter(|book| book.reference() == original.reference.variant.publication);
        let language = self.reader_settings.value.translation_language.clone();
        let qt_thread = self.qt_thread();
        let chapter_generation = self.reader.chapter.generation;
        let (cancel, cancelled) = watch::channel(false);
        {
            let state = &mut self.as_mut().rust_mut().reader.translation;
            state.reset_chapter();
            state.requested = true;
            state.busy = true;
            state.cancellation = Some(cancel);
        }
        let generation = self.reader.translation.pending.generation;
        self.as_mut().reader_changed();
        if self.reader.translation.pending.generation != generation
            || self.reader.chapter.generation != chapter_generation
        {
            return true;
        }
        let task = self.services.as_ref().unwrap().runtime.spawn(async move {
            let result = translate(
                original, chapters, catalog, local_book, library, language, cancelled,
            )
            .await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.reader.chapter.generation != chapter_generation
                    || controller.reader.translation.pending.generation != generation
                    || !controller.reader.translation.requested
                {
                    return;
                }
                controller
                    .as_mut()
                    .rust_mut()
                    .reader
                    .translation
                    .pending
                    .task = None;
                controller
                    .as_mut()
                    .rust_mut()
                    .reader
                    .translation
                    .cancellation = None;
                controller.as_mut().rust_mut().reader.translation.busy = false;
                match result {
                    Ok(chapter) => {
                        controller.as_mut().rust_mut().reader.translation.translated =
                            Some(Arc::clone(&chapter));
                        controller
                            .as_mut()
                            .rust_mut()
                            .reader
                            .text
                            .blocks
                            .pin_mut()
                            .replace(Some(chapter));
                        if controller.reader.chapter.generation != chapter_generation
                            || !controller.reader.translation.requested
                        {
                            return;
                        }
                        controller.as_mut().reader_changed();
                        if controller.reader.chapter.generation == chapter_generation
                            && controller.reader.translation.requested
                        {
                            let anchor = controller.reader.text.anchor.clone();
                            let offset = controller.reader.text.offset;
                            controller.as_mut().reader_text_position_requested(
                                chapter_generation,
                                anchor,
                                offset,
                            );
                        }
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("text translation"),
                            &QString::from(error.detail()),
                        );
                        let state = &mut controller.as_mut().rust_mut().reader.translation;
                        state.requested = false;
                        state.error = true;
                        state.needs_codex = error.needs_codex();
                        controller.as_mut().reader_changed();
                    }
                }
            });
        });
        self.as_mut().rust_mut().reader.translation.pending.task = Some(task);
        true
    }
}
