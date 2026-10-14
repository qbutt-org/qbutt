use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_codex::{self as codex, CodexError, ModelModality};
use qbutt_content::translation::openrouter::OpenRouterError;
use qbutt_content::translation::text::{
    TextTranslationError, TextTranslationOptions, TextTranslationProvider, translate_chapter,
};
use qbutt_content::{
    Chapter, ChapterRef, LibraryError, LocalBook, TextChapter, TextTranslationService,
    TranslationSelection,
};
use tokio::sync::{Semaphore, watch};

use super::super::library::LibraryWorker;
use super::super::{Pending, ffi};
use super::text::load_local_text_chapter;

#[derive(Default)]
pub(super) struct TextTranslationState {
    pub(super) requested: bool,
    pub(super) busy: bool,
    pub(super) translated: Option<Arc<TextChapter>>,
    pub(super) error: bool,
    pub(super) needs_codex: bool,
    pub(super) pending: Pending,
    cancellation: Option<watch::Sender<bool>>,
    job_chapter: Option<ChapterRef>,
    attempted_next: Option<ChapterRef>,
}

impl TextTranslationState {
    fn cancel_job(&mut self) {
        if let Some(cancel) = self.cancellation.take() {
            let _ = cancel.send(true);
        }
        self.pending.cancel();
        self.job_chapter = None;
        self.busy = false;
    }

    pub(super) fn reset_chapter(&mut self) {
        self.cancel_job();
        self.translated = None;
        self.error = false;
        self.needs_codex = false;
        self.attempted_next = None;
    }

    pub(super) fn select_chapter(&mut self, chapter: &ChapterRef) {
        if self.job_chapter.as_ref() != Some(chapter) {
            self.cancel_job();
        }
        self.translated = None;
        self.error = false;
        self.needs_codex = false;
        self.attempted_next = None;
        self.busy = self.job_chapter.is_some();
    }

    pub(super) fn reset_reader(&mut self) {
        self.reset_chapter();
        self.requested = false;
    }
}

enum TranslationFailure {
    Codex(CodexError),
    OpenRouter(OpenRouterError),
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
            Self::OpenRouter(error) => error.to_string(),
        }
    }
}

async fn translate(
    original: Arc<TextChapter>,
    chapters: Vec<Chapter>,
    catalog: Arc<super::super::catalog::Catalog>,
    local_book: Option<Arc<LocalBook>>,
    load_limit: Arc<Semaphore>,
    library: LibraryWorker,
    language: String,
    cancelled: watch::Receiver<bool>,
) -> Result<Arc<TextChapter>, TranslationFailure> {
    let settings = library
        .text_translation_settings()
        .await
        .map_err(TranslationFailure::Library)?;
    let (provider, lookup, model, effort) =
        if settings.service == TextTranslationService::OpenRouter {
            let model = settings.openrouter.model;
            let effort = settings.openrouter.effort;
            let lookup = library
                .translation_lookup(
                    Arc::clone(&original),
                    language.clone(),
                    "openrouter".into(),
                    model.clone(),
                    effort.clone(),
                )
                .await
                .map_err(TranslationFailure::Library)?;
            if let Some(cached) = lookup.cached {
                return Ok(Arc::new(cached));
            }
            let client = library
                .openrouter_client()
                .await
                .map_err(TranslationFailure::Library)?;
            (
                TextTranslationProvider::OpenRouter(client),
                lookup,
                model,
                effort,
            )
        } else {
            let saved = library
                .translation_selection(language.clone())
                .await
                .map_err(TranslationFailure::Library)?;
            if let Some(selection) = &saved {
                let lookup = library
                    .translation_lookup(
                        Arc::clone(&original),
                        language.clone(),
                        "codex".into(),
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
                ModelModality::Text,
                explicit.map(|selection| selection.model.as_str()),
                explicit.map(|selection| selection.effort.as_str()),
            )
            .await
            .map_err(TranslationFailure::Codex)?;
            let lookup = library
                .translation_lookup(
                    Arc::clone(&original),
                    language.clone(),
                    "codex".into(),
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

            (TextTranslationProvider::Codex, lookup, model, effort)
        };

    let source = move |reference: ChapterRef| {
        let catalog = Arc::clone(&catalog);
        let local_book = local_book.clone();
        let load_limit = Arc::clone(&load_limit);
        async move {
            if let Some(book) = local_book {
                load_local_text_chapter(book, reference, load_limit).await
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
        &provider,
        cancelled.clone(),
    )
    .await
    .map_err(|error| match error {
        TextTranslationError::Codex(error) => TranslationFailure::Codex(error),
        TextTranslationError::OpenRouter(error) => TranslationFailure::OpenRouter(error),
    })?;
    library
        .save_translation(
            lookup.key,
            lookup.context.revision,
            result.chapter.clone(),
            result.additions,
            result.evidence,
            cancelled,
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
            self.reader.manga_translation.error.is_some()
        }
    }

    pub(in crate::controller) fn reader_translation_needs_codex(&self) -> bool {
        if self.reader.is_text {
            self.reader.translation.needs_codex
        } else {
            self.reader
                .manga_translation
                .error
                .as_ref()
                .is_some_and(|error| error.needs_codex())
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
        let original = self.reader.text.original.as_ref().cloned();
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
            state.attempted_next = None;
            if was_translated && let Some(original) = original {
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
        let Some(original) = original else {
            if !self.reader.translation.requested {
                self.as_mut().rust_mut().reader.translation.requested = true;
                self.as_mut().reader_changed();
            }
            return true;
        };
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
            self.as_mut().start_text_translation_ahead();
            return true;
        }
        let reference = original.reference.clone();
        self.start_text_translation(reference, Some(original), false)
    }

    pub(super) fn start_text_translation_ahead(mut self: Pin<&mut Self>) {
        if !self.reader.translation.requested
            || self.reader.translation.translated.is_none()
            || self.reader.shown_position.is_none()
            || self.reader.translation.job_chapter.is_some()
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
        if self.reader.translation.attempted_next.as_ref() == Some(&next) {
            return;
        }
        let original = self.as_mut().rust_mut().reader.text.cache.get(&next);
        // Online originals have their own bounded acquisition path. Wait for
        // that prefetch instead of issuing a duplicate source request.
        if original.is_none() && next.variant.publication.source != qbutt_content::Source::LocalBook
        {
            return;
        }
        self.start_text_translation(next, original, true);
    }

    fn start_text_translation(
        mut self: Pin<&mut Self>,
        reference: ChapterRef,
        original: Option<Arc<TextChapter>>,
        ahead: bool,
    ) -> bool {
        let Some(library) = self.library_worker.as_ref().cloned() else {
            return false;
        };
        let Some(services) = self.services.as_ref() else {
            return false;
        };
        let chapters = self.reader.chapters.all();
        let catalog = Arc::clone(&services.catalog);
        let load_limit = Arc::clone(&services.reader_load);
        let local_book = self
            .local_book
            .as_ref()
            .cloned()
            .filter(|book| book.reference() == reference.variant.publication);
        let language = self.reader_settings.value.translation_language.clone();
        let qt_thread = self.qt_thread();
        let chapter_generation = self.reader.chapter.generation;
        let (cancel, cancelled) = watch::channel(false);
        {
            let state = &mut self.as_mut().rust_mut().reader.translation;
            if ahead {
                state.cancel_job();
                state.attempted_next = Some(reference.clone());
            } else {
                state.reset_chapter();
                state.requested = true;
                state.busy = true;
            }
            state.job_chapter = Some(reference.clone());
            state.cancellation = Some(cancel);
        }
        let generation = self.reader.translation.pending.generation;
        if !ahead {
            self.as_mut().reader_changed();
        }
        if self.reader.translation.pending.generation != generation
            || self.reader.chapter.generation != chapter_generation
        {
            return true;
        }
        let task = self.services.as_ref().unwrap().runtime.spawn(async move {
            let result = async {
                let original = if let Some(original) = original {
                    original
                } else {
                    let book = local_book
                        .as_ref()
                        .cloned()
                        .ok_or(TranslationFailure::Codex(CodexError::Protocol))?;
                    let original =
                        load_local_text_chapter(book, reference.clone(), Arc::clone(&load_limit))
                            .await
                            .map_err(|_| TranslationFailure::Codex(CodexError::Provider))?;
                    Arc::new(original)
                };
                translate(
                    original, chapters, catalog, local_book, load_limit, library, language,
                    cancelled,
                )
                .await
            }
            .await;
            let _ = qt_thread.queue(move |controller| {
                controller.complete_text_translation(reference, generation, result);
            });
        });
        self.as_mut().rust_mut().reader.translation.pending.task = Some(task);
        true
    }

    fn complete_text_translation(
        mut self: Pin<&mut Self>,
        reference: ChapterRef,
        generation: u64,
        result: Result<Arc<TextChapter>, TranslationFailure>,
    ) {
        if self.reader.translation.pending.generation != generation
            || self.reader.translation.job_chapter.as_ref() != Some(&reference)
            || !self.reader.translation.requested
        {
            return;
        }
        {
            let state = &mut self.as_mut().rust_mut().reader.translation;
            state.pending.task = None;
            state.cancellation = None;
            state.job_chapter = None;
        }
        let current = self.reader.reference.as_ref() == Some(&reference.variant)
            && self.reader.chapter_id.to_string() == reference.id;
        if let Err(error) = &result {
            ffi::log_content_error(
                &QString::from("text translation"),
                &QString::from(error.detail()),
            );
        }
        if !current {
            return;
        }
        self.as_mut().rust_mut().reader.translation.busy = false;
        match result {
            Ok(chapter) => {
                self.as_mut().rust_mut().reader.translation.translated = Some(chapter);
                // A promoted read-ahead may finish while its original is still
                // loading. on_original_text_chapter_ready applies it afterwards.
                if self.reader.text.original.is_some() {
                    self.as_mut().set_reader_text_translation(true);
                }
            }
            Err(error) => {
                let state = &mut self.as_mut().rust_mut().reader.translation;
                state.requested = false;
                state.error = true;
                state.needs_codex = error.needs_codex();
                self.as_mut().reader_changed();
            }
        }
    }
}
